use tracing_appender::non_blocking::WorkerGuard;

use crate::storage::DbPool;

pub struct AppState {
    pub db: DbPool,
    /// Keeps the non-blocking log writer alive for the process lifetime.
    pub _log_guard: WorkerGuard,
}
