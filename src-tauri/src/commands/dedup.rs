use crate::ipc::error::IpcError;
use crate::ipc::models::{DuplicateGroup, ScanConfig};
use crate::services::dedup::{run_move, run_scan, run_undo};
use crate::services::jobs::JobKind;
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
        let _ = run_scan(config, id_for_task, &handle, &store, cancel).await;
    });
    Ok(id_str)
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
        let _ = run_move(scan_job_id, confirmed, &handle, &store, &data_dir, cancel).await;
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
        let _ = run_undo(manifest_id, &handle, &data_dir, cancel).await;
    });
    Ok(id_str)
}
