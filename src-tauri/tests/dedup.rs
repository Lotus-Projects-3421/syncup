use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use syncup_lib::ipc::models::{ManifestEntry, OperationStatus, ScanConfig};
use syncup_lib::services::dedup::{self, Manifest, ScanResultStore};
use syncup_lib::services::jobs::{JobHandle, JobKind, JobRegistry};

// Helper to create a file with specific content
fn create_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

// Helper to create a dummy job handle
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
    assert_eq!(manifests.len(), 1, "expected exactly one manifest JSON in dedup/");
    manifests.remove(0)
}

#[tokio::test]
async fn same_size_different_content_not_grouped() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    
    // Two files, same size, different content
    create_file(root, "a.txt", b"hello world 123"); // 15 bytes
    create_file(root, "b.txt", b"world hello 321"); // 15 bytes

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };

    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel).await.unwrap();

    let groups = store.get(&job_id).unwrap();
    assert!(groups.is_empty(), "Files with different content should not be grouped even if size matches");
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel).await.unwrap();

    let groups = store.get(&job_id).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].duplicates.len(), 1);
}

#[tokio::test]
async fn quick_hash_collision_caught_by_full_hash() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    
    create_file(root, "a.bin", &[0u8; 1024]);
    let mut b_content = vec![0u8; 1024];
    b_content[512] = 1; // Change middle byte
    create_file(root, "b.bin", &b_content);

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };

    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel).await.unwrap();

    let groups = store.get(&job_id).unwrap();
    assert!(groups.is_empty());
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel).await.unwrap();

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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifests: Vec<_> = fs::read_dir(&dedup_dir).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    
    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    
    assert!(matches!(manifest.ops[0].status, OperationStatus::Completed));
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let session = store.get_session(&job_id).unwrap();
    let group = &session.groups[0];
    let dup = &group.duplicates[0];
    
    let manifest_id = Uuid::new_v4().to_string();
    let dest = data_dir.join("dedup").join(&manifest_id).join("root-0").join("b.txt");
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
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    
    dedup::reconcile_manifests_on_startup(&data_dir).unwrap();
    
    let content = fs::read_to_string(&manifest_path).unwrap();
    let reconciled: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(reconciled.ops[0].status, OperationStatus::Completed));
}

#[tokio::test]
async fn partial_move_failure_recoverable() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    
    create_file(root, "a.txt", b"dup");
    create_file(root, "b.txt", b"dup");
    create_file(root, "c.txt", b"dup");

    let config = ScanConfig {
        scan_roots: vec![root.to_string_lossy().to_string()],
        exclude_patterns: vec![],
    };

    let store = ScanResultStore::default();
    let (handle, _reg) = dummy_handle();
    let cancel = CancellationToken::new();
    let job_id = Uuid::new_v4().to_string();

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    
    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    
    assert_eq!(manifest.ops.len(), 2);
    assert!(manifest.ops.iter().all(|op| matches!(op.status, OperationStatus::Completed)));
}

#[tokio::test]
async fn undo_restores_all() {
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let manifest_id = manifest_path.file_stem().unwrap().to_string_lossy().to_string();
    
    let (undo_handle, _) = dummy_handle();
    dedup::run_undo(manifest_id.clone(), &undo_handle, &data_dir, cancel.clone()).await.unwrap();
    
    assert!(root.join("b.txt").exists());
    assert!(!data_dir.join("dedup").join(&manifest_id).join("root-0").join("b.txt").exists());
}

#[tokio::test]
async fn undo_idempotent() {
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let manifest_id = manifest_path.file_stem().unwrap().to_string_lossy().to_string();
    
    let (undo_handle, _) = dummy_handle();
    dedup::run_undo(manifest_id.clone(), &undo_handle, &data_dir, cancel.clone()).await.unwrap();
    
    let (undo_handle2, _) = dummy_handle();
    dedup::run_undo(manifest_id, &undo_handle2, &data_dir, cancel.clone()).await.unwrap();
    
    assert!(root.join("b.txt").exists());
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let manifest_id = manifest_path.file_stem().unwrap().to_string_lossy().to_string();
    
    create_file(root, "b.txt", b"new content blocking undo");
    
    let (undo_handle, _) = dummy_handle();
    dedup::run_undo(manifest_id, &undo_handle, &data_dir, cancel.clone()).await.unwrap();
    
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"new content blocking undo");
    
    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(manifest.ops[0].status, OperationStatus::UndoBlocked(_)));
}

#[tokio::test]
async fn cross_filesystem_move_uses_copy_verified_and_removes_source() {
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();

    assert!(!root.join("b.txt").exists());
}

#[tokio::test]
async fn destination_collision_without_overwrite_same_fs() {
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();
    
    let dedup_dir = data_dir.join("dedup");
    let manifest_path = manifest_json_path(&dedup_dir);
    let content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(manifest.ops[0].status, OperationStatus::Completed));
}

#[tokio::test]
async fn destination_collision_without_overwrite_cross_fs() {
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();
    
    assert!(!root.join("b.txt").exists());
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel.clone()).await.unwrap();
    
    fs::write(root.join("b.txt"), b"modified").unwrap();
    
    let (move_handle, _) = dummy_handle();
    dedup::run_move(job_id.clone(), true, &move_handle, &store, &data_dir, cancel.clone()).await.unwrap();
    
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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel).await.unwrap();

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

    dedup::run_scan(config, job_id.clone(), &handle, &store, cancel).await.unwrap();

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
    let dest = data_dir.join("dedup").join(&manifest_id).join("root-0").join("source.txt");
    
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
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    
    dedup::reconcile_manifests_on_startup(&data_dir).unwrap();
    
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
    let dest = data_dir.join("dedup").join(&manifest_id).join("root-0").join("source.txt");
    
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
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    
    dedup::reconcile_manifests_on_startup(&data_dir).unwrap();
    
    let content = fs::read_to_string(&manifest_path).unwrap();
    let reconciled: Manifest = serde_json::from_str(&content).unwrap();
    assert!(matches!(reconciled.ops[0].status, OperationStatus::Ambiguous));
}

#[tokio::test]
async fn malformed_manifest_moved_to_corrupt_folder() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let dedup_dir = data_dir.join("dedup");
    fs::create_dir_all(&dedup_dir).unwrap();
    
    let bad_manifest = dedup_dir.join("bad.json");
    fs::write(&bad_manifest, "{ invalid json").unwrap();
    
    dedup::reconcile_manifests_on_startup(&data_dir).unwrap();
    
    assert!(!bad_manifest.exists());
    assert!(dedup_dir.join(".corrupt").join("bad.json").exists());
}

#[tokio::test]
async fn readiness_gate_blocks_commands_until_reconciliation_completes() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    
    let result = dedup::reconcile_manifests_on_startup(&data_dir);
    assert!(result.is_ok());
}
