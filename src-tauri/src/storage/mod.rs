use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;
use thiserror::Error;

pub mod migration;
pub mod profiles;
pub mod schema;
pub mod settings;

pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to create the application data directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("failed to open the local database")]
    OpenDatabase(#[source] r2d2::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Opens the SQLite connection pool in the platform application data directory
/// and ensures Schema v2 tables and version markers are initialized.
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

    let pool = r2d2::Pool::builder()
        .max_size(4)
        .build(manager)
        .map_err(StorageError::OpenDatabase)?;

    let conn = pool.get().map_err(StorageError::OpenDatabase)?;
    schema::init_schema(&conn).map_err(StorageError::Sqlite)?;

    Ok(pool)
}
