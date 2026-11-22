# Phase 5 lean context (code-only; authority docs already in your history)

## git state
dc3af22 fix: run gate cargo commands against src-tauri manifest
afe71ee chore: add verification gate script
ff6e07b chore: add local AI harness tools
?? context-lean.md

## src tree map
src-tauri/src/bin/specta_gen.rs
src-tauri/src/commands/jobs.rs
src-tauri/src/commands/migration.rs
src-tauri/src/commands/mod.rs
src-tauri/src/commands/system.rs
src-tauri/src/domain/conflict.rs
src-tauri/src/domain/mod.rs
src-tauri/src/ipc/error.rs
src-tauri/src/ipc/mod.rs
src-tauri/src/lib.rs
src-tauri/src/main.rs
src-tauri/src/services/fileops.rs
src-tauri/src/services/jobs.rs
src-tauri/src/services/logging.rs
src-tauri/src/services/mod.rs
src-tauri/src/state.rs
src-tauri/src/storage/migration.rs
src-tauri/src/storage/mod.rs
src-tauri/src/storage/profiles.rs
src-tauri/src/storage/schema.rs
src-tauri/src/storage/settings.rs

## commands dir
jobs.rs
migration.rs
mod.rs
system.rs
# AI Context Snapshot

- Date: 2026-11-22T23:15:48+05:30
- Host: uday-Lenovo-C40-30
- Working directory: /home/uday/Music/SyncUp
- User: uday

## Git state

Branch: v2-migration

### git status --short
```bash
?? context-lean.md
```

### git log -n 5 --oneline
```bash
dc3af22 fix: run gate cargo commands against src-tauri manifest
afe71ee chore: add verification gate script
ff6e07b chore: add local AI harness tools
19c04df feat: phase 4c jobs engine and ipc bindings
cc56a06 feat: phase 4b file safety engine
```

## Requested context


### File: src-tauri/Cargo.toml

```toml
[package]
name = "syncup"
version = "2.0.0"
description = "Safe Android phone photo/video backup and Google Takeout organizer"
authors = ["SyncUp contributors"]
edition = "2021"
rust-version = "1.100"
license = "MIT"
repository = "https://github.com/Lotus-Projects-3421/syncup"

[lib]
name = "syncup_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[[bin]]
name = "syncup"
path = "src/main.rs"

[[bin]]
name = "specta-gen"
path = "src/bin/specta_gen.rs"

[build-dependencies]
tauri-build = { version = "2.7.6", features = [] }

[dependencies]
# blake2 0.10.6 (2023-10-09)
blake2 = "0.10.6"
r2d2 = "0.8.10"
r2d2_sqlite = "0.37.1"
rusqlite = { version = "0.42", features = ["bundled", "backup"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
specta = { version = "=2.0.0-rc.28", features = ["derive", "function"] }
specta-typescript = "0.0.15"
tauri = { version = "2.12.7", features = [] }
tauri-specta = { version = "=2.0.0-rc.29", features = ["derive", "typescript"] }
# tempfile 3.27.0 (promoted from dev-dependencies)
tempfile = "3.27.0"
thiserror = "2.0.21"
# tokio 1.54.1 (2026-02-18)
tokio = { version = "1.54.1", features = ["sync", "time", "rt", "rt-multi-thread", "macros"] }
# tokio-util 0.7.20 (2026-02-18)
tokio-util = "0.7.20"
tracing = "0.1.44"
tracing-appender = "0.2.5"
tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }
uuid = { version = "1", features = ["v4", "serde"] }

[target.'cfg(unix)'.dependencies]
# libc 0.2.191 (2026-02-18)
libc = "0.2.191"

[target.'cfg(windows)'.dependencies]
# windows-sys 0.59.0 (2024-09-08)
windows-sys = { version = "0.59.0", features = ["Win32_Storage_FileSystem", "Win32_Foundation"] }

[features]
custom-protocol = ["tauri/custom-protocol"]

[dev-dependencies]
sha2 = "0.10"
```


### File: src-tauri/src/ipc/error.rs

```rs
use serde::Serialize;
use specta::Type;

pub const DB_ERROR: &str = "DB_ERROR";
pub const MIGRATION_FAILED: &str = "MIGRATION_FAILED";
pub const UNKNOWN: &str = "UNKNOWN";

pub const DISK_FULL: &str = "DISK_FULL";
pub const PERMISSION_DENIED: &str = "PERMISSION_DENIED";
pub const IO_ERROR: &str = "IO_ERROR";
pub const PUBLISH_PENDING_DURABILITY: &str = "PUBLISH_PENDING_DURABILITY";
pub const CANCELLED: &str = "CANCELLED";
pub const VALIDATION_ERROR: &str = "VALIDATION_ERROR";
pub const VERIFY_FAILED: &str = "VERIFY_FAILED";
pub const PUBLISH_FAILED: &str = "PUBLISH_FAILED";

/// Structured error returned across the Tauri IPC boundary.
///
/// Messages are deliberately generic: internal paths, SQL text, and stack traces
/// must never be exposed to the webview.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

impl IpcError {
    #[must_use]
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn db(message: &str) -> Self {
        Self::new(DB_ERROR, message)
    }

    #[must_use]
    pub fn migration(message: &str) -> Self {
        Self::new(MIGRATION_FAILED, message)
    }

    #[must_use]
    pub fn disk_full(message: &str) -> Self {
        Self::new(DISK_FULL, message)
    }

    #[must_use]
    pub fn permission_denied(message: &str) -> Self {
        Self::new(PERMISSION_DENIED, message)
    }

    #[must_use]
    pub fn io(message: &str) -> Self {
        Self::new(IO_ERROR, message)
    }

    #[must_use]
    pub fn publish_pending_durability(message: &str) -> Self {
        Self::new(PUBLISH_PENDING_DURABILITY, message)
    }

    #[must_use]
    pub fn cancelled(message: &str) -> Self {
        Self::new(CANCELLED, message)
    }

    #[must_use]
    pub fn validation(message: &str) -> Self {
        Self::new(VALIDATION_ERROR, message)
    }

    #[must_use]
    pub fn verify_failed(message: &str) -> Self {
        Self::new(VERIFY_FAILED, message)
    }

    #[must_use]
    pub fn publish_failed(message: &str) -> Self {
        Self::new(PUBLISH_FAILED, message)
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self::new(UNKNOWN, "An unexpected error occurred.")
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for IpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_camel_case_code_and_message() {
        let error = IpcError::new(DB_ERROR, "The local database is unavailable.");
        let value = serde_json::to_value(&error);

        assert!(value.is_ok());
        if let Ok(value) = value {
            assert_eq!(
                value.get("code").and_then(serde_json::Value::as_str),
                Some(DB_ERROR)
            );
            assert_eq!(
                value.get("message").and_then(serde_json::Value::as_str),
                Some("The local database is unavailable.")
            );
        }
    }

    #[test]
    fn unknown_error_has_a_stable_code() {
        let error = IpcError::unknown();
        assert_eq!(error.code, UNKNOWN);
    }

    #[test]
    fn constructors_map_correct_error_codes() {
        assert_eq!(IpcError::disk_full("Disk is full").code, DISK_FULL);
        assert_eq!(
            IpcError::permission_denied("Permission denied").code,
            PERMISSION_DENIED
        );
        assert_eq!(IpcError::io("I/O error").code, IO_ERROR);
        assert_eq!(
            IpcError::publish_pending_durability("Publish pending durability").code,
            PUBLISH_PENDING_DURABILITY
        );
        assert_eq!(IpcError::cancelled("Cancelled").code, CANCELLED);
        assert_eq!(
            IpcError::validation("Validation error").code,
            VALIDATION_ERROR
        );
        assert_eq!(IpcError::verify_failed("Verify failed").code, VERIFY_FAILED);
        assert_eq!(
            IpcError::publish_failed("Publish failed").code,
            PUBLISH_FAILED
        );
    }
}
```


### File: src-tauri/src/state.rs

```rs
use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;

use crate::services::fileops::PathLockRegistry;
use crate::services::jobs::JobRegistry;
use crate::storage::DbPool;

pub struct AppState {
    pub db: DbPool,
    pub data_dir: PathBuf,
    pub jobs: JobRegistry,
    /// Keeps the non-blocking log writer alive for the process lifetime.
    pub _log_guard: WorkerGuard,
}

/// Equivalent state container for file operation path locks.
#[derive(Clone, Debug, Default)]
pub struct FileOpsState {
    pub path_locks: PathLockRegistry,
}
```


### File: src-tauri/src/lib.rs

```rs
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod commands;
pub mod ipc;
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
        commands::jobs::cancel_job
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
```


## API map: services/fileops.rs
63:pub fn resolve_destination_path(candidate: &Path, mode: ConflictMode) -> PathBuf {
71:pub fn reset_collision_tracker() {
80:pub struct PathLockRegistry {
84:impl PathLockRegistry {
86:    pub fn new() -> Self {
107:pub enum Source {
115:impl std::fmt::Debug for Source {
128:pub enum VerifyMode {
134:pub struct Destination {
142:pub struct Published {
149:pub enum FileOpError {
185:impl From<FileOpError> for IpcError {
252:impl Destination {
253:    pub fn validate_with_root(&self, approved_root: Option<&Path>) -> Result<(), FileOpError> {
340:    pub fn validate(&self) -> Result<(), FileOpError> {
771:pub fn cleanup_orphan_temp_files(dest_root: &Path) -> Result<usize, FileOpError> {

## API map: services/jobs.rs
36:pub struct JobId(Uuid);
38:impl JobId {
40:    pub fn new() -> Self {
45:impl Default for JobId {
51:impl std::fmt::Display for JobId {
57:impl Type for JobId {
69:pub enum JobKind {
85:pub enum JobStatus {
93:impl JobStatus {
102:pub struct JobProgress {
111:impl JobProgress {
114:    pub fn fraction(&self) -> Option<f64> {
125:pub struct Job {
142:pub struct JobProgressEvent {
150:pub struct JobHandle {
157:impl JobHandle {
182:    pub fn update_progress(&self, progress: JobProgress) {
201:    pub fn finish(&self, status: JobStatus, error: Option<String>) {
218:    pub fn cancel_token(&self) -> CancellationToken {
255:pub struct JobRegistry {
260:impl JobRegistry {
266:    pub fn new(app: Option<AppHandle>) -> Self {
274:    pub fn register(&self, kind: JobKind) -> (JobId, JobHandle, CancellationToken) {
286:    pub fn list(&self) -> Vec<Job> {
294:    pub fn get(&self, id: &JobId) -> Option<Job> {
302:    pub fn cancel(&self, id: &JobId) -> Result<(), IpcError> {

## API map: tests/fileops.rs
21:async fn copy_success_file_source() {
51:async fn copy_success_stream_source() {
85:async fn copy_size_mismatch_rejected() {
120:async fn copy_content_mismatch_rejected() {
151:async fn overwrite_existing_replaces_verified() {
183:async fn overwrite_existing_preserved_on_publish_failure() {
218:async fn overwrite_existing_preserved_on_verify_failure() {
250:async fn disk_full_error_mapped() {
292:async fn permission_denied_mapped() {
333:        // Chmod 0o555 permission test is unix-only; skipped on non-unix platforms.
344:    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
358:async fn cancel_mid_copy_leaves_no_temp() {
401:async fn cancel_preserves_existing_dest() {
436:async fn symlink_outside_root_rejected() {
475:async fn concurrent_same_destination_serialized() {
535:async fn same_run_collision_deterministic() {
590:fn orphan_temp_cleanup_on_startup() {
612:async fn publish_pending_durability_state_reported() {
661:fn windows_durability_compiles_only_on_windows() {
