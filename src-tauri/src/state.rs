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
