use std::path::Path;

use thiserror::Error;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("failed to create the log directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("failed to initialize the logging subscriber")]
    Initialize,
}

/// Initialises structured logging to a daily file plus stderr.
///
/// The returned guard must be kept alive for the lifetime of the application so
/// the non-blocking writer can flush. Logs intentionally exclude user data and
/// absolute filesystem paths.
pub fn init(log_dir: &Path) -> Result<WorkerGuard, LoggingError> {
    std::fs::create_dir_all(log_dir).map_err(LoggingError::CreateDirectory)?;

    let file_appender = tracing_appender::rolling::daily(log_dir, "syncup.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init()
        .map_err(|_| LoggingError::Initialize)?;

    Ok(guard)
}
