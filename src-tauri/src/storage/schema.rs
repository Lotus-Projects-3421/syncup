use rusqlite::{Connection, Error};

pub const SCHEMA_VERSION: i32 = 2;

const SCHEMA_V2_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS processed (
    device_id TEXT NOT NULL,
    rel_path TEXT NOT NULL,
    size INTEGER,
    mtime_ns INTEGER,
    copied_at TEXT,
    PRIMARY KEY (device_id, rel_path)
);

CREATE INDEX IF NOT EXISTS idx_processed_copied_at ON processed(copied_at);
CREATE INDEX IF NOT EXISTS idx_processed_device ON processed(device_id);

CREATE TABLE IF NOT EXISTS legacy_import_marker (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    imported_at TEXT NOT NULL,
    source_db TEXT,
    source_json_dir TEXT,
    processed_count INTEGER NOT NULL
);
"#;

/// Initializes or upgrades the SQLite database to schema v2.
///
/// Ensures tables, indexes, and schema version markers exist. Sets `PRAGMA user_version = 2`.
pub fn init_schema(conn: &Connection) -> Result<(), Error> {
    let current_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if current_version < SCHEMA_VERSION {
        conn.execute_batch(SCHEMA_V2_DDL)?;
        conn.execute(
            "INSERT INTO schema_version (id, version) VALUES (1, ?1)\
             ON CONFLICT(id) DO UPDATE SET version = excluded.version",
            [SCHEMA_VERSION],
        )?;
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        tracing::info!(
            previous_version = current_version,
            new_version = SCHEMA_VERSION,
            "database schema migrated to v2"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_schema_creates_tables_and_sets_version() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);

        let schema_ver: i32 = conn
            .query_row("SELECT version FROM schema_version WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(schema_ver, 2);

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"processed".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
        assert!(tables.contains(&"legacy_import_marker".to_string()));
    }
}
