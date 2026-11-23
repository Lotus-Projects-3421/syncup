//! Linux MTP device discovery, folder browsing, and mount-root resolution
//! via GVFS/GIO.
//!
//! GVFS mounts MTP devices under `$XDG_RUNTIME_DIR/gvfs/` as entries named
//! `mtp:host=<serial>`. That directory is the primary discovery source. The
//! `gio::VolumeMonitor` enumeration is a supplementary path for environments
//! where the mount directory has not been scanned yet.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ipc::models::{DeviceId, FolderNode, MtpDevice};

#[derive(Debug, thiserror::Error)]
pub enum MtpError {
    #[error("MTP device not found")]
    DeviceNotFound,
    #[error("GVFS runtime directory unavailable")]
    NoRuntimeDir,
    #[error("I/O error: {0}")]
    Io(String),
    #[error("Invalid MTP path")]
    InvalidPath,
}

/// Enumerates MTP devices mounted via GVFS.
pub fn discover_devices() -> Result<Vec<MtpDevice>, MtpError> {
    let mut devices = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Ok(gvfs_root) = gvfs_mount_root() {
        if let Ok(entries) = std::fs::read_dir(&gvfs_root) {
            for entry in entries.flatten() {
                if let Some(device) = device_from_mount_dir(&entry.path()) {
                    if seen.insert(device.id.0.clone()) {
                        devices.push(device);
                    }
                }
            }
        }
    }

    for device in gio_volume_monitor_devices() {
        if seen.insert(device.id.0.clone()) {
            devices.push(device);
        }
    }

    Ok(devices)
}

/// Resolves the local filesystem mount root for a specific device.
pub fn mount_root_for(device_id: &DeviceId) -> Result<PathBuf, MtpError> {
    let gvfs_root = gvfs_mount_root()?;
    let entry_name = format!("mtp:host={}", device_id.0);
    let candidate = gvfs_root.join(&entry_name);
    if candidate.exists() {
        return Ok(candidate);
    }
    for device in gio_volume_monitor_devices() {
        if device.id == *device_id {
            return Ok(PathBuf::from(device.mount_path));
        }
    }
    Err(MtpError::DeviceNotFound)
}

/// Browses the folder tree under the device root, bounded in depth and size.
pub fn browse_folders(device_id: &DeviceId) -> Result<Vec<FolderNode>, MtpError> {
    const MAX_DEPTH: usize = 3;
    const MAX_ENTRIES: usize = 2000;
    let root = mount_root_for(device_id)?;
    let mut budget = MAX_ENTRIES;
    browse_dir(&root, &root, MAX_DEPTH, &mut budget)
}

fn browse_dir(
    root: &Path,
    current: &Path,
    depth_remaining: usize,
    budget: &mut usize,
) -> Result<Vec<FolderNode>, MtpError> {
    if depth_remaining == 0 || *budget == 0 {
        return Ok(Vec::new());
    }
    let mut children = Vec::new();
    let entries = std::fs::read_dir(current).map_err(|e| MtpError::Io(e.to_string()))?;
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        *budget -= 1;
        let name = entry.file_name().to_string_lossy().to_string();
        let rel_path = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name.clone());
        let grandchildren = browse_dir(root, &path, depth_remaining - 1, budget)?;
        children.push(FolderNode {
            name,
            rel_path,
            children: grandchildren,
        });
    }
    Ok(children)
}

fn device_from_mount_dir(path: &Path) -> Option<MtpDevice> {
    let file_name = path.file_name()?.to_str()?;
    let serial = file_name.strip_prefix("mtp:host=")?;
    if serial.is_empty() {
        return None;
    }
    Some(MtpDevice {
        id: DeviceId(serial.to_string()),
        display_name: serial.to_string(),
        mount_path: path.to_string_lossy().to_string(),
    })
}

fn gvfs_mount_root() -> Result<PathBuf, MtpError> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime).join("gvfs"));
    }
    // SAFETY: `getuid` takes no arguments and cannot fail; it returns
    // the real UID of the calling process. No preconditions to uphold.
    let uid = unsafe { libc::getuid() };
    Ok(PathBuf::from(format!("/run/user/{uid}")).join("gvfs"))
}

fn gio_volume_monitor_devices() -> Vec<MtpDevice> {
    use gio::prelude::*;
    let monitor = gio::VolumeMonitor::get();
    let mut out = Vec::new();
    for mount in monitor.mounts() {
        let root = mount.root();
        let Some(path) = root.path() else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(serial) = name.strip_prefix("mtp:host=") else {
            continue;
        };
        if serial.is_empty() {
            continue;
        }
        out.push(MtpDevice {
            id: DeviceId(serial.to_string()),
            display_name: serial.to_string(),
            mount_path: path.to_string_lossy().to_string(),
        });
    }
    out
}
