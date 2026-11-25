use crate::ipc::error::{IpcError, CANCELLED};
use crate::services::jobs::{JobKind, JobProgress, JobStatus};
use crate::services::organizer::{run_organizer, OrganizerConfig};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub async fn start_organizer(
    config: OrganizerConfig,
    state: State<'_, AppState>,
) -> Result<String, IpcError> {
    let (id, handle, cancel) = state.jobs.register(JobKind::Organizer);
    let id_str = id.to_string();
    tokio::spawn(async move {
        let result = run_organizer(config, cancel, Some(handle.clone())).await;
        match result {
            Ok(outcome) => {
                let summary = format!(
                    "{} organized, {} skipped, {} errors",
                    outcome.processed, outcome.skipped, outcome.errors
                );
                let done = (outcome.processed + outcome.skipped) as u64;
                handle.update_progress(JobProgress {
                    current: done,
                    total: Some(done),
                    message: Some(summary.clone()),
                });
                if outcome.errors > 0 {
                    handle.finish(JobStatus::Failed, Some(summary));
                } else {
                    handle.finish(JobStatus::Completed, None);
                }
            }
            Err(e) => {
                if e.code == CANCELLED {
                    handle.finish(JobStatus::Cancelled, Some(e.message));
                } else {
                    handle.finish(JobStatus::Failed, Some(e.message));
                }
            }
        }
    });
    Ok(id_str)
}
