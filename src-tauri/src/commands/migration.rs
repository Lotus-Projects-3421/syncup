use std::path::Path;

use tauri::State;

use crate::ipc::error::IpcError;
use crate::state::AppState;
use crate::storage::migration::{MigrationReport, MigrationStatus};
use crate::storage::profiles::DeviceProfile;
use crate::storage::settings::AppSettings;

#[tauri::command]
#[specta::specta]
pub fn migrate_legacy_import(
    state: State<'_, AppState>,
    source_db_path: Option<String>,
    source_json_dir: Option<String>,
) -> Result<MigrationReport, IpcError> {
    let source_db = source_db_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new);

    let source_json = source_json_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new);

    crate::storage::migration::migrate_legacy_import(
        &state.db,
        &state.data_dir,
        source_db,
        source_json,
    )
    .map_err(|error| {
        tracing::error!(error = %error, "[migration] import operation failed");
        IpcError::migration(&error.to_string())
    })
}

#[tauri::command]
#[specta::specta]
pub fn get_migration_status(state: State<'_, AppState>) -> Result<MigrationStatus, IpcError> {
    crate::storage::migration::get_migration_status(&state.db, &state.data_dir)
        .map_err(|error| {
            tracing::error!(error = %error, "failed to get migration status");
            IpcError::db(&error.to_string())
        })
}

#[tauri::command]
#[specta::specta]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, IpcError> {
    crate::storage::settings::load_settings(&state.data_dir).map_err(|error| {
        tracing::error!(error = %error, "failed to load settings");
        IpcError::db(&error.to_string())
    })
}

#[tauri::command]
#[specta::specta]
pub fn update_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<(), IpcError> {
    crate::storage::settings::save_settings(&state.data_dir, &settings).map_err(|error| {
        tracing::error!(error = %error, "failed to save settings");
        IpcError::db(&error.to_string())
    })
}

#[tauri::command]
#[specta::specta]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<DeviceProfile>, IpcError> {
    crate::storage::profiles::list_profiles(&state.data_dir).map_err(|error| {
        tracing::error!(error = %error, "failed to list profiles");
        IpcError::db(&error.to_string())
    })
}

#[tauri::command]
#[specta::specta]
pub fn save_profile(state: State<'_, AppState>, profile: DeviceProfile) -> Result<(), IpcError> {
    crate::storage::profiles::save_profile(&state.data_dir, &profile).map_err(|error| {
        tracing::error!(error = %error, "failed to save profile");
        IpcError::db(&error.to_string())
    })
}
