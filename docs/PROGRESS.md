# SyncUp v2 — Progress Log

## Current phase: 5 (Phone Backup - Linux MTP) — COMPLETE
## Completed phases: 0 (Audit), 1 (Planning), 2 (Foundation), 3 (Storage), 4 (File Safety Engine and Jobs), 5 (Phone Backup - Linux MTP)
## Last commits: 2308fd5 chore: use pictures/screenshots for mtp hardware test; f97cd7e feat: phase 5 linux mtp phone backup; 19c04df feat: phase 4c jobs engine and ipc bindings

## Pinned toolchain
- rustc: 1.100.0
- cargo: 1.100.0
- node: v24.22.3
- npm: 11.19.1
- gio: 2.80.0

## Handoff for next session
- Paste this section at the top of the next session.
- Current phase: **Phase 5 is COMPLETE** (feature + hardware verification + PROGRESS.md update).
  Do not start Phase 6 until the human explicitly authorizes it.
- What exists now (Phase 4 baseline):
  - `src-tauri/src/services/jobs.rs`: in-memory job engine. `JobId` (uuid v4,
    serialized as a hyphenated string), `JobKind` (Backup/Wifi/Organizer/Dedup/
    Migration), `JobStatus`, `JobProgress` (`fraction()`), `Job`, `JobProgressEvent`,
    `JobHandle` (`update_progress`, `finish`, `cancel_token`), and `JobRegistry`
    (`register`/`list`/`get`/`cancel`).
  - Progress events: `job://progress` carrying `{ jobId, status, progress }`,
    coalesced to <=10 events/sec per job.
  - `src-tauri/src/commands/jobs.rs`: `list_jobs`, `get_job`, `cancel_job`.
  - `src-tauri/src/services/fileops.rs`: safe atomic file-safety engine. Public API:
    `Source` (File/Stream), `Destination` (final_path, conflict_mode,
    expected_size, verify), `Published` (final_path, bytes_written, hash),
    `VerifyMode` (SizeOnly/SizeAndHash), `copy_verified`,
    `copy_verified_with_root(source, dest, cancel, approved_root)`,
    `cleanup_orphan_temp_files`, `resolve_destination_path`,
    `reset_collision_tracker`, `PathLockRegistry`. Re-exports
    `format_collision_name`, `ConflictMode`, `RunCollisionTracker` for
    backward-compat with pre-Phase-5 call sites.
  - `src-tauri/src/domain/conflict.rs`: canonical home of `ConflictMode`
    (KeepBoth/Overwrite/Skip), `format_collision_name`, `RunCollisionTracker`.
    Registered in `lib.rs` as `pub mod domain;` during Phase 5.
- What exists now (Phase 5 additions):
  - `src-tauri/src/platform/mod.rs`, `src-tauri/src/platform/linux/mod.rs`,
    `src-tauri/src/platform/linux/mtp.rs`: GVFS/GIO MTP discovery, folder
    browsing, mount-root resolution. `discover_devices()`,
    `mount_root_for(device_id)`, `browse_folders(device_id)`, `MtpError` enum.
    Discovery enumerates `$XDG_RUNTIME_DIR/gvfs/mtp:host=<serial>` entries and
    supplements with `gio::VolumeMonitor::get().mounts()`. The phone must be
    MOUNTED (via Nautilus or `gio mount mtp://...`); a bare Volume in
    `gio mount -l` without a `Mount(...)` line is not enough.
  - `src-tauri/src/ipc/models.rs`: `DeviceId(pub String)`, `MtpDevice { id,
    display_name, mount_path }`, `FolderNode { name, rel_path, children }`,
    `BackupConfig { device_id, source_rel_paths, destination_root,
    conflict_mode, verify_hash }`. All `#[derive(Serialize, Deserialize, Type)]`
    with `#[serde(rename_all = "camelCase")]`.
  - `src-tauri/src/services/backup.rs`: pipeline orchestrator. `run_backup(
    config, handle, cancel)` (resolves mount via `mount_root_for`) and
    `run_backup_with_root(config, mount_root, handle, cancel)` (test entry).
    Canonicalizes mount + destination, enforces dest-not-inside-source,
    per-file `create_dir_all` + `copy_verified_with_root(source, dest,
    cancel.clone(), approved_root)`, 25-consecutive-error abort,
    `DEVICE_DISCONNECTED` detection when the mount vanishes mid-run,
    cancellation at file boundaries, 100k files-per-job cap. `map_mtp_error`
    converts `MtpError` -> `IpcError`.
  - `src-tauri/src/commands/backup.rs`: `discover_devices()`,
    `browse_folders(device_id)`, `start_backup(config) -> JobId`. Discovery
    and browse wrap their blocking work in `tokio::task::spawn_blocking`.
    `start_backup` validates inputs, registers a `JobKind::Backup` job via
    `state.jobs.register`, spawns the pipeline via `tauri::async_runtime::spawn`,
    finishes with `Completed`/`Cancelled`/`Failed`.
  - `src-tauri/src/ipc/error.rs`: added `DEVICE_DISCONNECTED` const +
    `IpcError::device_disconnected(message)` constructor + assertion in the
    existing `constructors_map_correct_error_codes` test.
  - `src-tauri/src/lib.rs`: added `pub mod domain;`, `pub mod platform;`, and
    registered `commands::backup::{discover_devices, browse_folders,
    start_backup}` in `specta_builder()`.
  - `src-tauri/Cargo.toml`: added `gio = "=0.18.4"` and `glib = "=0.18.5"`
    (pinned to Cargo.lock versions 0.18.4 / 0.18.5; transitive gio-sys 0.18.1
    and glib-sys 0.18.1 unchanged). No other dependency changes.
  - `src-tauri/src/services/fileops.rs`: `#[path = "../domain/conflict.rs"] pub
    mod conflict;` removed and replaced with `pub use crate::domain::conflict::{
    format_collision_name, ConflictMode, RunCollisionTracker };` to eliminate a
    `clippy::duplicate-mod` warning that surfaced when `pub mod domain;` was
    added to lib.rs.
  - `src-tauri/tests/backup_pipeline.rs`: 7 synthetic-fixture integration tests
    (copies all files, dest-inside-source rejection, skip preserves, overwrite
    replaces, keep-both adds ` (N)` suffix, cancel-before-first-file,
    size-only verify). Uses `tempfile::tempdir()` only; no real device; no real
    user data.
  - `src-tauri/tests/mtp_hardware.rs`: 3 `#[ignore]`-gated hardware tests
    (`discover_devices_hardware`, `browse_folders_hardware`,
    `full_backup_hardware`). Source path `Internal shared storage/Pictures/
    Screenshots` (~380 MB). `full_backup_hardware` writes to a fresh
    `tempfile::tempdir()` auto-cleaned on test exit. `walk_file_count` helper
    reports post-copy file count.
  - `src/lib/ipc.ts`: regenerated via `cargo run --bin specta-gen`. New exports:
    `discoverDevices`, `browseFolders`, `startBackup`, `BackupConfig`,
    `ConflictMode` (as TS union `"keepBoth" | "overwrite" | "skip"`), `DeviceId`
    (as `string`), `FolderNode`, `MtpDevice`.
- IpcError code mapping note (recorded per user request): PLAN.md §4 names
  `FS_PERMISSION`, `FS_SPACE`, `JOB_CANCELLED`, `INVALID_PATH`, `DEVICE_DISCONNECTED`.
  The codebase uses `PERMISSION_DENIED`, `DISK_FULL`, `CANCELLED`, `VALIDATION_ERROR`.
  Phase 5 reuses existing codes and adds only `DEVICE_DISCONNECTED`. This is an
  intentional divergence from PLAN.md §4 naming, not a gap.
- Hardware verification status (Phase 5):
  - **Linux MTP: VERIFIED on real device (Realme GT Neo 2 5G).**
  - `discover_devices_hardware`: PASSED — 1 device found,
    id=`realme_realme_GT_Neo2_5G_b20ea0fa`,
    mount=`/run/user/1000/gvfs/mtp:host=realme_realme_GT_Neo2_5G_b20ea0fa`.
  - `browse_folders_hardware`: PASSED — full folder tree returned, including
    `Internal shared storage/Pictures/Screenshots`.
  - `full_backup_hardware`: PASSED — `BACKUP RESULT IS OK: true`,
    `DEST ... FILE COUNT: 618` (~380 MB, 214.47s wall clock).
  - **Windows MTP: NOT implemented in Phase 5.** PLAN.md §8 Phase 5 scope is
    specifically Linux MTP; Windows is a follow-up.
  - **macOS MTP: NOT implemented in Phase 5.** Same.
- Verification status (last full gate: 19:42, on the code now committed at
  f97cd7e): `cargo fmt --check`, `cargo check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`,
  `npm run specta:check` all EXIT=0. Test counts: 25 lib + 7 backup_pipeline +
  16 fileops + 6 migration = 54 passed, 0 failed; 3 mtp_hardware tests
  correctly `ignored` under the normal gate. Cargo.lock's `syncup` dependency
  list includes `gio` and `glib`.
- What does NOT exist yet: `start_wifi_session`, `stop_wifi_session`,
  `start_organizer`, `scan_duplicates`, `move_duplicates`, `undo_duplicates`,
  `run_migration`. Specta event registration (jobs progress events still arrive
  untyped on the frontend). Windows/macOS MTP providers. All UI wiring.
  Phases 6-10.
- Next action: await human approval to begin **Phase 6 (Wi-Fi Backup)**. Do not
  start Phase 6 without explicit go.
- MUST READ FIRST: `docs/PLAN.md` §6 (File-safety invariants), §7 (Wi-Fi
  security), §8 (Phases 5-6), `docs/PROGRESS.md` (this section),
  `src-tauri/src/services/backup.rs`, `src-tauri/src/platform/linux/mtp.rs`,
  `src-tauri/src/services/fileops.rs`, `src-tauri/src/ipc/error.rs`,
  `src-tauri/src/services/jobs.rs`, `src-tauri/tests/mtp_hardware.rs`.
- Do not commit; the human controls Git.
