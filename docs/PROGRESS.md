# SyncUp v2 — Progress Log

**Current Phase:** 9 (UI Completion) — **Slice G Complete**
**Branch:** `v2-migration`
**HEAD:** `b3a69e5` (feat: phase 9 slice g - dashboard with health, recent activity and quick actions)

## Pinned Toolchain
- rustc: 1.100.0
- cargo: 1.100.0
- node: v24.22.3
- npm: 11.19.1
- gio: 2.80.0

## Completed Work

### Phase 0-4: Foundation
- Audit, Planning, Tauri 2 Scaffold, Storage (SQLite/WAL), File Safety Engine (Atomic Publish/Verify).

### Phase 5: Phone Backup (Linux MTP)
- GVFS/GIO discovery, folder browsing, backup pipeline.

### Phase 6: Wi-Fi Backup Backend
- Axum server, token auth, staging, manifest diff, atomic publish integration.
- **Tests:** 38 passing (unit + security + finalization).

### Phase 7: Takeout Organizer
- ZIP streaming, sidecar parsing, date precedence, idempotent naming.

### Phase 8: Duplicate Finder Backend
- Scan (size -> quick hash -> BLAKE2b), Move (manifest-based), Undo, Reconciliation.
- **CRITICAL DEBT:** `src-tauri/tests/dedup.rs` contains **21 empty test bodies**. These functions exist but perform no assertions. **Zero automated coverage** for dedup logic exists yet. This is a release blocker.

### Phase 9: UI Completion
- **Slice A (Foundation/Activity):** `6da01ba`
- **Slice B (Settings/Migration):** `78987a0`
- **Slice C (Organizer UI):** `f05aa2e`
- **Slice D (Phone Backup UI):** `c075f6f`
- **Slice E (Wi-Fi Backend Fix):** `d3b61e2` (Shutdown token separation).
- **Slice E (Wi-Fi UI):** `642eaf0` (QR, progress, stop, reattach).
- **Slice F0 (Dedup Backend Fix):** `b8f6381` (Job error handling).
- **Slice F1 (Dedup Scan UI):** `3e0af48` (Indeterminate progress, preview).
- **Slice F2 (Dedup Move UI):** `738440a` (Confirmation, manifest attribution).
- **Slice F3 (Dedup Undo UI):** `bd95f9a` + `8bc4fbf` (Manifest history, distinct messaging).
- **Slice G (Dashboard):** `b3a69e5` (Health, recent activity, quick actions).

## Current Status

- **All 7 UI routes implemented:** Dashboard, Phone Backup, Wi-Fi Backup, Organizer, Duplicates, Activity, Settings.
- **Backend complete:** All core workflows (Backup, Wi-Fi, Organizer, Dedup) have Rust implementations.
- **Documentation:** Stale (README/ARCHITECTURE still describe Python v1). **Action:** Rewrite for V2.
- **CI:** `.github/workflows/ci.yml` runs **Python** tests only. **Action:** Update for Rust/Node.

## Known Technical Debt

1.  **Empty Dedup Tests:** 21 empty bodies in `tests/dedup.rs`. Must be implemented before release.
2.  **Stale Docs:** README and ARCHITECTURE describe V1.
3.  **CI Workflow:** Runs V1 Python tests, not V2 Rust/Node checks.
4.  **Manual Testing:** Real-device MTP and Wi-Fi flows not yet validated on hardware.
5.  **Packaging:** No verified release build (AppImage/deb) tested on clean system.

## Next Actions

1.  **Documentation Reconciliation:** Rewrite README/ARCHITECTURE/PROGRESS for V2. (In Progress)
2.  **Dedup Tests:** Implement real bodies for the 21 empty tests.
3.  **CI Update:** Configure GitHub Actions for `cargo test` and `npm run check`.
4.  **Manual Validation:** Real phone MTP/Wi-Fi tests.
5.  **Packaging:** Build and test AppImage/deb.
