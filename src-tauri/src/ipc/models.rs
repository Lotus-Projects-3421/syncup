//! Phase 5 IPC models for MTP device discovery, folder browsing, and backup.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::domain::conflict::ConflictMode;

/// Opaque identifier for a discovered MTP device (the GVFS serial portion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DeviceId(pub String);

/// MTP device as discovered via GVFS/GIO.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MtpDevice {
    pub id: DeviceId,
    pub display_name: String,
    /// GVFS mount path on the local filesystem (read-only).
    pub mount_path: String,
}

/// A folder node in the device tree.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub name: String,
    /// Path relative to the device mount root.
    pub rel_path: String,
    pub children: Vec<FolderNode>,
}

/// Configuration for a phone backup job.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackupConfig {
    pub device_id: DeviceId,
    /// Paths relative to the device mount root to back up.
    pub source_rel_paths: Vec<String>,
    /// Absolute destination directory on the local machine.
    pub destination_root: String,
    pub conflict_mode: ConflictMode,
    /// Verify content via BLAKE2b hash (true) or size only (false).
    pub verify_hash: bool,
}
