use crate::ipc::error::IpcError;
use crate::services::organizer::{run_organizer, OrganizerConfig};
use tokio_util::sync::CancellationToken;

#[tauri::command]
#[specta::specta]
pub async fn start_organizer(config: OrganizerConfig) -> Result<String, IpcError> {
    let job_id = "organizer-job".to_string();
    let cancel = CancellationToken::new();
    tokio::spawn(async move {
        let _ = run_organizer(config, cancel).await;
    });
    Ok(job_id)
}
