use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;
use thiserror::Error;

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to create the application data directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("failed to open the local database")]
    OpenDatabase(#[source] r2d2::Error),
}

/// Opens the SQLite connection pool in the platform application data directory.
///
/// Phase 2 intentionally creates no tables and does not write `PRAGMA user_version`.
/// Writing a schema version here would make Phase 3 believe its migrations had
/// already run. The version is only read and reported.
pub fn open_pool(data_dir: &Path) -> Result<DbPool, StorageError> {
    std::fs::create_dir_all(data_dir).map_err(StorageError::CreateDirectory)?;

    let manager =
        SqliteConnectionManager::file(data_dir.join("state_v2.db")).with_init(|connection| {
            connection
                .execute_batch(
                    "PRAGMA journal_mode=WAL;\
                     PRAGMA synchronous=NORMAL;\
                     PRAGMA foreign_keys=ON;\
                     PRAGMA busy_timeout=5000;",
                )
                .map_err(|error| {
                    tracing::error!(error = %error, "failed to configure a SQLite connection");
                    error
                })
        });

    r2d2::Pool::builder()
        .max_size(4)
        .build(manager)
        .map_err(StorageError::OpenDatabase)
}
