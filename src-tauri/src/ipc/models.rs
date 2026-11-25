//! IPC models for SyncUp v2.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::domain::conflict::ConflictMode;
use crate::services::jobs::JobId;

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

// ---- Phase 6: Wi-Fi backup ----

/// Configuration for a Wi-Fi upload session.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WifiConfig {
    /// Absolute local directory used READ-ONLY for manifest diff comparison.
    pub destination_root: String,
    /// Explicit opt-in to bind 0.0.0.0 (all interfaces). Defaults to false.
    #[serde(default)]
    pub bind_all: bool,
    /// Session lifetime in seconds; defaults to 1800 when absent.
    #[serde(default)]
    #[specta(type = Option<specta_typescript::Number>)]
    pub ttl_secs: Option<u64>,
}

/// Returned once by `start_wifi_session`. The token travels over local IPC
/// only; it must never be placed in a URL, query parameter, or log. Debug is
/// manually implemented to redact the token.
#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WifiSessionInfo {
    pub url: String,
    pub session_id: String,
    pub ip: String,
    pub port: u16,
    pub token: String,
    pub job_id: JobId,
}

impl std::fmt::Debug for WifiSessionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiSessionInfo")
            .field("url", &self.url)
            .field("session_id", &self.session_id)
            .field("ip", &self.ip)
            .field("port", &self.port)
            .field("token", &"[REDACTED]")
            .field("job_id", &self.job_id)
            .finish()
    }
}

/// One client-declared file in a Wi-Fi manifest. Untrusted: re-validated in
/// Rust; size is enforced against actual streamed bytes by the file engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiManifestEntry {
    pub rel_path: String,
    pub size: u64,
    pub mtime_ns: i64,
}

/// POST /manifest request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiManifestRequest {
    pub files: Vec<WifiManifestEntry>,
}

/// POST /manifest response: which files need uploading. Phase 6 diff is
/// existence + size against the destination root; mtime comparison arrives
/// with the processed-state DB integration in Phase 9.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiManifestResponse {
    pub missing: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: Vec<String>,
}

/// POST /upload success response. `hash` is the BLAKE2b-512 hex of the staged
/// content as verified by the file engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiUploadResult {
    pub rel_path: String,
    pub bytes_received: u64,
    pub hash: Option<String>,
}

// ---- Phase 8: Duplicate Finder ----

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanConfig {
    pub scan_roots: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateFile {
    pub path: String,
    #[specta(type = specta_typescript::Number)]
    pub size: u64,
    #[specta(type = specta_typescript::Number)]
    pub mtime_ns: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    #[specta(type = specta_typescript::Number)]
    pub size: u64,
    pub hash: String,
    pub keep: DuplicateFile,
    pub duplicates: Vec<DuplicateFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OperationStatus {
    Pending,
    Completed,
    Failed(String),
    UndoPending,
    Undone,
    UndoBlocked(String),
    Ambiguous,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub op_id: String,
    pub source: String,
    pub dest: String,
    #[specta(type = specta_typescript::Number)]
    pub expected_size: u64,
    pub expected_hash: String,
    pub status: OperationStatus,
}

/// Read-only summary of one persisted duplicate-move manifest.
///
/// `move_duplicates` returns a job id, while `undo_duplicates` requires the
/// manifest id that `run_move` generates internally. This summary is how a
/// caller discovers that id without guessing it.
///
/// Counts partition every `OperationStatus` variant exactly once:
/// `completed_ops` = Completed, `undone_ops` = Undone, `failed_ops` = Failed,
/// `blocked_ops` = UndoBlocked, `pending_ops` = Pending + UndoPending (not yet
/// resolved), `attention_ops` = Ambiguous + Lost (need user review).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DedupManifestSummary {
    pub manifest_id: String,
    /// RFC3339 timestamp recorded when the manifest was created.
    pub created_at: String,
    #[specta(type = specta_typescript::Number)]
    pub total_ops: u64,
    #[specta(type = specta_typescript::Number)]
    pub completed_ops: u64,
    #[specta(type = specta_typescript::Number)]
    pub undone_ops: u64,
    #[specta(type = specta_typescript::Number)]
    pub failed_ops: u64,
    #[specta(type = specta_typescript::Number)]
    pub blocked_ops: u64,
    #[specta(type = specta_typescript::Number)]
    pub pending_ops: u64,
    #[specta(type = specta_typescript::Number)]
    pub attention_ops: u64,
    /// ADVISORY ONLY: true when at least one operation is still in the
    /// Completed state, which is exactly the set `run_undo` selects. It does
    /// NOT guarantee that undo will succeed. Undo re-validates every path,
    /// refuses occupied originals and modified quarantine files, and may mark
    /// operations UndoBlocked or Failed. Never present this as a promise.
    pub potentially_undoable: bool,
}
