//! Phone backup pipeline for Phase 5 (Linux MTP).
//!
//! Orchestrates MTP source discovery, the file-safety engine (`fileops`),
//! conflict handling, cancellation, and progress reporting. All blocking I/O
//! runs inside `tokio::task::spawn_blocking`; the async loop only awaits the
//! file engine and updates job progress.

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use crate::ipc::error::IpcError;
use crate::ipc::models::{BackupConfig, DeviceId};
use crate::services::fileops::{
    copy_verified_with_root, Destination, FileOpError, Source, VerifyMode,
};
use crate::services::jobs::{JobHandle, JobProgress};

const CONSECUTIVE_ERROR_ABORT: usize = 25;
const MAX_FILES_PER_JOB: usize = 100_000;

/// Runs a phone backup using the device id in `config` (resolves the GVFS mount
/// path itself). Prefer `run_backup_with_root` in tests.
pub async fn run_backup(
    config: BackupConfig,
    handle: JobHandle,
    cancel: CancellationToken,
) -> Result<(), IpcError> {
    let device_id = config.device_id.clone();
    let mount_root = tokio::task::spawn_blocking(move || resolve_mount_root(&device_id))
        .await
        .map_err(|e| IpcError::io(&format!("mount resolution task failed: {e}")))??;
    run_backup_with_root(config, mount_root, handle, cancel).await
}

/// Runs the backup pipeline given an explicit device mount root.
///
/// `mount_root` is the read-only GVFS directory that represents the device.
/// The destination-inside-source guard fires before any file operation.
pub async fn run_backup_with_root(
    config: BackupConfig,
    mount_root: PathBuf,
    handle: JobHandle,
    cancel: CancellationToken,
) -> Result<(), IpcError> {
    let (mount_root, destination_root) = {
        let mount = mount_root;
        let dest = config.destination_root.clone();
        tokio::task::spawn_blocking(move || {
            let mount_root = std::fs::canonicalize(&mount).map_err(|e| {
                IpcError::device_disconnected(&format!("device mount unavailable: {e}"))
            })?;
            let destination_root = std::fs::canonicalize(&dest)
                .map_err(|e| IpcError::validation(&format!("destination_root unavailable: {e}")))?;
            if destination_root.starts_with(&mount_root) {
                return Err(IpcError::validation(
                    "destination_root must not be inside the device mount",
                ));
            }
            Ok::<_, IpcError>((mount_root, destination_root))
        })
        .await
        .map_err(|e| IpcError::io(&format!("canonicalize task failed: {e}")))??
    };

    let files: Vec<PathBuf> = {
        let mount = mount_root.clone();
        let rels = config.source_rel_paths.clone();
        tokio::task::spawn_blocking(move || collect_files_for_job(&mount, &rels))
            .await
            .map_err(|e| IpcError::io(&format!("walk task failed: {e}")))??
    };

    let total = files.len() as u64;
    let verify = if config.verify_hash {
        VerifyMode::SizeAndHash
    } else {
        VerifyMode::SizeOnly
    };

    handle.update_progress(JobProgress {
        current: 0,
        total: Some(total),
        message: Some(format!("prepared {total} files")),
    });

    let mut consecutive_errors = 0usize;
    let mut completed = 0u64;

    for source_path in files {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("backup cancelled by user"));
        }

        let rel = match source_path.strip_prefix(&mount_root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => {
                return Err(IpcError::validation("source path outside mount root"));
            }
        };
        let dest_path = destination_root.join(&rel);

        if let Some(parent) = dest_path.parent() {
            let parent = parent.to_path_buf();
            tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(&parent).map_err(|e| map_fs_error(&e))
            })
            .await
            .map_err(|e| IpcError::io(&format!("mkdir task failed: {e}")))??;
        }

        let dest = Destination {
            final_path: dest_path,
            conflict_mode: config.conflict_mode,
            expected_size: None,
            verify,
        };
        let source = Source::File(source_path);
        let approved_root = Some(destination_root.as_path());
        let result = copy_verified_with_root(source, dest, cancel.clone(), approved_root).await;

        match result {
            Ok(_published) => {
                consecutive_errors = 0;
                completed += 1;
            }
            Err(FileOpError::Cancelled) => {
                return Err(IpcError::cancelled("backup cancelled by user"));
            }
            Err(err) => {
                consecutive_errors += 1;
                if !mount_root.exists() {
                    return Err(IpcError::device_disconnected(
                        "device disconnected during backup",
                    ));
                }
                if consecutive_errors >= CONSECUTIVE_ERROR_ABORT {
                    return Err(err.into());
                }
            }
        }

        handle.update_progress(JobProgress {
            current: completed,
            total: Some(total),
            message: None,
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn resolve_mount_root(device_id: &DeviceId) -> Result<PathBuf, IpcError> {
    crate::platform::linux::mtp::mount_root_for(device_id).map_err(map_mtp_error)
}

#[cfg(not(target_os = "linux"))]
fn resolve_mount_root(_device_id: &DeviceId) -> Result<PathBuf, IpcError> {
    Err(IpcError::device_disconnected(
        "MTP backup is not yet supported on this platform",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn map_mtp_error(err: crate::platform::linux::mtp::MtpError) -> IpcError {
    use crate::platform::linux::mtp::MtpError;
    match err {
        MtpError::DeviceNotFound => IpcError::device_disconnected("MTP device not found"),
        MtpError::NoRuntimeDir => IpcError::io("GVFS runtime directory unavailable"),
        MtpError::Io(msg) => IpcError::io(&msg),
        MtpError::InvalidPath => IpcError::validation("Invalid MTP path"),
    }
}

fn collect_files_for_job(
    mount_root: &Path,
    source_rel_paths: &[String],
) -> Result<Vec<PathBuf>, IpcError> {
    let mut files = Vec::new();
    for rel in source_rel_paths {
        let Some(rel_path) = sanitize_rel_path(rel) else {
            return Err(IpcError::validation(&format!(
                "invalid source_rel_path: {rel}"
            )));
        };
        let root = mount_root.join(&rel_path);
        if !root.exists() {
            return Err(IpcError::validation(&format!(
                "source path does not exist: {rel}"
            )));
        }
        collect_files(&root, mount_root, &mut files, MAX_FILES_PER_JOB)?;
    }
    Ok(files)
}

fn sanitize_rel_path(input: &str) -> Option<PathBuf> {
    let p = PathBuf::from(input);
    if p.is_absolute() {
        return None;
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(p)
}

fn collect_files(
    dir: &Path,
    mount_root: &Path,
    out: &mut Vec<PathBuf>,
    max: usize,
) -> Result<(), IpcError> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if out.len() >= max {
            return Err(IpcError::validation("too many files for a single job"));
        }
        let entries = std::fs::read_dir(&current).map_err(|e| map_fs_error(&e))?;
        for entry in entries {
            let entry = entry.map_err(|e| map_fs_error(&e))?;
            let path = entry.path();
            let ft = entry.file_type().map_err(|e| map_fs_error(&e))?;
            if ft.is_symlink() {
                let target = std::fs::canonicalize(&path).map_err(|e| map_fs_error(&e))?;
                if !target.starts_with(mount_root) {
                    continue;
                }
                let meta = std::fs::metadata(&target).map_err(|e| map_fs_error(&e))?;
                if meta.is_dir() {
                    stack.push(target);
                } else if meta.is_file() {
                    out.push(target);
                }
            } else if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn map_fs_error(err: &std::io::Error) -> IpcError {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::PermissionDenied => IpcError::permission_denied("Permission denied"),
        ErrorKind::StorageFull => IpcError::disk_full("Disk full"),
        _ => {
            let message = err.to_string();
            IpcError::io(&message)
        }
    }
}
