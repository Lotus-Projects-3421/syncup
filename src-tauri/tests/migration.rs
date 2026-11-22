use std::path::Path;
use std::time::SystemTime;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use syncup_lib::storage::migration::{get_migration_status, migrate_legacy_import, MigrationError};
use syncup_lib::storage::open_pool;
use syncup_lib::storage::profiles::{load_profile, DeviceProfile};
use syncup_lib::storage::settings::{load_settings, AppSettings};
use tempfile::tempdir;

fn compute_file_sha256(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("failed to read file for sha256");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

fn create_legacy_db(path: &Path) {
    let conn = Connection::open(path).expect("failed to create legacy db");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;\
         PRAGMA synchronous=NORMAL;\
         CREATE TABLE processed (\
             rel_path TEXT PRIMARY KEY,\
             size INTEGER,\
             mtime INTEGER,\
             copied_at TEXT\
         );\
         INSERT INTO processed VALUES ('DCIM/Camera/IMG_0001.jpg', 1048576, 1700000000000000000, '2026-01-01T12:00:00');\
         INSERT INTO processed VALUES ('DCIM/Camera/IMG_0002.jpg', 2097152, 1700000060000000000, '2026-01-01T12:01:00');",
    )
    .expect("failed to seed legacy db");
}

fn create_legacy_files(dir: &Path, device_id: &str) {
    // 1. settings.json
    let settings = AppSettings {
        theme: "light".to_string(),
        window_width: 1400,
        window_height: 900,
    };
    let settings_json = serde_json::to_string_pretty(&settings).unwrap();
    std::fs::write(dir.join("settings.json"), settings_json).unwrap();

    // 2. profile_<id>.json
    let profile = DeviceProfile {
        id: device_id.to_string(),
        destination: "/tmp/legacy_photos".to_string(),
        sorting: "month".to_string(),
        filter: "photos".to_string(),
        conflict: "keep".to_string(),
        folders: vec!["DCIM/Camera".to_string(), "Pictures".to_string()],
    };
    let profile_json = serde_json::to_string_pretty(&profile).unwrap();
    std::fs::write(dir.join(format!("profile_{device_id}.json")), profile_json).unwrap();

    // 3. state_<id>.json
    let state_paths = vec![
        "DCIM/Camera/IMG_0001.jpg",
        "DCIM/Camera/IMG_9999_from_json.jpg",
    ];
    let state_json = serde_json::to_string_pretty(&state_paths).unwrap();
    std::fs::write(dir.join(format!("state_{device_id}.json")), state_json).unwrap();
}

#[test]
fn fresh_import() {
    let target_dir = tempdir().unwrap();
    let legacy_dir = tempdir().unwrap();

    let legacy_db_path = legacy_dir.path().join("state_phone-test.db");
    create_legacy_db(&legacy_db_path);
    create_legacy_files(legacy_dir.path(), "phone-test");

    let pool = open_pool(target_dir.path()).unwrap();

    let report = migrate_legacy_import(
        &pool,
        target_dir.path(),
        Some(&legacy_db_path),
        Some(legacy_dir.path()),
    )
    .unwrap();

    assert!(report.success);
    assert!(!report.already_migrated);
    assert_eq!(report.db_rows_imported, 2);
    assert_eq!(report.json_records_imported, 2);
    assert!(report.settings_imported);
    assert_eq!(report.profiles_imported, 1);
    assert_eq!(report.snapshot_paths.len(), 1);

    // Verify v2 database contents
    let conn = pool.get().unwrap();
    let user_ver: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(user_ver, 2);

    let row_count: i64 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get(0))
        .unwrap();
    // 2 from DB + 1 unique new from JSON = 3 rows in total
    assert_eq!(row_count, 3);

    let (device_id, size, mtime_ns): (String, Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT device_id, size, mtime_ns FROM processed WHERE rel_path = 'DCIM/Camera/IMG_0001.jpg'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(device_id, "phone-test");
    assert_eq!(size, Some(1048576));
    assert_eq!(mtime_ns, Some(1700000000000000000));

    let json_only_size: Option<i64> = conn
        .query_row(
            "SELECT size FROM processed WHERE rel_path = 'DCIM/Camera/IMG_9999_from_json.jpg'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(json_only_size, None);

    // Verify marker
    let marker_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM legacy_import_marker WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1);

    // Verify status
    let status = get_migration_status(&pool, target_dir.path()).unwrap();
    assert!(status.is_migrated);
    assert_eq!(status.schema_version, 2);
    assert_eq!(status.processed_count, 3);
    assert_eq!(status.profiles_count, 1);
    assert!(status.has_settings);

    // Verify settings and profile on disk
    let settings = load_settings(target_dir.path()).unwrap();
    assert_eq!(settings.theme, "light");
    assert_eq!(settings.window_width, 1400);

    let profile = load_profile(target_dir.path(), "phone-test")
        .unwrap()
        .unwrap();
    assert_eq!(profile.destination, "/tmp/legacy_photos");
    assert_eq!(profile.sorting, "month");
}

#[test]
fn idempotent_rerun() {
    let target_dir = tempdir().unwrap();
    let legacy_dir = tempdir().unwrap();

    let legacy_db_path = legacy_dir.path().join("state_rerun.db");
    create_legacy_db(&legacy_db_path);
    create_legacy_files(legacy_dir.path(), "rerun");

    let pool = open_pool(target_dir.path()).unwrap();

    let first_report = migrate_legacy_import(
        &pool,
        target_dir.path(),
        Some(&legacy_db_path),
        Some(legacy_dir.path()),
    )
    .unwrap();
    assert!(first_report.success);
    assert!(!first_report.already_migrated);

    let conn = pool.get().unwrap();
    let count_before: i64 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get(0))
        .unwrap();

    // Second run
    let second_report = migrate_legacy_import(
        &pool,
        target_dir.path(),
        Some(&legacy_db_path),
        Some(legacy_dir.path()),
    )
    .unwrap();

    assert!(second_report.success);
    assert!(second_report.already_migrated);
    assert_eq!(second_report.db_rows_imported, 0);

    let count_after: i64 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count_before, count_after);
}

#[test]
fn interrupted_import_resume() {
    let target_dir = tempdir().unwrap();
    let legacy_dir = tempdir().unwrap();

    let legacy_db_path = legacy_dir.path().join("state_resume.db");
    create_legacy_db(&legacy_db_path);
    create_legacy_files(legacy_dir.path(), "resume");

    let pool = open_pool(target_dir.path()).unwrap();

    // Simulate an interrupted migration where some rows were inserted, but no marker was written
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO processed (device_id, rel_path, size, mtime_ns, copied_at)\
             VALUES ('resume', 'DCIM/Camera/IMG_0001.jpg', 1048576, 1700000000000000000, '2026-01-01T12:00:00')",
            [],
        )
        .unwrap();
        // Ensure marker is NOT present
        let marker_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM legacy_import_marker WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(marker_count, 0);
    }

    // Now run migrate_legacy_import to resume
    let report = migrate_legacy_import(
        &pool,
        target_dir.path(),
        Some(&legacy_db_path),
        Some(legacy_dir.path()),
    )
    .unwrap();

    assert!(report.success);
    assert!(!report.already_migrated);

    // Verify all rows are now present and marker is recorded
    let conn = pool.get().unwrap();
    let total_rows: i64 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total_rows, 3);

    let marker_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM legacy_import_marker WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 1);
}

#[test]
fn corrupt_snapshot_rejected() {
    let target_dir = tempdir().unwrap();
    let legacy_dir = tempdir().unwrap();

    let corrupt_db_path = legacy_dir.path().join("state_corrupted.db");
    {
        let conn = Connection::open(&corrupt_db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\
             CREATE TABLE processed (rel_path TEXT PRIMARY KEY, size INTEGER, mtime INTEGER, copied_at TEXT);\
             INSERT INTO processed VALUES ('photo.jpg', 1234, 5678, '2026-01-01');",
        )
        .unwrap();
    }

    // Overwrite pages beyond the 100-byte SQLite header with corrupt garbage
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&corrupt_db_path)
            .unwrap();
        file.seek(SeekFrom::Start(104)).unwrap();
        file.write_all(&[0x00, 0x01, 0x02, 0x03, 0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA])
            .unwrap();
        file.sync_all().unwrap();
    }

    let pool = open_pool(target_dir.path()).unwrap();

    let result = migrate_legacy_import(&pool, target_dir.path(), Some(&corrupt_db_path), None);

    assert!(result.is_err());
    match result {
        Err(MigrationError::IntegrityCheckFailed(details)) => {
            assert!(!details.is_empty());
        }
        Err(MigrationError::SnapshotFailed(_)) | Err(MigrationError::Sqlite(_)) => {}
        Err(other) => panic!("expected integrity failure or snapshot failure, got: {other:?}"),
        Ok(_) => panic!("corrupt database should have been rejected!"),
    }

    // Ensure no corrupted rows or marker were inserted into v2
    let conn = pool.get().unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);

    let markers: i64 = conn
        .query_row("SELECT count(*) FROM legacy_import_marker", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(markers, 0);
}

#[test]
fn wal_source_handled() {
    let target_dir = tempdir().unwrap();
    let legacy_dir = tempdir().unwrap();

    let wal_db_path = legacy_dir.path().join("state_wal_test.db");

    // Create DB with disabled auto-checkpoint so commits stay in WAL
    {
        let conn = Connection::open(&wal_db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\
             PRAGMA wal_autocheckpoint=0;\
             CREATE TABLE processed (\
                 rel_path TEXT PRIMARY KEY,\
                 size INTEGER,\
                 mtime INTEGER,\
                 copied_at TEXT\
             );\
             INSERT INTO processed VALUES ('file1.jpg', 100, 1000, '2026-01-01');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO processed VALUES ('file2_in_wal.jpg', 200, 2000, '2026-01-02');",
            [],
        )
        .unwrap();

        // Verify WAL sidecar exists
        let wal_sidecar = legacy_dir.path().join("state_wal_test.db-wal");
        assert!(
            wal_sidecar.exists(),
            "WAL file should exist with uncheckpointed commits"
        );
    }

    let pool = open_pool(target_dir.path()).unwrap();

    let report = migrate_legacy_import(&pool, target_dir.path(), Some(&wal_db_path), None).unwrap();

    assert!(report.success);
    assert_eq!(report.db_rows_imported, 2);

    // Verify both rows from the uncheckpointed WAL exist in v2
    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM processed", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);

    let exists_wal_row: bool = conn
        .query_row(
            "SELECT count(*) FROM processed WHERE rel_path = 'file2_in_wal.jpg'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c == 1)
        .unwrap();
    assert!(exists_wal_row, "row committed in WAL must be migrated");
}

#[test]
fn source_unchanged_after_migration() {
    let target_dir = tempdir().unwrap();
    let legacy_dir = tempdir().unwrap();

    let legacy_db_path = legacy_dir.path().join("state_audit.db");
    create_legacy_db(&legacy_db_path);
    create_legacy_files(legacy_dir.path(), "audit");

    let settings_path = legacy_dir.path().join("settings.json");
    let profile_path = legacy_dir.path().join("profile_audit.json");
    let state_json_path = legacy_dir.path().join("state_audit.json");

    // Record SHA-256 and mtime before migration
    let db_sha_before = compute_file_sha256(&legacy_db_path);
    let db_mtime_before: SystemTime = std::fs::metadata(&legacy_db_path)
        .unwrap()
        .modified()
        .unwrap();

    let settings_sha_before = compute_file_sha256(&settings_path);
    let settings_mtime_before: SystemTime = std::fs::metadata(&settings_path)
        .unwrap()
        .modified()
        .unwrap();

    let profile_sha_before = compute_file_sha256(&profile_path);
    let profile_mtime_before: SystemTime = std::fs::metadata(&profile_path)
        .unwrap()
        .modified()
        .unwrap();

    let state_json_sha_before = compute_file_sha256(&state_json_path);
    let state_json_mtime_before: SystemTime = std::fs::metadata(&state_json_path)
        .unwrap()
        .modified()
        .unwrap();

    // Run migration
    let pool = open_pool(target_dir.path()).unwrap();
    let report = migrate_legacy_import(
        &pool,
        target_dir.path(),
        Some(&legacy_db_path),
        Some(legacy_dir.path()),
    )
    .unwrap();
    assert!(report.success);

    // Record SHA-256 and mtime after migration
    let db_sha_after = compute_file_sha256(&legacy_db_path);
    let db_mtime_after: SystemTime = std::fs::metadata(&legacy_db_path)
        .unwrap()
        .modified()
        .unwrap();

    let settings_sha_after = compute_file_sha256(&settings_path);
    let settings_mtime_after: SystemTime = std::fs::metadata(&settings_path)
        .unwrap()
        .modified()
        .unwrap();

    let profile_sha_after = compute_file_sha256(&profile_path);
    let profile_mtime_after: SystemTime = std::fs::metadata(&profile_path)
        .unwrap()
        .modified()
        .unwrap();

    let state_json_sha_after = compute_file_sha256(&state_json_path);
    let state_json_mtime_after: SystemTime = std::fs::metadata(&state_json_path)
        .unwrap()
        .modified()
        .unwrap();

    // Assert identical hashes and mtimes
    assert_eq!(db_sha_before, db_sha_after, "DB SHA-256 must match exactly");
    assert_eq!(
        db_mtime_before, db_mtime_after,
        "DB mtime must match exactly"
    );

    assert_eq!(
        settings_sha_before, settings_sha_after,
        "settings.json SHA-256 must match exactly"
    );
    assert_eq!(
        settings_mtime_before, settings_mtime_after,
        "settings.json mtime must match exactly"
    );

    assert_eq!(
        profile_sha_before, profile_sha_after,
        "profile.json SHA-256 must match exactly"
    );
    assert_eq!(
        profile_mtime_before, profile_mtime_after,
        "profile.json mtime must match exactly"
    );

    assert_eq!(
        state_json_sha_before, state_json_sha_after,
        "state.json SHA-256 must match exactly"
    );
    assert_eq!(
        state_json_mtime_before, state_json_mtime_after,
        "state.json mtime must match exactly"
    );
}
