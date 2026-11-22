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
- Current phase: 4b (File Safety Engine). Phase 4 is NOT complete; do not start 4c until instructed.
- What exists now:
  - `src-tauri/src/services/fileops.rs`: atomic, verified `copy_verified` engine with
    `Source`, `VerifyMode`, `Destination`, `Published`, `FileOpError`, and
    `impl From<FileOpError> for IpcError`.
  - Same-filesystem `tempfile::NamedTempFile` staging, 64 KiB streamed copy with
    cancellation checks, size/BLAKE2b verification, atomic publish, POSIX directory
    fsync (Windows uses `ReplaceFileW` / `MoveFileExW` with write-through), and
    `PublishPendingDurability` on directory-flush failure.
  - `PathLockRegistry` (exposed via `FileOpsState` in `state.rs`) and
    `cleanup_orphan_temp_files`.
  - Deterministic same-run `KeepBoth` collision handling (reuses `domain/conflict.rs`).
  - Tests: 5 inline `fileops` unit tests plus 16 Linux-compiled integration tests in
    `src-tauri/tests/fileops.rs` (the 17th, `windows_durability_compiles_only_on_windows`,
    is Windows-only and therefore not compiled on Linux). Global fault-injection tests are
    serialized by a test-only async mutex so hooks cannot leak between parallel tests.
- Verification status (last run): `cargo fmt --check`, `cargo check`,
  `cargo clippy -- -D warnings`, and `cargo test --no-fail-fast` all pass.
  Test totals: 17 lib tests + 16 fileops integration tests + 6 migration tests = 39 passed,
  0 failed. Unused-dependency grep = 0. No `unwrap()/expect()/panic!` in production
  `fileops.rs`; all matches are inside integration-test code.
- What does NOT exist yet: job manager/`jobs.rs`, job commands, Specta/IPC bindings,
  and UI wiring for file operations.
- Next action: await explicit approval, then begin 4c (jobs + commands + bindings).
- MUST READ FIRST: `docs/PLAN.md` §6 (File-safety invariants) and §8 (Phase 4),
  `docs/PROGRESS.md` (this section), `src-tauri/src/services/fileops.rs`,
  `src-tauri/src/domain/conflict.rs`, `src-tauri/src/ipc/error.rs`,
  `src-tauri/src/state.rs`, and `src-tauri/tests/fileops.rs`.
- Do not commit yet; the human controls Git. Do not mark Phase 4 complete.
