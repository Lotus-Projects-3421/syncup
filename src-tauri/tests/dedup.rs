use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use blake2::digest::consts::U16;
use blake2::{Blake2b, Blake2b512, Digest};

use syncup_lib::commands::dedup::list_manifest_summaries;
use syncup_lib::domain::conflict::ConflictMode;
use syncup_lib::ipc::models::{ManifestEntry, OperationStatus, ScanConfig};
use syncup_lib::services::dedup::{
    reconcile_manifests_on_startup, run_move, run_scan, run_undo, Manifest, ScanResultStore,
};
use syncup_lib::services::fileops::{move_verified, Destination, FileOpError, VerifyMode};
use syncup_lib::services::jobs::{JobHandle, JobKind, JobRegistry};

// ---------------- helpers ----------------

fn create_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

fn dummy_handle() -> (JobHandle, Arc<JobRegistry>) {
    let registry = Arc::new(JobRegistry::new(None));
    let (_id, handle, _cancel) = registry.register(JobKind::Dedup);
    (handle, registry)
}

/// Locate the single manifest JSON inside `dedup/`.
///
/// After a successful move, `dedup/` contains BOTH `<manifest_id>.json` (the manifest)
/// and `<manifest_id>/` (the quarantine directory tree). `read_dir` yields entries in
/// arbitrary filesystem order, so taking the first entry is not a valid way to find the
/// manifest: it may return the directory, and reading it fails with EISDIR.
/// Entry errors are surfaced rather than skipped.
fn manifest_json_path(dedup_dir: &Path) -> PathBuf {
    let mut manifests: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(dedup_dir).expect("read dedup directory") {
        let entry = entry.expect("read dedup directory entry");
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
            manifests.push(path);
        }
    }
    assert_eq!(
        manifests.len(),
        1,
        "expected exactly one manifest JSON in dedup/"
    );
    manifests.remove(0)
}

/// Compute the BLAKE2b-512 hash of a file, mirroring the engine's `compute_file_hash`.
fn blake2b_512_of(path: &Path) -> [u8; 64] {
    let mut file = fs::File::open(path).expect("open file for hashing");
    let mut hasher = Blake2b512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).expect("read file for hashing");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finalize().into()
}

/// Compute quick_hash mirroring domain::dedup::quick_hash, so the test can assert
/// that two files genuinely collide at the quick stage before the full hash catches them.
///
/// Windows are: decimal-size prefix, first up to 64 KiB, last 64 KiB if size > 64 KiB.
/// A deterministic collision therefore requires two files of identical size > 128 KiB
/// whose middle gap differs.
fn quick_hash_bytes(path: &Path, size: u64) -> [u8; 16] {
    const QUICK_CHUNK: usize = 64 * 1024;
    let mut hasher = Blake2b::<U16>::new();
    hasher.update(size.to_string().as_bytes());
    let mut file = fs::File::open(path).expect("open file for quick hash");
    let mut buf = vec![0u8; QUICK_CHUNK];
    let n = file.read(&mut buf).expect("read head for quick hash");
    hasher.update(&buf[..n]);
    if size > QUICK_CHUNK as u64 {
        file.seek(SeekFrom::End(-(QUICK_CHUNK as i64)))
            .expect("seek for tail window");
        let n = file.read(&mut buf).expect("read tail for quick hash");
        hasher.update(&buf[..n]);
    }
    hasher.finalize().into()
}

fn count_files_recursive(dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                n += 1;
            } else if path.is_dir() {
                n += count_files_recursive(&path);
            }
        }
    }
    n
}

fn force_mtime(path: &Path, when: std::time::SystemTime) {
    let f = fs::File::open(path).expect("open for set_modified");
    f.set_modified(when).expect("set_modified");
}

// ---------------- tests ----------------

#[tokio::test]
async fn same_size_different_content_not_grouped() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    create_file(root, "a.txt", b"hello world 123");
    create_file(root, "b.txt", b"world hello 321");
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel)
        .await
        .unwrap();
    let groups = store.get(&job_id).unwrap();
    assert!(
        groups.is_empty(),
        "Files with different content should not be grouped even if size matches"
    );
}

#[tokio::test]
async fn exact_dupes_grouped() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let content = b"duplicate content here";
    create_file(root, "a.txt", content);
    create_file(root, "sub/b.txt", content);
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel)
        .await
        .unwrap();
    let groups = store.get(&job_id).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].duplicates.len(), 1);
}

#[tokio::test]
async fn quick_hash_collision_caught_by_full_hash() {
    // Deterministic collision: 192 KiB files with identical head and tail 64 KiB windows,
    // differing in exactly one byte in the middle gap [64 KiB, 128 KiB).
    // Proves (1) the quick hashes collide, (2) the full BLAKE2b-512 hashes differ,
    // and (3) run_scan therefore produces zero groups (the full-hash stage filters
    // the quick-hash collision).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    const SIZE: usize = 192 * 1024;
    let a = vec![0xAAu8; SIZE];
    let mut b = a.clone();
    b[64 * 1024 + 1024] ^= 0xFF;

    let a_path = create_file(root, "a.bin", &a);
    let b_path = create_file(root, "b.bin", &b);

    let qh_a = quick_hash_bytes(&a_path, SIZE as u64);
    let qh_b = quick_hash_bytes(&b_path, SIZE as u64);
    assert_eq!(
        qh_a, qh_b,
        "files must collide at the quick-hash stage (equal size, equal head, equal tail)"
    );

    let fh_a = blake2b_512_of(&a_path);
    let fh_b = blake2b_512_of(&b_path);
    assert_ne!(
        fh_a, fh_b,
        "full BLAKE2b-512 must distinguish files that share a quick hash"
    );

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel)
        .await
        .unwrap();
    let groups = store.get(&job_id).unwrap();
    assert!(
        groups.is_empty(),
        "run_scan must not group files whose full BLAKE2b-512 hashes differ, even if quick_hash collides"
    );
}

#[tokio::test]
async fn preview_no_side_effects() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    create_file(root, "a.txt", b"dup");
    create_file(root, "b.txt", b"dup");
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel)
        .await
        .unwrap();
    assert!(root.join("a.txt").exists());
    assert!(root.join("b.txt").exists());
    let dedup_dir = tmp.path().join("dedup");
    assert!(!dedup_dir.exists());
}

#[tokio::test]
async fn manifest_written_before_first_move() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    create_file(root, "a.txt", b"dup");
    create_file(root, "b.txt", b"dup");
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();
    let (move_handle, _) = dummy_handle();
    run_move(
        job_id.clone(),
        true,
        &move_handle,
        &store,
        &data_dir,
        cancel.clone(),
    )
    .await
    .unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifests: Vec<_> = fs::read_dir(&dedup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(manifests.len(), 1);
    let manifest_path = manifests[0].path();
    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    assert_eq!(manifest.ops.len(), 1);
    assert!(matches!(manifest.ops[0].status, OperationStatus::Completed));
}

#[tokio::test]
async fn manifest_fsync_after_each_move() {
    // Observable-state proxy for per-op durable persistence: after a successful multi-op
    // move, the on-disk manifest reflects every operation's final status and no orphan
    // .manifest-* temporary files remain. This does NOT count fsync syscalls; an induced
    // directory-fsync failure test is deferred until the production PublishPendingDurability
    // recovery path is corrected (see the separate defect report).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    let content = b"same content across three files";
    create_file(root, "a.txt", content);
    create_file(root, "b.txt", content);
    create_file(root, "c.txt", content);
    let fixed =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for name in ["a.txt", "b.txt", "c.txt"] {
        force_mtime(&root.join(name), fixed);
    }

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();
    let (move_handle, _) = dummy_handle();
    run_move(
        job_id.clone(),
        true,
        &move_handle,
        &store,
        &data_dir,
        cancel.clone(),
    )
    .await
    .unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_str).unwrap();

    assert_eq!(
        manifest.ops.len(),
        2,
        "retention keeps one of three identical files"
    );
    assert!(
        manifest
            .ops
            .iter()
            .all(|op| matches!(op.status, OperationStatus::Completed)),
        "every op must be recorded as Completed on disk"
    );
    assert_eq!(manifest.version, 1);
    assert!(!manifest.manifest_id.is_empty());
    assert!(!manifest.created_at.is_empty());

    let orphan_temps: Vec<PathBuf> = fs::read_dir(&dedup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(".manifest-"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        orphan_temps.is_empty(),
        "no orphan .manifest-* temps may remain: {orphan_temps:?}"
    );
}

#[tokio::test]
async fn crash_between_move_and_manifest_reconciled() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    create_file(root, "a.txt", b"dup");
    create_file(root, "b.txt", b"dup");
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();

    let session = store.get_session(&job_id).unwrap();
    let group = &session.groups[0];
    let dup = &group.duplicates[0];

    let manifest_id = Uuid::new_v4().to_string();
    let dest = data_dir
        .join("dedup")
        .join(&manifest_id)
        .join("root-0")
        .join("b.txt");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::rename(root.join("b.txt"), &dest).unwrap();

    let manifest = Manifest {
        version: 1,
        manifest_id: manifest_id.clone(),
        scan_job_id: job_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ops: vec![ManifestEntry {
            op_id: Uuid::new_v4().to_string(),
            source: root.join("b.txt").to_string_lossy().to_string(),
            dest: dest.to_string_lossy().to_string(),
            expected_size: dup.size,
            expected_hash: group.hash.clone(),
            status: OperationStatus::Pending,
        }],
    };
    let manifest_path = data_dir.join("dedup").join(format!("{}.json", manifest_id));
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    reconcile_manifests_on_startup(&data_dir).unwrap();

    let content = fs::read_to_string(&manifest_path).unwrap();
    let reconciled: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(
        reconciled.ops[0].status,
        OperationStatus::Completed
    ));
}

#[tokio::test]
async fn partial_move_failure_recoverable() {
    // Deterministic partial failure: three identical files at the same depth with identical
    // mtime, so retention sorts them alphabetically and retains a.txt. Duplicates are
    // [b.txt, c.txt]. Modifying c.txt after the scan makes its move fail with
    // "Source modified" while b.txt's move completes. Undo then restores b.txt.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = TempDir::new().unwrap();

    let content = b"identical content for partial-failure test";
    create_file(root, "a.txt", content);
    create_file(root, "b.txt", content);
    create_file(root, "c.txt", content);
    let fixed =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for name in ["a.txt", "b.txt", "c.txt"] {
        force_mtime(&root.join(name), fixed);
    }

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();

    // Induce failure on c.txt AFTER scan: source_hash will no longer match expected_hash.
    fs::write(root.join("c.txt"), b"MODIFIED CONTENT HERE").unwrap();

    let (move_handle, _) = dummy_handle();
    run_move(
        job_id.clone(),
        true,
        &move_handle,
        &store,
        data_dir.path(),
        cancel.clone(),
    )
    .await
    .unwrap();

    let manifest_path = manifest_json_path(&data_dir.path().join("dedup"));
    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_str).unwrap();

    let completed: Vec<_> = manifest
        .ops
        .iter()
        .filter(|op| matches!(op.status, OperationStatus::Completed))
        .collect();
    let failed: Vec<_> = manifest
        .ops
        .iter()
        .filter(|op| matches!(op.status, OperationStatus::Failed(_)))
        .collect();
    assert_eq!(completed.len(), 1, "exactly one op must have completed");
    assert_eq!(failed.len(), 1, "exactly one op must have failed");

    let comp = &completed[0];
    assert!(
        !Path::new(&comp.source).exists(),
        "completed op's source must be moved"
    );
    assert!(
        Path::new(&comp.dest).exists(),
        "completed op's quarantine dest must exist"
    );
    assert_eq!(
        fs::read(&comp.dest).unwrap(),
        content,
        "quarantined copy must match original content"
    );

    let fail = &failed[0];
    assert!(
        fail.source.ends_with("c.txt"),
        "the modified file's op must be the one that failed (got {})",
        fail.source
    );
    assert!(
        Path::new(&fail.source).exists(),
        "failed op must not remove its source"
    );
    assert_eq!(
        fs::read(&fail.source).unwrap(),
        b"MODIFIED CONTENT HERE",
        "failed op's source must retain post-scan bytes"
    );

    // Undo restores the completed op byte-identically.
    let manifest_id = manifest.manifest_id.clone();
    let (undo_handle, _) = dummy_handle();
    run_undo(manifest_id, &undo_handle, data_dir.path(), cancel.clone())
        .await
        .unwrap();

    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_str).unwrap();
    let undone: Vec<_> = manifest
        .ops
        .iter()
        .filter(|op| matches!(op.status, OperationStatus::Undone))
        .collect();
    assert_eq!(undone.len(), 1, "the completed op must now be Undone");
    assert!(
        Path::new(&comp.source).exists(),
        "undo must restore the completed op's source"
    );
    assert_eq!(
        fs::read(&comp.source).unwrap(),
        content,
        "restored file must be byte-identical to original"
    );
}

#[tokio::test]
async fn undo_restores_all() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = TempDir::new().unwrap();

    let content = b"identical content for undo-restores-all test";
    create_file(root, "a.txt", content);
    create_file(root, "b.txt", content);
    create_file(root, "c.txt", content);
    let fixed =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for name in ["a.txt", "b.txt", "c.txt"] {
        force_mtime(&root.join(name), fixed);
    }

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();

    let (move_handle, _) = dummy_handle();
    run_move(
        job_id.clone(),
        true,
        &move_handle,
        &store,
        data_dir.path(),
        cancel.clone(),
    )
    .await
    .unwrap();

    let manifest_path = manifest_json_path(&data_dir.path().join("dedup"));
    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_str).unwrap();
    let manifest_id = manifest.manifest_id.clone();

    // Both duplicates must have been moved.
    let completed_before: usize = manifest
        .ops
        .iter()
        .filter(|op| matches!(op.status, OperationStatus::Completed))
        .count();
    assert_eq!(completed_before, 2);
    assert!(!root.join("b.txt").exists());
    assert!(!root.join("c.txt").exists());
    assert!(root.join("a.txt").exists(), "retained file must remain");

    let (undo_handle, _) = dummy_handle();
    run_undo(manifest_id.clone(), &undo_handle, data_dir.path(), cancel.clone())
        .await
        .unwrap();

    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_str).unwrap();
    let undone: usize = manifest
        .ops
        .iter()
        .filter(|op| matches!(op.status, OperationStatus::Undone))
        .count();
    assert_eq!(undone, 2, "every completed op must now be Undone");
    assert!(
        manifest
            .ops
            .iter()
            .all(|op| !matches!(op.status, OperationStatus::UndoPending)),
        "no op may be left in the UndoPending intermediate state"
    );

    // Content byte-identical to the originals.
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), content);
    assert_eq!(fs::read(root.join("c.txt")).unwrap(), content);

    // Quarantine tree for this manifest must be empty.
    let quarantine_root = data_dir.path().join("dedup").join(&manifest_id);
    assert_eq!(
        count_files_recursive(&quarantine_root),
        0,
        "quarantine must be empty after full undo"
    );
}

#[tokio::test]
async fn undo_idempotent() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = TempDir::new().unwrap();

    let content = b"identical content for undo-idempotent test";
    create_file(root, "a.txt", content);
    create_file(root, "b.txt", content);
    create_file(root, "c.txt", content);
    let fixed =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for name in ["a.txt", "b.txt", "c.txt"] {
        force_mtime(&root.join(name), fixed);
    }

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();

    let (move_handle, _) = dummy_handle();
    run_move(
        job_id.clone(),
        true,
        &move_handle,
        &store,
        data_dir.path(),
        cancel.clone(),
    )
    .await
    .unwrap();

    let manifest_path = manifest_json_path(&data_dir.path().join("dedup"));
    let manifest_str = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&manifest_str).unwrap();
    let manifest_id = manifest.manifest_id.clone();

    let (undo1, _) = dummy_handle();
    run_undo(manifest_id.clone(), &undo1, data_dir.path(), cancel.clone())
        .await
        .unwrap();

    // Snapshot state after first undo.
    let manifest_after_first = fs::read_to_string(&manifest_path).unwrap();
    let b_hash_after_first = blake2b_512_of(&root.join("b.txt"));
    let c_hash_after_first = blake2b_512_of(&root.join("c.txt"));

    // Second undo.
    let (undo2, _) = dummy_handle();
    run_undo(manifest_id, &undo2, data_dir.path(), cancel.clone())
        .await
        .unwrap();

    let manifest_after_second = fs::read_to_string(&manifest_path).unwrap();
    assert_eq!(
        manifest_after_first, manifest_after_second,
        "second undo must not modify the manifest (loop body skipped: no Completed ops remain)"
    );
    assert_eq!(
        blake2b_512_of(&root.join("b.txt")),
        b_hash_after_first,
        "restored b.txt must not change on second undo"
    );
    assert_eq!(
        blake2b_512_of(&root.join("c.txt")),
        c_hash_after_first,
        "restored c.txt must not change on second undo"
    );
}

#[tokio::test]
async fn undo_collision_aware_skips_blocked_op() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    create_file(root, "a.txt", b"dup");
    create_file(root, "b.txt", b"dup");
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();

    let (move_handle, _) = dummy_handle();
    run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone())
        .await
        .unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let manifest_id = manifest_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Recreate original file to block undo.
    create_file(root, "b.txt", b"new content blocking undo");

    let (undo_handle, _) = dummy_handle();
    run_undo(manifest_id, &undo_handle, &data_dir, cancel.clone())
        .await
        .unwrap();

    assert_eq!(
        fs::read(root.join("b.txt")).unwrap(),
        b"new content blocking undo"
    );

    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(
        manifest.ops[0].status,
        OperationStatus::UndoBlocked(_)
    ));
}

#[tokio::test]
async fn cross_filesystem_move_uses_copy_verified_and_removes_source() {
    // Source lives on the default temporary filesystem; quarantine lives on /dev/shm
    // (tmpfs on Linux). A distinct st_dev forces move_verified into the EXDEV branch,
    // which copies, verifies, renames, and removes the source. The inode identity test
    // discriminates this path from the same-fs rename path: rename preserves the source
    // inode, copy+delete does not.
    let source_root = TempDir::new().unwrap();
    let source = source_root.path().join("a.bin");
    let payload = b"cross-filesystem verified move content";
    fs::write(&source, payload).unwrap();

    let source_meta = fs::metadata(&source).unwrap();
    let source_ino = source_meta.ino();
    let source_dev = source_meta.dev();

    let shm = Path::new("/dev/shm");
    assert!(
        shm.exists(),
        "/dev/shm is required on this platform to exercise the EXDEV branch of move_verified"
    );
    let data_root =
        tempfile::TempDir::new_in(shm).expect("create quarantine TempDir on /dev/shm");
    let data_dev = fs::metadata(data_root.path()).unwrap().dev();
    assert_ne!(
        source_dev, data_dev,
        "/dev/shm must provide a distinct device from the default temp dir; st_dev({source_dev}) == st_dev({data_dev}) means the EXDEV branch cannot be reached on this system"
    );

    let dest = data_root.path().join("a.bin");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();

    let expected_hash = blake2b_512_of(&source);
    let dest_spec = Destination {
        final_path: dest.clone(),
        conflict_mode: ConflictMode::Skip,
        expected_size: Some(payload.len() as u64),
        verify: VerifyMode::SizeAndHash,
    };

    let cancel = CancellationToken::new();
    let published = move_verified(source.clone(), dest_spec, expected_hash, cancel)
        .await
        .expect("EXDEV move_verified must succeed");

    assert_eq!(published.final_path, dest);
    assert_eq!(published.bytes_written, payload.len() as u64);
    assert_eq!(fs::read(&dest).unwrap(), payload);
    assert!(
        !source.exists(),
        "EXDEV path must remove the source after verified copy"
    );

    let dest_ino = fs::metadata(&dest).unwrap().ino();
    assert_ne!(
        dest_ino, source_ino,
        "EXDEV path must produce a new inode; equal inodes would mean the same-fs rename path ran instead"
    );
}

#[tokio::test]
async fn destination_collision_without_overwrite_same_fs() {
    // Engine-level collision: call move_verified directly with an occupied destination.
    // The occupied-destination guard must return Validation("destination_occupied")
    // without touching source or occupant.
    let root = TempDir::new().unwrap();
    let source = root.path().join("source.txt");
    let source_bytes = b"source content";
    fs::write(&source, source_bytes).unwrap();

    let dest = root.path().join("dest.txt");
    let occupant_bytes = b"occupant content must survive";
    fs::write(&dest, occupant_bytes).unwrap();

    let expected_hash = blake2b_512_of(&source);
    let dest_spec = Destination {
        final_path: dest.clone(),
        conflict_mode: ConflictMode::Skip,
        expected_size: Some(source_bytes.len() as u64),
        verify: VerifyMode::SizeAndHash,
    };

    let cancel = CancellationToken::new();
    let result = move_verified(source.clone(), dest_spec, expected_hash, cancel).await;

    match result {
        Err(FileOpError::Validation { reason }) => {
            assert_eq!(
                reason, "destination_occupied",
                "reason text must be the stable sentinel"
            );
        }
        other => panic!("expected Validation(destination_occupied), got {other:?}"),
    }

    assert!(
        source.exists(),
        "collision guard must leave the source untouched"
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        source_bytes,
        "source bytes must be unchanged"
    );
    assert_eq!(
        fs::read(&dest).unwrap(),
        occupant_bytes,
        "occupant bytes must be unchanged"
    );
}

#[tokio::test]
async fn destination_collision_without_overwrite_cross_fs() {
    // The occupied-destination guard must fire regardless of device layout: it runs before
    // any rename or EXDEV fallback. With source on the default FS and destination on
    // /dev/shm, a real cross-device setup is in place, yet the collision guard still wins.
    let source_root = TempDir::new().unwrap();
    let source = source_root.path().join("source.txt");
    let source_bytes = b"cross-fs collision source";
    fs::write(&source, source_bytes).unwrap();
    let source_dev = fs::metadata(&source).unwrap().dev();

    let shm = Path::new("/dev/shm");
    assert!(
        shm.exists(),
        "/dev/shm required for cross-fs collision test"
    );
    let dest_root = TempDir::new_in(shm).expect("create dest TempDir on /dev/shm");
    let dest = dest_root.path().join("dest.txt");
    let occupant_bytes = b"cross-fs occupant must survive";
    fs::write(&dest, occupant_bytes).unwrap();
    let dest_dev = fs::metadata(&dest).unwrap().dev();
    assert_ne!(
        source_dev, dest_dev,
        "/dev/shm must be on a distinct device from the default temp dir"
    );

    let expected_hash = blake2b_512_of(&source);
    let dest_spec = Destination {
        final_path: dest.clone(),
        conflict_mode: ConflictMode::Skip,
        expected_size: Some(source_bytes.len() as u64),
        verify: VerifyMode::SizeAndHash,
    };

    let cancel = CancellationToken::new();
    let result = move_verified(source.clone(), dest_spec, expected_hash, cancel).await;

    match result {
        Err(FileOpError::Validation { reason }) => {
            assert_eq!(reason, "destination_occupied");
        }
        other => panic!("expected Validation(destination_occupied), got {other:?}"),
    }

    assert!(
        source.exists(),
        "collision guard must leave the source untouched on cross-FS"
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(fs::read(&dest).unwrap(), occupant_bytes);
}

#[tokio::test]
async fn source_modified_after_scan_rejected() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    create_file(root, "a.txt", b"dup");
    create_file(root, "b.txt", b"dup");
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel.clone())
        .await
        .unwrap();
    fs::write(root.join("b.txt"), b"modified").unwrap();
    let (move_handle, _) = dummy_handle();
    run_move(
        job_id.clone(),
        true,
        &move_handle,
        &store,
        &data_dir,
        cancel.clone(),
    )
    .await
    .unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(manifest.ops[0].status, OperationStatus::Failed(_)));
}

#[tokio::test]
async fn symlink_not_followed_during_scan() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    create_file(root, "a.txt", b"dup");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("a.txt"), root.join("link.txt")).unwrap();
    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel)
        .await
        .unwrap();
    let groups = store.get(&job_id).unwrap();
    assert!(groups.is_empty());
}

#[tokio::test]
async fn overlapping_scan_roots_deduplicated() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    create_file(root, "a.txt", b"dup");
    create_file(root, "sub/b.txt", b"dup");
    let config = ScanConfig {
        scan_roots: vec![
            root.to_string_lossy().to_string(),
            root.join("sub").to_string_lossy().to_string(),
        ],
        exclude_patterns: vec![],
    };
    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();
    run_scan(config, job_id.clone(), &handle, &store, cancel)
        .await
        .unwrap();
    let groups = store.get(&job_id).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].duplicates.len(), 1);
}

#[tokio::test]
async fn undo_pending_reconciliation_resumes_safely() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    let manifest_id = Uuid::new_v4().to_string();
    let source = tmp.path().join("source.txt");
    let dest = data_dir
        .join("dedup")
        .join(&manifest_id)
        .join("root-0")
        .join("source.txt");

    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&source, b"content").unwrap();

    let manifest = Manifest {
        version: 1,
        manifest_id: manifest_id.clone(),
        scan_job_id: "dummy".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ops: vec![ManifestEntry {
            op_id: Uuid::new_v4().to_string(),
            source: source.to_string_lossy().to_string(),
            dest: dest.to_string_lossy().to_string(),
            expected_size: 7,
            expected_hash: "dummy_hash".to_string(),
            status: OperationStatus::UndoPending,
        }],
    };

    let manifest_path = data_dir.join("dedup").join(format!("{}.json", manifest_id));
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    reconcile_manifests_on_startup(&data_dir).unwrap();

    let content = fs::read_to_string(&manifest_path).unwrap();
    let reconciled: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(reconciled.ops[0].status, OperationStatus::Undone));
}

#[tokio::test]
async fn undo_ambiguous_reconciliation_surfaced() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    let manifest_id = Uuid::new_v4().to_string();
    let source = tmp.path().join("source.txt");
    let dest = data_dir
        .join("dedup")
        .join(&manifest_id)
        .join("root-0")
        .join("source.txt");

    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&source, b"content").unwrap();
    fs::write(&dest, b"content").unwrap();

    let manifest = Manifest {
        version: 1,
        manifest_id: manifest_id.clone(),
        scan_job_id: "dummy".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ops: vec![ManifestEntry {
            op_id: Uuid::new_v4().to_string(),
            source: source.to_string_lossy().to_string(),
            dest: dest.to_string_lossy().to_string(),
            expected_size: 7,
            expected_hash: "dummy_hash".to_string(),
            status: OperationStatus::UndoPending,
        }],
    };

    let manifest_path = data_dir.join("dedup").join(format!("{}.json", manifest_id));
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    reconcile_manifests_on_startup(&data_dir).unwrap();

    let content = fs::read_to_string(&manifest_path).unwrap();
    let reconciled: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(
        reconciled.ops[0].status,
        OperationStatus::Ambiguous
    ));
}

#[tokio::test]
async fn malformed_manifest_moved_to_corrupt_folder() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let dedup_dir = data_dir.join("dedup");
    fs::create_dir_all(&dedup_dir).unwrap();

    let bad_manifest = dedup_dir.join("bad.json");
    fs::write(&bad_manifest, "{ invalid json").unwrap();

    reconcile_manifests_on_startup(&data_dir).unwrap();

    assert!(!bad_manifest.exists());
    assert!(dedup_dir.join(".corrupt").join("bad.json").exists());
}

#[tokio::test]
async fn startup_reconciliation_resolves_pending_manifest_before_listing() {
    // Simulates the crash-window state: the move happened on disk (source gone, quarantine
    // dest present with the expected hash) but the manifest was never updated past Pending.
    // After reconcile_manifests_on_startup, list_manifest_summaries (the same function
    // served to commands via list_dedup_manifests) must report the reconciled Completed
    // count and no lingering Pending.
    //
    // NOTE: ordering in production is enforced by lib.rs setup() sequencing — reconcile
    // runs synchronously before app.manage/run — not by a runtime readiness gate. No
    // independent runtime gate exists in v2.0. If a concurrent command could ever race
    // reconciliation, this test would not detect it.
    let data_dir = TempDir::new().unwrap();
    let dedup_dir = data_dir.path().join("dedup");
    fs::create_dir_all(&dedup_dir).unwrap();

    let manifest_id = Uuid::new_v4().to_string();
    let source = data_dir.path().join("source.txt");
    let dest = dedup_dir
        .join(&manifest_id)
        .join("root-0")
        .join("source.txt");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();

    // Simulate the completed move: dest exists with known content, source is gone.
    let payload = b"reconciled";
    fs::write(&dest, payload).unwrap();
    let expected_hash_hex = hex::encode(blake2b_512_of(&dest));

    // Manifest still says Pending (as if the process crashed after rename but before update).
    let manifest = Manifest {
        version: 1,
        manifest_id: manifest_id.clone(),
        scan_job_id: "synthetic-scan".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ops: vec![ManifestEntry {
            op_id: Uuid::new_v4().to_string(),
            source: source.to_string_lossy().to_string(),
            dest: dest.to_string_lossy().to_string(),
            expected_size: payload.len() as u64,
            expected_hash: expected_hash_hex,
            status: OperationStatus::Pending,
        }],
    };

    let manifest_path = dedup_dir.join(format!("{}.json", manifest_id));
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    reconcile_manifests_on_startup(data_dir.path()).unwrap();

    let listing = list_manifest_summaries(data_dir.path()).expect("listing must succeed");
    assert_eq!(listing.len(), 1, "exactly one manifest must be listed");
    let summary = &listing[0];
    assert_eq!(summary.manifest_id, manifest_id);
    assert_eq!(
        summary.completed_ops, 1,
        "reconciliation must promote the Pending op to Completed"
    );
    assert_eq!(
        summary.pending_ops, 0,
        "no op may remain Pending after reconciliation"
    );
    assert!(
        summary.potentially_undoable,
        "a manifest with one Completed op is undoable"
    );
}
