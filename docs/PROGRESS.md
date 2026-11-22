# SyncUp v2 — Progress Log

## Current phase: 4 (File Safety Engine and Jobs)
## Completed phases: 0 (Audit), 1 (Planning), 2 (Foundation), 3 (Storage)
## Last commit: chore: update generated ipc bindings for phase 3

## Pinned toolchain
- rustc: 1.100.0
- cargo: 1.100.0
- node: v24.22.3
- npm: 11.19.1
- gio: 2.80.0

## Handoff for next session
- Paste this section at the top of the next OpenCode session.
- Current phase: Phase 4 is COMPLETE (4a + 4b + 4c; 4d was subsumed into 4b).
  Do not start Phase 5 until instructed.
- What exists now:
  - `src-tauri/src/services/jobs.rs`: in-memory job engine. `JobId` (uuid v4,
    serialized as a hyphenated string), `JobKind` (Backup/Wifi/Organizer/Dedup/
    Migration, reserved for PLAN.md §8 Phases 5-8 and the §4 `run_migration`
    command), `JobStatus`, `JobProgress` (`fraction()`), `Job`, `JobProgressEvent`,
    `JobHandle` (`update_progress`, `finish`, `cancel_token`), and `JobRegistry`
    (`register`/`list`/`get`/`cancel`).
  - Progress events: `job://progress` carrying `{ jobId, status, progress }`,
    coalesced to <=10 events/sec per job. Pending->Running and terminal
    transitions always emit. The Specta builder does not enumerate events yet,
    so the frontend receives these payloads untyped until a later phase.
  - `src-tauri/src/commands/jobs.rs`: `list_jobs`, `get_job`, `cancel_job`.
    Unknown ids map to `VALIDATION_ERROR` with the id in the message;
    `src-tauri/src/ipc/error.rs` is unchanged.
  - `AppState.jobs: JobRegistry`, constructed in the `lib.rs` setup block with
    the app handle so events can be emitted.
  - `src/lib/ipc.ts` regenerated: `listJobs`, `getJob`, `cancelJob`, `Job`,
    `JobKind`, `JobProgress`, `JobStatus` (plus the phase-split aliases Specta
    emits for structs with optional fields).
  - Jobs are memory-only: every record disappears on app exit. `cancel_job`
    only cancels the token; the owning workflow must observe it and call
    `finish(JobStatus::Cancelled)`.
- Verification status (last run): `cargo fmt --check`, `cargo check`,
  `cargo clippy -- -D warnings`, and `cargo test --no-fail-fast` all pass.
  Totals: 25 lib tests (8 new jobs tests) + 16 fileops integration tests + 6
  migration tests = 47 passed, 0 failed. The fileops suite is still 16
  Linux-compiled tests plus 1 Windows-gated test. Unused-dependency grep = 0.
  No `unwrap()/expect()/panic!` and no `allow(dead_code)` in the jobs modules.
- What does NOT exist yet: workflow commands (`start_backup`,
  `start_wifi_session`, `stop_wifi_session`, `start_organizer`,
  `scan_duplicates`, `move_duplicates`, `undo_duplicates`, `discover_devices`,
  `browse_folders`, `run_migration`), Specta event registration, and all UI
  wiring.
- Next action: await human approval to commit Phase 4c; Phase 5 then begins in
  a fresh session.
- MUST READ FIRST: `docs/PLAN.md` §6 (File-safety invariants) and §8 (Phase 4/5),
  `docs/PROGRESS.md` (this section), `src-tauri/src/services/jobs.rs`,
  `src-tauri/src/services/fileops.rs`, `src-tauri/src/ipc/error.rs`,
  `src-tauri/src/state.rs`, and `src-tauri/tests/fileops.rs`.
- Do not commit; the human controls Git.
