use std::fs;
use std::path::Path;

use crate::ipc::error::{IpcError, CANCELLED};
use crate::ipc::models::{DedupManifestSummary, DuplicateGroup, OperationStatus, ScanConfig};
use crate::services::dedup::{run_move, run_scan, run_undo, Manifest};
use crate::services::jobs::{JobHandle, JobKind, JobStatus};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn scan_duplicates(
    config: ScanConfig,
    state: State<'_, AppState>,
) -> Result<String, IpcError> {
    let (id, handle, cancel) = state.jobs.register(JobKind::Dedup);
    let store = state.scan_results.clone();
    let id_str = id.to_string();
    let id_for_task = id_str.clone();
    tokio::spawn(async move {
        if let Err(error) = run_scan(config, id_for_task, &handle, &store, cancel).await {
            finish_on_error(&handle, "scan_duplicates", &error);
        }
    });
    Ok(id_str)
}

/// Classify a dedup service error into the terminal job status it must produce.
///
/// Matches the established cancellation sentinel by exact code equality, as
/// `services/fileops.rs` does when mapping `FileOpError::Cancelled`. Message
/// text is never inspected: a validation or I/O failure whose message happens
/// to mention cancellation is still a failure.
fn terminal_status_for_error(error: &IpcError) -> JobStatus {
    if error.code == CANCELLED {
        JobStatus::Cancelled
    } else {
        JobStatus::Failed
    }
}

/// Record a dedup task failure and move its job to a terminal state.
///
/// The services return `Err` on several early-exit paths without calling
/// `JobHandle::finish`, so discarding the result would leave the job `Pending`
/// forever, make cancellation look ineffective, and hide the reason from both
/// the UI and the log. `JobHandle::finish` ignores a second call once the job is
/// terminal, so paths that already finished are unaffected.
fn finish_on_error(handle: &JobHandle, command: &str, error: &IpcError) {
    let status = terminal_status_for_error(error);
    tracing::warn!(
        command = command,
        code = %error.code,
        error = %error.message,
        "dedup task did not complete"
    );
    handle.finish(status, Some(error.message.clone()));
}

#[tauri::command]
#[specta::specta]
pub async fn get_scan_results(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<DuplicateGroup>, IpcError> {
    state
        .scan_results
        .get(&job_id)
        .ok_or_else(|| IpcError::validation("Scan results not found or expired"))
}

#[tauri::command]
#[specta::specta]
pub async fn move_duplicates(
    scan_job_id: String,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<String, IpcError> {
    let (id, handle, cancel) = state.jobs.register(JobKind::Dedup);
    let store = state.scan_results.clone();
    let data_dir = state.data_dir.clone();
    let id_str = id.to_string();
    tokio::spawn(async move {
        if let Err(error) =
            run_move(scan_job_id, confirmed, &handle, &store, &data_dir, cancel).await
        {
            finish_on_error(&handle, "move_duplicates", &error);
        }
    });
    Ok(id_str)
}

#[tauri::command]
#[specta::specta]
pub async fn undo_duplicates(
    manifest_id: String,
    state: State<'_, AppState>,
) -> Result<String, IpcError> {
    let (_id, handle, cancel) = state.jobs.register(JobKind::Dedup);
    let data_dir = state.data_dir.clone();
    let id_str = _id.to_string();
    tokio::spawn(async move {
        if let Err(error) = run_undo(manifest_id, &handle, &data_dir, cancel).await {
            finish_on_error(&handle, "undo_duplicates", &error);
        }
    });
    Ok(id_str)
}

/// Enumerate persisted duplicate-move manifests. Strictly read-only.
///
/// `run_move` writes `data_dir/dedup/<manifest_id>.json` before the first move
/// and rewrites it after every operation, so a manifest can exist with zero
/// completed operations. Listing therefore reports what is recorded, not what
/// ought to be there.
///
/// Error handling:
/// - Missing `dedup` directory: empty list, because nothing has been moved yet.
/// - `dedup` exists but is not a directory, or cannot be read: IpcError.
/// - A non-regular entry, an unreadable file, unparseable JSON, or a manifest
///   whose internal `manifest_id` differs from its filename stem: skipped with
///   a warning. One bad file must never hide every other manifest.
///
/// Ordering is by the recorded RFC3339 `created_at` string descending, with
/// `manifest_id` as a deterministic tiebreaker. Because chrono varies the
/// fractional-second width in RFC3339 output, this is a stable display ordering
/// rather than a guaranteed chronological ordering across all timestamps.
pub fn list_manifest_summaries(data_dir: &Path) -> Result<Vec<DedupManifestSummary>, IpcError> {
    let dedup_dir = data_dir.join("dedup");

    match fs::symlink_metadata(&dedup_dir) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(IpcError::validation(
                    "dedup path exists but is not a directory",
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(IpcError::io(&e.to_string())),
    }

    let entries = fs::read_dir(&dedup_dir).map_err(|e| IpcError::io(&e.to_string()))?;
    let mut summaries: Vec<DedupManifestSummary> = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "cannot read dedup directory entry; skipping");
                continue;
            }
        };
        let path = entry.path();

        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "cannot stat dedup entry; skipping");
                continue;
            }
        };
        // Quarantine folders, .corrupt/ and symlinks are not manifest files.
        if !file_type.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            tracing::warn!(file = %path.display(), "manifest filename is not valid UTF-8; skipping");
            continue;
        };

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "cannot read dedup manifest; skipping");
                continue;
            }
        };
        let manifest: Manifest = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "cannot parse dedup manifest; skipping");
                continue;
            }
        };
        // The filename is the lookup key used by run_undo, so a mismatch means
        // this file cannot be addressed correctly and must not be offered.
        if manifest.manifest_id != stem {
            tracing::warn!(
                file = %path.display(),
                manifest_id = %manifest.manifest_id,
                "manifest id does not match filename stem; skipping"
            );
            continue;
        }

        let mut completed = 0u64;
        let mut undone = 0u64;
        let mut failed = 0u64;
        let mut blocked = 0u64;
        let mut pending = 0u64;
        let mut attention = 0u64;
        for op in &manifest.ops {
            match &op.status {
                OperationStatus::Completed => completed += 1,
                OperationStatus::Undone => undone += 1,
                OperationStatus::Failed(_) => failed += 1,
                OperationStatus::UndoBlocked(_) => blocked += 1,
                OperationStatus::Pending | OperationStatus::UndoPending => pending += 1,
                OperationStatus::Ambiguous | OperationStatus::Lost => attention += 1,
            }
        }

        summaries.push(DedupManifestSummary {
            manifest_id: manifest.manifest_id,
            created_at: manifest.created_at,
            total_ops: manifest.ops.len() as u64,
            completed_ops: completed,
            undone_ops: undone,
            failed_ops: failed,
            blocked_ops: blocked,
            pending_ops: pending,
            attention_ops: attention,
            potentially_undoable: completed > 0,
        });
    }

    summaries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.manifest_id.cmp(&a.manifest_id))
    });
    Ok(summaries)
}

/// List persisted duplicate-move manifests so a caller can obtain the manifest
/// id required by `undo_duplicates`. Additive and read-only: it does not change
/// the `move_duplicates` or `undo_duplicates` contracts.
///
/// Filesystem reads run in `spawn_blocking` so the async runtime is never
/// blocked.
#[tauri::command]
#[specta::specta]
pub async fn list_dedup_manifests(
    state: State<'_, AppState>,
) -> Result<Vec<DedupManifestSummary>, IpcError> {
    let data_dir = state.data_dir.clone();
    tokio::task::spawn_blocking(move || list_manifest_summaries(&data_dir))
        .await
        .map_err(|e| IpcError::io(&format!("manifest listing task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::models::ManifestEntry;

    fn op(status: OperationStatus) -> ManifestEntry {
        ManifestEntry {
            op_id: "op".to_string(),
            source: "/synthetic/source.jpg".to_string(),
            dest: "/synthetic/quarantine/source.jpg".to_string(),
            expected_size: 4,
            expected_hash: "00".to_string(),
            status,
        }
    }

    fn manifest(id: &str, created_at: &str, ops: Vec<ManifestEntry>) -> Manifest {
        Manifest {
            version: 1,
            manifest_id: id.to_string(),
            scan_job_id: "synthetic-scan-job".to_string(),
            created_at: created_at.to_string(),
            ops,
        }
    }

    fn write_manifest(
        data_dir: &Path,
        file_stem: &str,
        value: &Manifest,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = data_dir.join("dedup");
        fs::create_dir_all(&dir)?;
        fs::write(
            dir.join(format!("{file_stem}.json")),
            serde_json::to_vec_pretty(value)?,
        )?;
        Ok(())
    }

    fn only(data_dir: &Path) -> Result<DedupManifestSummary, Box<dyn std::error::Error>> {
        let mut list = list_manifest_summaries(data_dir)?;
        if list.len() != 1 {
            return Err(format!("expected exactly one summary, got {}", list.len()).into());
        }
        Ok(list.remove(0))
    }

    #[test]
    fn cancelled_code_maps_to_cancelled_status() {
        let error = IpcError::cancelled("Scan cancelled");
        assert_eq!(terminal_status_for_error(&error), JobStatus::Cancelled);
    }

    #[test]
    fn non_cancelled_codes_map_to_failed_status() {
        for error in [
            IpcError::validation("No valid scan roots provided"),
            IpcError::validation("Retained file modified since scan"),
            IpcError::io("cannot read manifest"),
            IpcError::verify_failed("hash mismatch"),
            IpcError::unknown(),
        ] {
            assert_eq!(
                terminal_status_for_error(&error),
                JobStatus::Failed,
                "code {} must be a failure",
                error.code
            );
        }
    }

    #[test]
    fn cancellation_message_text_does_not_imply_cancellation() {
        // Exact code equality, never substring matching on the message.
        let error = IpcError::validation("operation was cancelled by the user");
        assert_eq!(terminal_status_for_error(&error), JobStatus::Failed);
    }

    #[test]
    fn cancellation_code_match_is_exact_and_case_sensitive() {
        let error = IpcError::new("cancelled", "lowercase is a different code");
        assert_eq!(terminal_status_for_error(&error), JobStatus::Failed);
        let prefixed = IpcError::new("NOT_CANCELLED", "prefixed is a different code");
        assert_eq!(terminal_status_for_error(&prefixed), JobStatus::Failed);
    }

    #[test]
    fn missing_dedup_directory_returns_empty_list() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        let list = list_manifest_summaries(data.path())?;
        assert!(list.is_empty(), "nothing has been moved yet");
        Ok(())
    }

    #[test]
    fn dedup_path_not_a_directory_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        fs::write(data.path().join("dedup"), b"not a directory")?;
        let result = list_manifest_summaries(data.path());
        assert!(
            result.is_err(),
            "a non-directory dedup path must be reported"
        );
        Ok(())
    }

    #[test]
    fn valid_manifest_reports_correct_operation_counts() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        let value = manifest(
            "m1",
            "2026-01-01T00:00:00+00:00",
            vec![
                op(OperationStatus::Completed),
                op(OperationStatus::Completed),
                op(OperationStatus::Undone),
                op(OperationStatus::Failed("source modified".to_string())),
                op(OperationStatus::UndoBlocked("occupied".to_string())),
                op(OperationStatus::Pending),
                op(OperationStatus::UndoPending),
                op(OperationStatus::Ambiguous),
                op(OperationStatus::Lost),
            ],
        );
        write_manifest(data.path(), "m1", &value)?;

        // Quarantine output and quarantined-corrupt files live alongside the
        // manifests and must never be listed as manifests.
        fs::create_dir_all(data.path().join("dedup").join("m1").join("root-0"))?;
        fs::create_dir_all(data.path().join("dedup").join(".corrupt"))?;
        fs::write(
            data.path().join("dedup").join(".corrupt").join("old.json"),
            b"{}",
        )?;
        fs::write(data.path().join("dedup").join("notes.txt"), b"ignore me")?;

        let summary = only(data.path())?;
        assert_eq!(summary.manifest_id, "m1");
        assert_eq!(summary.created_at, "2026-01-01T00:00:00+00:00");
        assert_eq!(summary.total_ops, 9);
        assert_eq!(summary.completed_ops, 2);
        assert_eq!(summary.undone_ops, 1);
        assert_eq!(summary.failed_ops, 1);
        assert_eq!(summary.blocked_ops, 1);
        assert_eq!(summary.pending_ops, 2);
        assert_eq!(summary.attention_ops, 2);
        assert!(summary.potentially_undoable);
        Ok(())
    }

    #[test]
    fn manifests_sorted_newest_first() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        write_manifest(
            data.path(),
            "older",
            &manifest(
                "older",
                "2026-01-01T00:00:00+00:00",
                vec![op(OperationStatus::Completed)],
            ),
        )?;
        write_manifest(
            data.path(),
            "newest",
            &manifest(
                "newest",
                "2026-01-03T00:00:00+00:00",
                vec![op(OperationStatus::Completed)],
            ),
        )?;
        write_manifest(
            data.path(),
            "middle",
            &manifest(
                "middle",
                "2026-01-02T00:00:00+00:00",
                vec![op(OperationStatus::Completed)],
            ),
        )?;

        let list = list_manifest_summaries(data.path())?;
        let ids: Vec<String> = list.into_iter().map(|s| s.manifest_id).collect();
        assert_eq!(ids, vec!["newest", "middle", "older"]);
        Ok(())
    }

    #[test]
    fn empty_manifest_is_listed_and_not_undoable() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        write_manifest(
            data.path(),
            "empty",
            &manifest("empty", "2026-01-01T00:00:00+00:00", Vec::new()),
        )?;
        let summary = only(data.path())?;
        assert_eq!(summary.total_ops, 0);
        assert_eq!(summary.completed_ops, 0);
        assert!(
            !summary.potentially_undoable,
            "a manifest written before any move has nothing to undo"
        );
        Ok(())
    }

    #[test]
    fn all_failed_manifest_is_listed_and_not_undoable() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        write_manifest(
            data.path(),
            "failed",
            &manifest(
                "failed",
                "2026-01-01T00:00:00+00:00",
                vec![
                    op(OperationStatus::Failed("source missing".to_string())),
                    op(OperationStatus::Failed("source modified".to_string())),
                ],
            ),
        )?;
        let summary = only(data.path())?;
        assert_eq!(summary.total_ops, 2);
        assert_eq!(summary.failed_ops, 2);
        assert_eq!(summary.completed_ops, 0);
        assert!(!summary.potentially_undoable);
        Ok(())
    }

    #[test]
    fn undone_only_manifest_is_not_undoable() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        write_manifest(
            data.path(),
            "undone",
            &manifest(
                "undone",
                "2026-01-01T00:00:00+00:00",
                vec![op(OperationStatus::Undone), op(OperationStatus::Undone)],
            ),
        )?;
        let summary = only(data.path())?;
        assert_eq!(summary.undone_ops, 2);
        assert_eq!(summary.completed_ops, 0);
        assert!(
            !summary.potentially_undoable,
            "undo selects only Completed ops, so a fully undone manifest has none"
        );
        Ok(())
    }

    #[test]
    fn corrupt_manifest_is_skipped_without_hiding_valid_ones(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        write_manifest(
            data.path(),
            "good",
            &manifest(
                "good",
                "2026-01-01T00:00:00+00:00",
                vec![op(OperationStatus::Completed)],
            ),
        )?;
        fs::write(
            data.path().join("dedup").join("broken.json"),
            b"this is not json",
        )?;
        let summary = only(data.path())?;
        assert_eq!(
            summary.manifest_id, "good",
            "one corrupt file must not empty the whole listing"
        );
        Ok(())
    }

    #[test]
    fn filename_internal_id_mismatch_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
        let data = tempfile::tempdir()?;
        write_manifest(
            data.path(),
            "filename-stem",
            &manifest(
                "different-internal-id",
                "2026-01-02T00:00:00+00:00",
                vec![op(OperationStatus::Completed)],
            ),
        )?;
        write_manifest(
            data.path(),
            "matching",
            &manifest(
                "matching",
                "2026-01-01T00:00:00+00:00",
                vec![op(OperationStatus::Completed)],
            ),
        )?;
        let summary = only(data.path())?;
        assert_eq!(
            summary.manifest_id, "matching",
            "a manifest whose id does not match its filename cannot be addressed by undo"
        );
        Ok(())
    }
}
