//! Synthetic-fixture tests for the Phase 5 backup pipeline.
//!
//! No real device; no real user data. `run_backup_with_root` is driven with a
//! tempdir as the device mount root so the whole pipeline is exercised locally.

use std::fs;
use std::path::{Path, PathBuf};

use syncup_lib::domain::conflict::ConflictMode;
use syncup_lib::ipc::models::{BackupConfig, DeviceId};
use syncup_lib::services::backup::run_backup_with_root;
use syncup_lib::services::jobs::{JobKind, JobRegistry};

fn setup_fixture() -> (tempfile::TempDir, tempfile::TempDir, PathBuf, PathBuf) {
    let source_tmp = tempfile::tempdir().unwrap();
    let dest_tmp = tempfile::tempdir().unwrap();
    let source_root = source_tmp.path().to_path_buf();
    let dest_root = dest_tmp.path().to_path_buf();
    fs::create_dir_all(source_root.join("DCIM")).unwrap();
    fs::write(source_root.join("DCIM/photo1.jpg"), b"content-a").unwrap();
    fs::write(source_root.join("DCIM/photo2.jpg"), b"content-b").unwrap();
    (source_tmp, dest_tmp, source_root, dest_root)
}

fn config_for(dest_root: &Path, mode: ConflictMode) -> BackupConfig {
    BackupConfig {
        device_id: DeviceId("test-device".to_string()),
        source_rel_paths: vec!["DCIM".to_string()],
        destination_root: dest_root.to_string_lossy().to_string(),
        conflict_mode: mode,
        verify_hash: true,
    }
}

#[tokio::test]
async fn copies_all_files_under_source_dir() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);

    let config = config_for(&dest_root, ConflictMode::Skip);
    run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap();

    let a = fs::read(dest_root.join("DCIM/photo1.jpg")).unwrap();
    let b = fs::read(dest_root.join("DCIM/photo2.jpg")).unwrap();
    assert_eq!(a, b"content-a");
    assert_eq!(b, b"content-b");
}

#[tokio::test]
async fn destination_inside_source_rejected() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);

    let inside = source_root.join("nested-dest");
    fs::create_dir_all(&inside).unwrap();
    let mut config = config_for(&dest_root, ConflictMode::Skip);
    config.destination_root = inside.to_string_lossy().to_string();

    let err = run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap_err();
    assert_eq!(err.code, "VALIDATION_ERROR");
}

#[tokio::test]
async fn skip_mode_preserves_existing_destination() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    fs::create_dir_all(dest_root.join("DCIM")).unwrap();
    fs::write(dest_root.join("DCIM/photo1.jpg"), b"existing-1").unwrap();

    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);
    let config = config_for(&dest_root, ConflictMode::Skip);
    run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap();

    let preserved = fs::read(dest_root.join("DCIM/photo1.jpg")).unwrap();
    assert_eq!(preserved, b"existing-1");
    let copied = fs::read(dest_root.join("DCIM/photo2.jpg")).unwrap();
    assert_eq!(copied, b"content-b");
}

#[tokio::test]
async fn overwrite_mode_replaces_existing_destination() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    fs::create_dir_all(dest_root.join("DCIM")).unwrap();
    fs::write(dest_root.join("DCIM/photo1.jpg"), b"existing-1").unwrap();

    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);
    let config = config_for(&dest_root, ConflictMode::Overwrite);
    run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap();

    let replaced = fs::read(dest_root.join("DCIM/photo1.jpg")).unwrap();
    assert_eq!(replaced, b"content-a");
}

#[tokio::test]
async fn keep_both_mode_adds_suffix() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    fs::create_dir_all(dest_root.join("DCIM")).unwrap();
    fs::write(dest_root.join("DCIM/photo1.jpg"), b"existing-1").unwrap();

    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);
    let config = config_for(&dest_root, ConflictMode::KeepBoth);
    run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap();

    let original = fs::read(dest_root.join("DCIM/photo1.jpg")).unwrap();
    assert_eq!(original, b"existing-1");
    let suffix_candidate = dest_root.join("DCIM/photo1 (1).jpg");
    assert!(
        suffix_candidate.exists(),
        "expected keep-both suffix at {suffix_candidate:?}"
    );
    let suffixed = fs::read(&suffix_candidate).unwrap();
    assert_eq!(suffixed, b"content-a");
}

#[tokio::test]
async fn cancellation_before_first_file_returns_cancelled() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);
    cancel.cancel();

    let config = config_for(&dest_root, ConflictMode::Skip);
    let err = run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap_err();
    assert_eq!(err.code, "CANCELLED");
}

#[tokio::test]
async fn size_only_verify_path_copies_files() {
    let (_src, _dst, source_root, dest_root) = setup_fixture();
    let registry = JobRegistry::new(None);
    let (_job, handle, cancel) = registry.register(JobKind::Backup);
    let mut config = config_for(&dest_root, ConflictMode::Skip);
    config.verify_hash = false;
    run_backup_with_root(config, source_root, handle, cancel)
        .await
        .unwrap();
    assert!(dest_root.join("DCIM/photo1.jpg").exists());
    assert!(dest_root.join("DCIM/photo2.jpg").exists());
}
