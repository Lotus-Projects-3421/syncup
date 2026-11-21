use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::ipc::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HealthStatus {
    pub ok: bool,
    pub app_version: String,
    pub db_user_version: i32,
}

#[tauri::command]
#[specta::specta]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[tauri::command]
#[specta::specta]
pub fn health_check(state: State<'_, AppState>) -> Result<HealthStatus, IpcError> {
    let connection = state.db.get().map_err(|error| {
        tracing::error!(error = %error, "database pool is unavailable");
        IpcError::db("The local database is unavailable.")
    })?;

    let user_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
        .map_err(|error| {
            tracing::error!(error = %error, "failed to read PRAGMA user_version");
            IpcError::db("The local database version could not be read.")
        })?;

    Ok(HealthStatus {
        ok: true,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        db_user_version: user_version,
    })
}
