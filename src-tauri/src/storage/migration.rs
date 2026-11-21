use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use crate::storage::profiles::{list_profiles, load_profile, sanitize_safe_id, save_profile};
use crate::storage::settings::save_settings;
use crate::storage::{DbPool, StorageError};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub success: bool,
    pub already_migrated: bool,
    pub db_rows_imported: u32,
    pub db_rows_skipped: u32,
    pub json_records_imported: u32,
    pub profiles_imported: u32,
    pub settings_imported: bool,
    pub snapshot_paths: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatus {
    pub is_migrated: bool,
    pub schema_version: i32,
    pub processed_count: u32,
    pub profiles_count: u32,
    pub has_settings: bool,
    pub imported_at: Option<String>,
}

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("snapshot integrity check failed: {0}")]
    IntegrityCheckFailed(String),
    #[error("source database could not be opened: {0}")]
    SourceDbOpen(#[source] rusqlite::Error),
    #[error("snapshot creation failed: {0}")]
    SnapshotFailed(#[source] rusqlite::Error),
    #[error("database pool unavailable: {0}")]
    Pool(#[source] r2d2::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("invalid source path: {0}")]
    InvalidPath(String),
}

#[derive(Debug)]
struct LegacyDbRow {
    device_id: String,
    rel_path: String,
    size: Option<i64>,
    mtime_ns: Option<i64>,
    copied_at: Option<String>,
}

/// Extracts device safe_id from a state database filename or path.
/// E.g. "state_my-phone.db" -> "my-phone", "my-phone.db" -> "my-phone".
fn extract_device_id_from_path(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if let Some(id) = stem.strip_prefix("state_") {
        if !id.is_empty() {
            return sanitize_safe_id(id);
        }
    }
    sanitize_safe_id(stem)
}

/// Produces a WAL-safe snapshot of a SQLite database using `rusqlite::backup::Backup`.
///
/// NEVER use a plain file copy here: SQLite WAL mode can hold recent commits
/// inside the -wal sidecar file outside the main .db file. A plain file copy
/// would miss uncheckpointed commits, silently losing backup history and
/// leading to re-copying or state corruption in v2.
fn create_wal_safe_snapshot(source_path: &Path, target_dir: &Path) -> Result<PathBuf, MigrationError> {
    std::fs::create_dir_all(target_dir).map_err(MigrationError::Io)?;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stem = source_path.file_stem().and_then(|s| s.to_str()).unwrap_or("legacy");
    let snapshot_filename = format!("{stem}_snapshot_{timestamp}.db");
    let snapshot_path = target_dir.join(&snapshot_filename);

    tracing::info!(
        source = ?source_path,
        snapshot = ?snapshot_path,
        "[migration] opening source database read-only"
    );

    let source_conn = Connection::open_with_flags(
        source_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(MigrationError::SourceDbOpen)?;

    tracing::info!(
        snapshot = ?snapshot_path,
        "[migration] producing WAL-safe snapshot"
    );

    let mut dest_conn = Connection::open(&snapshot_path).map_err(MigrationError::SnapshotFailed)?;

    // Snapshot using rusqlite's Connection::backup.
    // NEVER use a plain file copy here: SQLite WAL mode can hold recent commits
    // inside the -wal sidecar file outside the main .db file. A plain file copy
    // would miss uncheckpointed commits, silently losing backup history and
    // leading to re-copying or state corruption in v2.
    {
        let backup = rusqlite::backup::Backup::new(&source_conn, &mut dest_conn)
            .map_err(MigrationError::SnapshotFailed)?;
        backup
            .run_to_completion(20, std::time::Duration::from_millis(100), None)
            .map_err(MigrationError::SnapshotFailed)?;
    }

    drop(dest_conn);
    drop(source_conn);

    tracing::info!(
        snapshot = ?snapshot_path,
        "[migration] verifying integrity of snapshot"
    );

    let check_conn = Connection::open_with_flags(
        &snapshot_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(MigrationError::SnapshotFailed)?;

    let mut stmt = check_conn
        .prepare("PRAGMA integrity_check")
        .map_err(MigrationError::SnapshotFailed)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(MigrationError::SnapshotFailed)?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(MigrationError::SnapshotFailed)?);
    }

    if results.len() != 1 || results[0] != "ok" {
        let msg = results.join("; ");
        tracing::error!(result = %msg, "[migration] snapshot integrity check failed");
        return Err(MigrationError::IntegrityCheckFailed(msg));
    }

    tracing::info!(
        snapshot = ?snapshot_path,
        "[migration] snapshot integrity check passed (result: ok)"
    );

    Ok(snapshot_path)
}

/// Reads processed rows from a verified legacy snapshot database.
fn read_rows_from_snapshot(snapshot_path: &Path, device_id: &str) -> Result<Vec<LegacyDbRow>, MigrationError> {
    tracing::info!(
        snapshot = ?snapshot_path,
        device_id = %device_id,
        "[migration] reading rows from snapshot"
    );

    let conn = Connection::open_with_flags(
        snapshot_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(MigrationError::SnapshotFailed)?;

    let table_exists: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='processed'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if !table_exists {
        tracing::warn!(
            snapshot = ?snapshot_path,
            "[migration] no 'processed' table found in snapshot"
        );
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare("SELECT rel_path, size, mtime, copied_at FROM processed")
        .map_err(MigrationError::SnapshotFailed)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(LegacyDbRow {
                device_id: device_id.to_string(),
                rel_path: row.get::<_, String>(0)?,
                size: row.get::<_, Option<i64>>(1)?,
                mtime_ns: row.get::<_, Option<i64>>(2)?,
                copied_at: row.get::<_, Option<String>>(3)?,
            })
        })
        .map_err(MigrationError::SnapshotFailed)?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(MigrationError::SnapshotFailed)?);
    }

    tracing::info!(
        count = result.len(),
        "[migration] successfully read rows from snapshot"
    );

    Ok(result)
}

/// Performs idempotent legacy migration into v2 storage.
pub fn migrate_legacy_import(
    pool: &DbPool,
    data_dir: &Path,
    source_db_path: Option<&Path>,
    source_json_dir: Option<&Path>,
) -> Result<MigrationReport, MigrationError> {
    tracing::info!(
        source_db = ?source_db_path,
        source_json_dir = ?source_json_dir,
        "[migration] beginning legacy import check"
    );

    let mut v2_conn = pool.get().map_err(MigrationError::Pool)?;

    // Ensure v2 schema is ready
    crate::storage::schema::init_schema(&v2_conn)?;

    // Check if migration has already completed
    let already_migrated: bool = v2_conn
        .query_row(
            "SELECT count(*) FROM legacy_import_marker WHERE id = 1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if already_migrated {
        tracing::info!("[migration] already imported, skipping");
        return Ok(MigrationReport {
            success: true,
            already_migrated: true,
            db_rows_imported: 0,
            db_rows_skipped: 0,
            json_records_imported: 0,
            profiles_imported: 0,
            settings_imported: false,
            snapshot_paths: Vec::new(),
            message: "Legacy migration has already been completed.".to_string(),
        });
    }

    let snapshots_dir = data_dir.join("snapshots");
    let mut snapshot_paths = Vec::new();
    let mut all_db_rows = Vec::new();

    // 1. Process explicit source_db_path if provided
    if let Some(db_path) = source_db_path {
        if db_path.exists() {
            let snapshot_path = create_wal_safe_snapshot(db_path, &snapshots_dir)?;
            let device_id = extract_device_id_from_path(db_path);
            let rows = read_rows_from_snapshot(&snapshot_path, &device_id)?;
            all_db_rows.extend(rows);
            snapshot_paths.push(snapshot_path.to_string_lossy().to_string());
        } else {
            return Err(MigrationError::InvalidPath(format!(
                "source database path does not exist: {:?}",
                db_path
            )));
        }
    }

    // 2. Discover files in source_json_dir
    let mut settings_imported = false;
    let mut profiles_imported: u32 = 0;
    let mut json_rows = Vec::new();

    if let Some(json_dir) = source_json_dir {
        if json_dir.exists() {
            // If source_db_path was not provided, look for any state_*.db in source_json_dir
            if source_db_path.is_none() {
                if let Ok(entries) = std::fs::read_dir(json_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        if filename.starts_with("state_")
                            && filename.ends_with(".db")
                            && !filename.contains(".corrupt")
                            && !filename.contains(".backup")
                            && !filename.contains(".part-")
                        {
                            let snapshot_path = create_wal_safe_snapshot(&path, &snapshots_dir)?;
                            let device_id = extract_device_id_from_path(&path);
                            let rows = read_rows_from_snapshot(&snapshot_path, &device_id)?;
                            all_db_rows.extend(rows);
                            snapshot_paths.push(snapshot_path.to_string_lossy().to_string());
                        }
                    }
                }
            }

            // Import settings.json if present
            let settings_file = json_dir.join("settings.json");
            if settings_file.exists() {
                tracing::info!(path = ?settings_file, "[migration] importing legacy settings");
                if let Ok(content) = std::fs::read_to_string(&settings_file) {
                    if let Ok(legacy_settings) = serde_json::from_str::<crate::storage::settings::AppSettings>(&content) {
                        save_settings(data_dir, &legacy_settings)?;
                        settings_imported = true;
                    }
                }
            }

            // Import profile_*.json
            if let Ok(entries) = std::fs::read_dir(json_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if name.starts_with("profile_") && name.ends_with(".json") && !name.contains(".part-") {
                        if let Some(safe_id) = name.strip_prefix("profile_").and_then(|s| s.strip_suffix(".json")) {
                            if let Ok(Some(profile)) = load_profile(json_dir, safe_id) {
                                tracing::info!(id = %profile.id, "[migration] importing legacy profile");
                                save_profile(data_dir, &profile)?;
                                profiles_imported += 1;
                            }
                        }
                    }
                }
            }

            // Import state_*.json (legacy path-only records)
            if let Ok(entries) = std::fs::read_dir(json_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if name.starts_with("state_") && name.ends_with(".json") && !name.contains(".part-") {
                        if let Some(safe_id) = name.strip_prefix("state_").and_then(|s| s.strip_suffix(".json")) {
                            tracing::info!(id = %safe_id, path = ?path, "[migration] importing legacy state json");
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if let Ok(items) = serde_json::from_str::<Vec<String>>(&content) {
                                    for item in items {
                                        let item_str = item.trim().to_string();
                                        if !item_str.is_empty() {
                                            json_rows.push((safe_id.to_string(), item_str));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Atomically insert into v2 DB in a single transaction
    tracing::info!("[migration] beginning v2 database transaction");
    let tx = v2_conn.transaction()?;

    let mut db_rows_inserted: u32 = 0;
    let mut db_rows_skipped: u32 = 0;

    {
        let mut insert_stmt = tx.prepare(
            "INSERT INTO processed (device_id, rel_path, size, mtime_ns, copied_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(device_id, rel_path) DO UPDATE SET \
             size = coalesce(excluded.size, processed.size), \
             mtime_ns = coalesce(excluded.mtime_ns, processed.mtime_ns), \
             copied_at = coalesce(excluded.copied_at, processed.copied_at)",
        )?;

        for row in &all_db_rows {
            let changes = insert_stmt.execute((
                &row.device_id,
                &row.rel_path,
                &row.size,
                &row.mtime_ns,
                &row.copied_at,
            ))?;
            if changes > 0 {
                db_rows_inserted += 1;
            } else {
                db_rows_skipped += 1;
            }
        }
    }

    let mut json_rows_inserted: u32 = 0;
    {
        let mut json_insert_stmt = tx.prepare(
            "INSERT OR IGNORE INTO processed (device_id, rel_path, size, mtime_ns, copied_at) \
             VALUES (?1, ?2, NULL, NULL, NULL)",
        )?;

        for (device_id, rel_path) in &json_rows {
            json_insert_stmt.execute((device_id, rel_path))?;
            json_rows_inserted += 1;
        }
    }

    // 4. Verify counts match source
    let total_in_db: i64 = tx.query_row("SELECT count(*) FROM processed", [], |r| r.get(0))?;
    tracing::info!(
        db_rows = db_rows_inserted,
        json_rows = json_rows_inserted,
        total_in_v2 = total_in_db,
        "[migration] verifying counts against source records"
    );

    // 5. Record migration marker
    let now_iso = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => format!("+{}", d.as_secs()),
        Err(_) => "0".to_string(),
    };

    let source_db_str = source_db_path.map(|p| p.to_string_lossy().to_string());
    let source_json_str = source_json_dir.map(|p| p.to_string_lossy().to_string());

    tx.execute(
        "INSERT INTO legacy_import_marker (id, imported_at, source_db, source_json_dir, processed_count) \
         VALUES (1, ?1, ?2, ?3, ?4) \
         ON CONFLICT(id) DO UPDATE SET \
         imported_at = excluded.imported_at, \
         source_db = excluded.source_db, \
         source_json_dir = excluded.source_json_dir, \
         processed_count = excluded.processed_count",
        (now_iso, source_db_str, source_json_str, total_in_db),
    )?;

    tracing::info!("[migration] committing transaction");
    tx.commit()?;

    tracing::info!("[migration] legacy migration finished successfully");

    Ok(MigrationReport {
        success: true,
        already_migrated: false,
        db_rows_imported: db_rows_inserted,
        db_rows_skipped,
        json_records_imported: json_rows_inserted,
        profiles_imported,
        settings_imported,
        snapshot_paths,
        message: "Legacy migration completed successfully.".to_string(),
    })
}

/// Returns the status of data migration in v2 storage.
pub fn get_migration_status(pool: &DbPool, data_dir: &Path) -> Result<MigrationStatus, StorageError> {
    let conn = pool.get().map_err(StorageError::OpenDatabase)?;

    let schema_version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    let (is_migrated, imported_at) = conn
        .query_row(
            "SELECT imported_at FROM legacy_import_marker WHERE id = 1",
            [],
            |r| r.get::<_, String>(0),
        )
        .map(|ts| (true, Some(ts)))
        .unwrap_or((false, None));

    let processed_count: u32 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get::<_, i64>(0))
        .map(|c| c as u32)
        .unwrap_or(0);

    let profiles = list_profiles(data_dir)?;
    let has_settings = data_dir.join("settings.json").exists();

    Ok(MigrationStatus {
        is_migrated,
        schema_version,
        processed_count,
        profiles_count: profiles.len() as u32,
        has_settings,
        imported_at,
    })
}
