#![cfg(target_os = "linux")]
//! Headless Phase 5 MTP hardware verification.
//!
//! Requires a real Android phone attached via USB in File Transfer mode and
//! mounted by GVFS. Run manually:
//!
//!   cargo test --manifest-path src-tauri/Cargo.toml --test mtp_hardware -- --ignored --nocapture
//!
//! These tests are #[ignore] so the normal gate never touches real hardware.

use std::path::Path;

use syncup_lib::domain::conflict::ConflictMode;
use syncup_lib::ipc::models::BackupConfig;
use syncup_lib::platform::linux::mtp;
use syncup_lib::services::backup::run_backup;
use syncup_lib::services::jobs::{JobKind, JobRegistry};

#[test]
#[ignore = "requires real Android device via USB File Transfer mode"]
fn discover_devices_hardware() {
    let devices = mtp::discover_devices().expect("discover_devices failed");
    println!("DISCOVERED {} DEVICES", devices.len());
    for d in &devices {
        println!(
            "  id={} name={} mount={}",
            d.id.0, d.display_name, d.mount_path
        );
    }
    assert!(!devices.is_empty(), "no MTP devices discovered");
}

#[test]
#[ignore = "requires real Android device via USB File Transfer mode"]
fn browse_folders_hardware() {
    let devices = mtp::discover_devices().expect("discover_devices failed");
    let device = devices.first().expect("no MTP devices");
    let folders = mtp::browse_folders(&device.id).expect("browse_folders failed");
    println!("FOLDERS: {folders:#?}");
    assert!(
        !folders.is_empty(),
        "no folders discovered under device root"
    );
}

#[tokio::test]
#[ignore = "requires real Android device via USB File Transfer mode"]
async fn full_backup_hardware() {
    let devices = mtp::discover_devices().expect("discover_devices failed");
    let device = devices.first().expect("no MTP devices").clone();
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_root = dest_dir.path().to_path_buf();

    let registry = JobRegistry::new(None);
    let (_job_id, handle, cancel) = registry.register(JobKind::Backup);

    let config = BackupConfig {
        device_id: device.id.clone(),
        source_rel_paths: vec!["DCIM".to_string()],
        destination_root: dest_root.to_string_lossy().to_string(),
        conflict_mode: ConflictMode::Skip,
        verify_hash: true,
    };

    let result = run_backup(config, handle, cancel).await;
    println!("BACKUP RESULT IS OK: {}", result.is_ok());
    result.expect("backup failed");

    let dcim_dest = dest_root.join("DCIM");
    println!("DEST DCIM EXISTS: {}", dcim_dest.exists());
    let _: &Path = dcim_dest.as_path();
}
