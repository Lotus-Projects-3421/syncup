//! Phase 6 IPC commands for Wi-Fi backup sessions (thin delegation).
//!
//! `start_wifi_session` registers one `JobKind::Wifi` job per session and returns
//! the URL + token once over local IPC. `stop_wifi_session` cancels in-flight
//! uploads, shuts the server down, wipes staging, and finishes the job exactly
//! once. Session state lives in the managed [`WifiSessionManager`], not AppState.

use tauri::State;

use crate::ipc::error::IpcError;
use crate::ipc::models::{WifiConfig, WifiSessionInfo};
use crate::services::wifi::WifiSessionManager;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn start_wifi_session(
    config: WifiConfig,
    state: State<'_, AppState>,
    wifi: State<'_, WifiSessionManager>,
) -> Result<WifiSessionInfo, IpcError> {
    wifi.start(config, &state.data_dir, &state.jobs).await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_wifi_session(wifi: State<'_, WifiSessionManager>) -> Result<(), IpcError> {
    wifi.stop().await
}
