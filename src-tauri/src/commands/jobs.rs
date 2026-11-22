use tauri::State;

use crate::ipc::error::IpcError;
use crate::services::jobs::{job_not_found, Job, JobId};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_jobs(state: State<'_, AppState>) -> Result<Vec<Job>, IpcError> {
    Ok(state.jobs.list())
}

#[tauri::command]
#[specta::specta]
pub async fn get_job(job_id: JobId, state: State<'_, AppState>) -> Result<Job, IpcError> {
    state
        .jobs
        .get(&job_id)
        .ok_or_else(|| job_not_found(&job_id))
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_job(job_id: JobId, state: State<'_, AppState>) -> Result<(), IpcError> {
    state.jobs.cancel(&job_id)
}
