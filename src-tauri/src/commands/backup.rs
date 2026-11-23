//! Phase 5 IPC commands for Linux MTP phone backup.
//!
//! Thin delegation: discovery and browse run inside `spawn_blocking`; `start_backup`
//! registers a `JobKind::Backup` job and spawns the pipeline, then finishes the
//! job with the appropriate terminal status.

use tauri::State;

use crate::ipc::error::{IpcError, CANCELLED};
use crate::ipc::models::{BackupConfig, DeviceId, FolderNode, MtpDevice};
use crate::services::jobs::{JobId, JobKind, JobStatus};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn discover_devices() -> Result<Vec<MtpDevice>, IpcError> {
    tokio::task::spawn_blocking(discover_devices_sync)
        .await
        .map_err(|e| IpcError::io(&format!("discovery task failed: {e}")))?
}

fn discover_devices_sync() -> Result<Vec<MtpDevice>, IpcError> {
    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::mtp::discover_devices()
            .map_err(crate::services::backup::map_mtp_error)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(IpcError::device_disconnected(
            "MTP discovery is not yet supported on this platform",
        ))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn browse_folders(device_id: DeviceId) -> Result<Vec<FolderNode>, IpcError> {
    tokio::task::spawn_blocking(move || browse_folders_sync(device_id))
        .await
        .map_err(|e| IpcError::io(&format!("browse task failed: {e}")))?
}

fn browse_folders_sync(device_id: DeviceId) -> Result<Vec<FolderNode>, IpcError> {
    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::mtp::browse_folders(&device_id)
            .map_err(crate::services::backup::map_mtp_error)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device_id;
        Err(IpcError::device_disconnected(
            "MTP browsing is not yet supported on this platform",
        ))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn start_backup(
    config: BackupConfig,
    state: State<'_, AppState>,
) -> Result<JobId, IpcError> {
    let dest = std::path::PathBuf::from(&config.destination_root);
    if !dest.is_absolute() {
        return Err(IpcError::validation(
            "destination_root must be an absolute path",
        ));
    }
    if !dest.is_dir() {
        return Err(IpcError::validation(
            "destination_root must be an existing directory",
        ));
    }
    if config.source_rel_paths.is_empty() {
        return Err(IpcError::validation("source_rel_paths must not be empty"));
    }

    let (job_id, handle, cancel) = state.jobs.register(JobKind::Backup);
    tauri::async_runtime::spawn(async move {
        let result = crate::services::backup::run_backup(config, handle.clone(), cancel).await;
        match result {
            Ok(()) => handle.finish(JobStatus::Completed, None),
            Err(err) if err.code.as_str() == CANCELLED => {
                handle.finish(JobStatus::Cancelled, None);
            }
            Err(err) => handle.finish(JobStatus::Failed, Some(err.message)),
        }
    });
    Ok(job_id)
}
