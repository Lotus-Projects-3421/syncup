# docs/PLAN.md

## §1 Target architecture overview

**Backend service map**: The Rust backend is divided into a thin `commands` layer for IPC, a `services` layer for orchestrating workflows (backup, Wi-Fi, organizer, dedup), a `domain` layer for pure business rules (date parsing, naming, conflict resolution), and a `storage` layer for SQLite and JSON persistence. Platform-specific code (like Linux MTP/GVFS) is isolated in a `platform` module to allow future Windows/macOS implementations without touching core logic.

**Frontend structure**: The Svelte 5 frontend operates as a single-page application using Svelte runes (`$state`, `$derived`) for reactive state management, completely avoiding legacy stores unless required by third-party libraries. UI components are built on `shadcn-svelte` and styled with Tailwind CSS v4, organized into feature-based routes and reusable component libraries.

**IPC boundary**: Communication between Rust and TypeScript is strictly governed by `tauri-specta`, which generates TypeScript definitions directly from Rust structs and command signatures. Rust commands are the absolute source of truth; hand-edited bindings are forbidden, and all untrusted frontend inputs are re-validated in Rust before any filesystem or database operation.

**Storage layer**: Persistence relies on `rusqlite` with WAL mode enabled for the backup state database, managed via an `r2d2_sqlite` connection pool to safely handle concurrent Tauri command executions. Global settings and device profiles remain atomic JSON files, while legacy state migration is handled through a strict, versioned, and WAL-aware import pipeline.

## §2 Target folder tree

```text
project/
├── src/                      # Svelte 5 frontend
│   ├── lib/
│   │   ├── components/       # shadcn-svelte wrappers and reusable UI elements
│   │   ├── state/            # Global Svelte 5 runes ($state) for app and job status
│   │   ├── services/         # Frontend wrappers for typed IPC calls
│   │   ├── types/            # Auto-generated TS bindings from tauri-specta
│   │   └── utils/            # Pure TS helpers (formatting, date display)
│   ├── routes/               # Main views (Dashboard, Phone, WiFi, Organizer, Dedup, Settings)
│   ├── app.css               # Tailwind v4 entry and theme variables
│   └── main.ts               # Svelte app bootstrap
├── src-tauri/                # Rust backend
│   ├── src/
│   │   ├── commands/         # Tauri IPC handlers (thin delegation to services)
│   │   ├── services/         # Workflow orchestrators (backup, wifi, organizer, dedup, fileops)
│   │   ├── domain/           # Pure business logic (naming, dates, conflict rules, constants)
│   │   ├── platform/         # OS-specific implementations (linux/mtp, windows, macos)
│   │   ├── storage/          # rusqlite schema, migrations, settings, profiles
│   │   ├── ipc/              # Specta type definitions, IpcError enums
│   │   ├── bindings.rs       # Auto-generated TS bindings (do not edit)
│   │   └── lib.rs            # Tauri plugin registration and setup
│   ├── capabilities/         # Tauri v2 permission scopes
│   ├── Cargo.toml
│   └── tauri.conf.json
├── tests/                    # Rust integration tests and legacy Python test fixtures
├── scripts/                  # CI scripts, binding diff checks, migration helpers
├── package.json
├── vite.config.ts
└── tsconfig.json
```

## §3 Migration matrix

| Legacy Module (AUDIT §A2.1) | New Rust/Svelte Location | Status | Preserve with Reason |
|---|---|---|---|
| `core/constants.py` | `src-tauri/src/domain/constants.rs` | Preserve | Media extension taxonomy (including RAW) and default settings must remain identical to prevent silent skipping or UI drift. |
| `core/cli.py` | Dropped | Drop | AppImage `%U`/`%F` sanitization is specific to the old Linux packaging; Tauri handles OS-level file open events natively. |
| `core/logging_setup.py` | `src-tauri/src/services/logging.rs` | Rewrite | Replace Python `logging` with Rust `tracing` + `tracing-subscriber` for structured, async-safe logging. |
| `core/device.py` | `src-tauri/src/platform/linux/mtp.rs` | Rewrite | Preserve GVFS/GIO discovery logic for Linux; add stubs for Windows/macOS. |
| `core/fileops.py` | `src-tauri/src/services/fileops.rs` | Rewrite | Preserve destination-local temp files and atomic publish; rewrite to enforce `fsync` and add BLAKE2b verification. |
| `core/naming.py` | `src-tauri/src/domain/naming.rs` | Preserve | Date precedence rules, regex patterns, and idempotent prefix stripping must be preserved bit-for-bit to ensure organizer re-runs remain idempotent. |
| `core/storage.py` | `src-tauri/src/storage/` | Rewrite | Preserve tri-state row semantics (`size IS NULL` = trusted); rewrite to add schema versioning, WAL-safe backups, and connection pooling. |
| `core/telemetry.py` | `src-tauri/src/services/telemetry.rs` | Rewrite | Replace queue-based threading with Tauri's async event system; preserve event schema concepts. |
| `core/takeout_engine.py` | `src-tauri/src/services/organizer.rs` | Preserve | Preserve member-by-member ZIP streaming (no `extractall`), sidecar JSON parsing, and destination-inside-input guards. |
| `core/dedup_engine.py` | `src-tauri/src/services/dedup.rs` | Preserve | Preserve size -> quick-hash -> BLAKE2b pipeline, shallowest-copy retention policy, and manifest-before-move invariant. |
| `core/engine.py` | `src-tauri/src/services/backup.rs` | Rewrite | Unify `run_backup` and `run_wifi_backup` into a single pipeline; preserve 25-consecutive-error abort and file-boundary cancellation. |
| `core/wifi_models.py` | `src-tauri/src/ipc/models.rs` | Rewrite | Translate Pydantic models to Rust structs with `serde` and `specta` derives. |
| `core/wifi_telemetry.py` | Merged into `services/wifi.rs` | Drop | Separate telemetry bridge is unnecessary in Rust's async event model. |
| `core/wifi_server.py` | `src-tauri/src/services/wifi.rs` | Rewrite | Replace FastAPI with `axum`; preserve path validation rules; completely rewrite auth/session to meet v2 security model. |
| `views/*` | `src/routes/*` & `src/lib/components/*` | Rewrite | Rebuild UI in Svelte 5 + shadcn-svelte; preserve confirmation gates and empty states. |
| `main.py` | `src-tauri/src/lib.rs` & `src/main.ts` | Rewrite | Replace Flet bootstrap with Tauri window/state setup and Svelte mounting. |

## §4 Typed IPC design

All IPC is defined via `tauri-specta` macros on Rust commands. TS bindings are generated to `src/lib/types/`.

- `get_settings` | `()` -> `Result<AppSettings, IpcError>`
- `update_settings` | `AppSettings` -> `Result<(), IpcError>`
- `get_profiles` | `()` -> `Result<Vec<DeviceProfile>, IpcError>`
- `save_profile` | `DeviceProfile` -> `Result<(), IpcError>`
- `discover_devices` | `()` -> `Result<Vec<MtpDevice>, IpcError>`
- `browse_folders` | `DeviceId` -> `Result<Vec<FolderNode>, IpcError>`
- `start_backup` | `BackupConfig` -> `Result<JobId, IpcError>`
- `cancel_job` | `JobId` -> `Result<(), IpcError>`
- `start_wifi_session` | `WifiConfig` -> `Result<WifiSessionInfo, IpcError>` (Returns token, port, IP)
- `stop_wifi_session` | `()` -> `Result<(), IpcError>`
- `start_organizer` | `OrganizerConfig` -> `Result<JobId, IpcError>`
- `scan_duplicates` | `ScanConfig` -> `Result<JobId, IpcError>`
- `move_duplicates` | `JobId` -> `Result<(), IpcError>`
- `undo_duplicates` | `ManifestPath` -> `Result<JobId, IpcError>`
- `run_migration` | `()` -> `Result<MigrationReport, IpcError>`

*IpcError codes*: `DB_ERROR`, `FS_PERMISSION`, `FS_SPACE`, `INVALID_PATH`, `JOB_CANCELLED`, `WIFI_AUTH_FAILED`, `WIFI_RATE_LIMIT`, `MIGRATION_FAILED`, `DEVICE_DISCONNECTED`, `UNKNOWN`.

*CI Check*: A `package.json` script `npm run specta:check` must exist. It executes `cargo run --bin specta-gen` to regenerate bindings into a temporary directory, then runs `diff -r` against the committed `src/lib/types/`. The CI pipeline fails if any diff is detected, preventing hand-edited bindings.

## §5 SQLite schema v2

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;

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

PRAGMA user_version = 2;
```

**Migration Plan**:
- **Snapshot**: Use `rusqlite::Connection::backup` to produce a WAL-safe snapshot of the legacy DB. NEVER a plain file copy — WAL can hold recent commits outside the .db file, meaning a plain copy would silently omit recent backup history and cause v2 to re-copy or mis-reconcile files.
- **Verify**: Run `PRAGMA integrity_check` on the snapshot. Refuse to proceed if the result is not "ok".
- **Preserve**: Rename the original legacy DB to `state_<id>.db.v1.backup`; never modify or delete it.
- **Import**: Read legacy `settings.json`, `profile_*.json`, and `state_*.json` and insert their contents into the new schema within a single SQLite transaction.
- **Idempotency**: Check `schema_version` table. If `version == 2`, skip migration. If interrupted, the transaction rollback ensures no partial state is written, and re-running will safely resume.
- **Rollback**: If migration fails, delete the new v2 DB, rename `.v1.backup` back to the original filename, and surface a `MIGRATION_FAILED` error to the UI.

**Thread-safety plan**: Use `r2d2_sqlite` connection pool managed in Tauri State.
*Justification*: Tauri commands execute concurrently on a threadpool. A single `Mutex<Connection>` would serialize all DB access, causing UI polling (e.g., job status) to block behind long-running Wi-Fi or backup writes. `r2d2_sqlite` provides safe concurrent read access while respecting SQLite's WAL concurrency model and busy timeouts.

## §6 File-safety invariants

```rust
enum Source {
    Path(PathBuf),
    Stream(Box<dyn Read + Send>), // For USB/MTP streams and Wi-Fi uploads
}

struct Destination {
    final_path: PathBuf,
    explicit_filename: String,
    conflict_mode: ConflictMode, // KeepBoth, Overwrite, Skip
}

// Procedure per file:
// a. Validate source exists/readable, destination parent exists/writable, and disk space > source size + buffer.
// b. Create NamedTempFile in the DESTINATION directory (guarantees same-filesystem rename).
// c. Stream-copy using a bounded buffer (e.g., 1MiB); never read whole file into memory.
// d. flush(); fsync(temp file fd). On Windows, FlushFileBuffers.
// e. Verify size (always); verify content via BLAKE2b hash when mode requires or source is untrusted stream.
// f. Publish atomically:
//    POSIX: NamedTempFile::persist(final). (Note: tempfile crate v0.3+ persist uses renameat2 where available, falling back to rename; it is atomic).
//    Windows: ReplaceFileW (with MOVEFILE_WRITE_THROUGH) or MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH).
//    Why fs::rename/persist alone are insufficient on Windows: Standard rename operations on NTFS/FAT32 do not guarantee directory entry durability on power loss without explicit write-through flags or subsequent directory fsync.
// g. fsync(destination directory) on POSIX. Windows durability strategy: rely on MOVEFILE_WRITE_THROUGH and periodic OS cache flushes, as Windows lacks a direct user-space directory fsync equivalent.
// h. If publish fails: existing destination remains untouched; temp file is dropped/cleaned.
// i. If publish succeeds but durability fails: file is at final path; state = "publish-pending-durability"; caller decides recovery (usually safe to re-verify on next run).
// j. Database update ONLY after durability step completes successfully.

// Cancellation: Checked at file boundaries (before opening source, before publish). Temp file is cleaned up via Drop guard.
// Symlink policy: Refuse to follow untrusted symlinks. Check via std::fs::symlink_metadata; reject if source or destination parent chain contains symlinks outside trusted roots.
// Concurrent jobs: Serialized via a Mutex<HashMap<PathBuf, ()>> tracking active destination directories; no two writers to the same path.
// Crash recovery: On startup, scan destination directories for orphan `.part-*` or temp files older than 1 hour and delete them.
```

## §7 Wi-Fi security model

- **Transport Decision**: HTTP with a scoped, short-lived, single-device session. Justification: Native certificate pinning and TLS are impractical without a dedicated native Android companion app, and generating trusted local certs on desktop triggers OS warnings.
- **Residual Risk**: Bearer tokens and traffic are observable to any device that can monitor the LAN. This is NOT secure against LAN eavesdropping or malicious local network actors.
- **Token**: CSPRNG, ≥128 bits, stored in-memory only, never persisted to disk, never included in URLs or logs. Single-use per session, invalidated on explicit stop, session expiry, or app exit.
- **Session Isolation**: One device per session. Additional devices attempting to connect are rejected until the current session is stopped.
- **Staging**: Per-session staging directory created with `0o700` permissions; completely isolated between sessions and wiped on session end.
- **Filename Sanitization**: Reject `..`, absolute paths, path separators, control characters, and OS-specific reserved names (e.g., `CON`, `NUL` on Windows).
- **Caps & Limits**: Hard request size cap (e.g., 100MB per file), per-session total cap (e.g., 50GB), and strict rate limiting on `/manifest` and `/upload` endpoints.
- **Auth**: Host/Origin headers are checked on every request to prevent DNS rebinding, but the Bearer token is the sole authentication mechanism.
- **Binding**: Binds to a specific LAN interface (e.g., `192.168.1.x`) by default. Binding to `0.0.0.0` requires an explicit user opt-in with a severe warning.
- **UI & Lifecycle**: Visible "Stop Session" control in the UI. On app exit, cancel in-flight uploads, revoke tokens, and securely remove the staging directory.
- **Test Requirements**: Expired session rejected, wrong token rejected, missing token rejected, path traversal rejected, oversized rejected, origin mismatch rejected, rate limit enforced, session isolation verified, cleanup on shutdown verified, token never logged, session reuse rejected.

## §8 Implementation plan (Phases 2-10)

**Phase 2: Foundation and Shell**
- Scope: Tauri 2, Svelte 5, Tailwind v4, shadcn-svelte, Vite, Specta IPC, basic routing, theme system.
- Files: `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `package.json`, `src/main.ts`, `src/app.css`.
- Verification: `npm run dev` starts window; `npm run specta:check` passes.
- DoD: Empty shell renders with dark/light theme toggle; IPC ping command works.
- Must NOT defer: Strict TS config, Specta CI check.

**Phase 3: Storage and Migration**
- Scope: rusqlite setup, WAL-safe legacy DB backup, JSON import, schema versioning.
- Files: `src-tauri/src/storage/`, `src-tauri/src/commands/migration.rs`.
- Verification: `cargo test --test migration` (uses copies / fixtures, never real data).
- DoD: Legacy v1 DB and JSON are safely imported to v2 schema; original files preserved.
- Must NOT defer: Rollback procedure, integrity checks.

**Phase 4: Core File Safety**
- Scope: Atomic publish, fsync enforcement, BLAKE2b verification, conflict modes.
- Files: `src-tauri/src/services/fileops.rs`, `src-tauri/src/domain/conflict.rs`.
- Verification: `cargo test --test fileops` (simulates power loss, ENOSPC).
- DoD: All file-safety invariants (§6) pass automated tests.
- Must NOT defer: Directory fsync, Windows durability strategy.

**Phase 5: Phone Backup (Linux MTP)**
- Scope: GVFS discovery, folder browsing, backup pipeline integration.
- Files: `src-tauri/src/platform/linux/mtp.rs`, `src-tauri/src/services/backup.rs`.
- Verification: Manual test with real Android device; `cargo test` for pipeline logic.
- DoD: Linux MTP backup works with skip/overwrite/keep-both modes.
- Must NOT defer: Destination-inside-source guard.

**Phase 6: Wi-Fi Backup**
- Scope: Axum server, token auth, staging, manifest diff.
- Files: `src-tauri/src/services/wifi.rs`, `src-tauri/src/ipc/models.rs`.
- Verification: `cargo test --test wifi_security` (all §7 test requirements).
- DoD: Secure, authenticated Wi-Fi upload pipeline functional.
- Must NOT defer: Rate limiting, session isolation.

**Phase 7: Google Takeout Organizer**
- Scope: ZIP streaming, sidecar parsing, date precedence.
- Files: `src-tauri/src/services/organizer.rs`, `src-tauri/src/domain/naming.rs`.
- Verification: `cargo test --test organizer` (uses copies / fixtures, never real data).
- DoD: Idempotent Takeout processing matching v1 date rules.
- Must NOT defer: ZIP-slip prevention, streaming memory bounds.

**Phase 8: Duplicate Finder**
- Scope: Quick hash, BLAKE2b, manifest generation, safe move/undo.
- Files: `src-tauri/src/services/dedup.rs`.
- Verification: `cargo test --test dedup`.
- DoD: Preview, move, and undo work without permanent deletion.
- Must NOT defer: Manifest-before-move invariant.

**Phase 9: UI Completion**
- Scope: Connect all Svelte routes to Rust IPC, progress bars, logs, empty states.
- Files: `src/routes/*`, `src/lib/components/*`.
- Verification: Manual UI walkthrough, `npm run build` succeeds.
- DoD: No fake buttons, all jobs show progress and support cancellation.
- Must NOT defer: Accessible keyboard navigation.

**Phase 10: Verification and Release**
- Scope: Packaging (AppImage, MSI, DMG), CI/CD pipelines, final manual QA.
- Files: `.github/workflows/`, `src-tauri/tauri.conf.json`.
- Verification: Packaged apps run on clean VMs.
- DoD: Release artifacts generated and verified.
- Must NOT defer: Cross-platform matrix honesty.

## §9 Testing plan

- **Unit Tests**: Rust `cargo test` for domain logic (naming, dates, conflict resolution) and pure fileops logic.
- **Integration Tests**: Rust tests for DB migrations, Wi-Fi security endpoints, and backup pipelines. *Rule: Every test that touches legacy data uses copies / fixtures, never real data.*
- **Frontend Tests**: Svelte component tests (via Vitest) for state management and IPC mocking.
- **Manual Tests**: Real-device MTP tests, real-phone Wi-Fi tests, and Takeout archive processing on Linux, Windows, and macOS.
- **Per Phase**: Each phase §8 includes specific automated verification gates that must pass before proceeding.

## §10 Platform matrix

- **Linux**: Required and verified. Primary development target. MTP via GVFS, AppImage/deb/rpm packaging.
- **Windows**: Planned, code-ready but not verified until built and run on Windows. MSI/NSIS packaging. MTP via WPD (Windows Portable Devices) API (requires follow-up).
- **macOS**: Planned, code-ready but not verified until built and run on macOS. DMG/app packaging. MTP via Android File Transfer or libmtp (requires follow-up).
- **Android**: Evaluated — likely APK of a companion client only if scope demands native Wi-Fi pinning/TLS, otherwise "not supported in v2.0" (relies on mobile browser).
- **iOS**: Not supported in v2.0.

## §11 Known limitations and explicit "not in v2.0" list

- **EXIF Limits**: HEIC, HEIF, PNG, WebP, and RAW EXIF parsing are NOT supported; falls back to filename/mtime.
- **Wi-Fi Security**: No TLS/HTTPS. Traffic is observable on the LAN.
- **Dedup Moves**: Cross-filesystem duplicate moves are not strictly atomic (copy+delete).
- **Resumable Uploads**: Wi-Fi uploads do not support chunked resumption; interrupted files must be re-uploaded.
- **Not in v2.0**: iOS support, native Android companion app, EXIF write-back, cloud storage integrations, multi-user profiles.

## §12 Rollback and recovery

- **Abandoning Migration**: If the v2 migration fails or the user wishes to revert, the v2 app deletes its new `state_v2.db` and restores the `state_<id>.db.v1.backup` to its original name.
- **Using the Old App**: The legacy Python/Flet AppImage and source code are preserved in the `archive/` directory and remain fully functional, as v2 never modifies the original v1 binaries or active config files during the read-only audit phase.
- **Restoring State**: Because v2 uses `rusqlite::Connection::backup` and never deletes the original v1 DB or legacy JSON files during import, the user can simply delete the v2 configuration directory (`~/.config/SyncUp/v2/` or equivalent) and launch the v1 AppImage to resume exactly where they left off.
