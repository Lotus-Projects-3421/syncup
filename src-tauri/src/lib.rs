#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod commands;
pub mod domain;
pub mod ipc;
pub mod platform;
pub mod services;
pub mod state;
pub mod storage;

use tauri::Manager;
use tauri_specta::{collect_commands, Builder};

use crate::services::jobs::JobRegistry;
use crate::state::AppState;

/// Single source of truth for the typed IPC surface.
///
/// Both the application and the `specta-gen` binary build their command list from
/// this function so generated bindings can never drift from the registered
/// handlers.
#[must_use]
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::system::app_version,
        commands::system::health_check,
        commands::migration::migrate_legacy_import,
        commands::migration::get_migration_status,
        commands::migration::get_settings,
        commands::migration::update_settings,
        commands::migration::list_profiles,
        commands::migration::save_profile,
        commands::jobs::list_jobs,
        commands::jobs::get_job,
        commands::jobs::cancel_job,
        commands::backup::discover_devices,
        commands::backup::browse_folders,
        commands::backup::start_backup
    ])
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let builder = specta_builder();

    tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            let log_guard = services::logging::init(&log_dir)?;
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "SyncUp starting");

            let data_dir = app.path().app_data_dir()?;
            let db = storage::open_pool(&data_dir)?;
            tracing::info!("database connection pool ready");

            let jobs = JobRegistry::new(Some(app.handle().clone()));

            app.manage(AppState {
                db,
                data_dir,
                jobs,
                _log_guard: log_guard,
            });

            Ok(())
        })
        .run(tauri::generate_context!())?;

    Ok(())
}
