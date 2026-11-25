# SyncUp v2 — Release Status

**Version:** 2.0.0-pre
**Date:** 2026-11-26
**Target:** Linux Desktop

This document tracks the honest readiness of SyncUp v2 for release. It distinguishes between code existence, automated verification, and real-world validation.

## Feature Matrix

| Feature | Status | Automated Tests | Manual Validation | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **App Shell / Nav** | ✅ Implemented | ✅ `npm run check` | ⚠️ Basic | Svelte 5, Tailwind, Routing. |
| **File Safety Engine** | ✅ Implemented | ✅ `fileops.rs` | N/A | Atomic publish, verify, fsync. Core logic tested. |
| **Storage / Migration** | ✅ Implemented | ✅ `migration.rs` | ⚠️ Synthetic | WAL-safe snapshot, JSON import. Tested with fixtures. |
| **Phone Backup (MTP)** | ✅ Implemented | ⚠️ Pipeline only | ❌ **Required** | Linux GVFS. Needs real Android phone test. |
| **Wi-Fi Backup** | ✅ Implemented | ✅ `wifi_security.rs` | ❌ **Required** | HTTP/LAN. 38 tests pass. Needs real phone upload test. |
| **Takeout Organizer** | ✅ Implemented | ⚠️ Unit only | ❌ **Required** | ZIP streaming, date logic. Needs real Takeout ZIP test. |
| **Duplicate Finder** | ✅ Implemented | ❌ **EMPTY** | ❌ **Required** | **BLOCKER:** 21 empty test bodies. Logic exists but unverified by automation. |
| **Dashboard** | ✅ Implemented | ✅ `npm run check` | ⚠️ Basic | Health, recent jobs. |
| **Settings** | ✅ Implemented | ✅ `npm run check` | ⚠️ Basic | Theme, window size, legacy import UI. |
| **Packaging** | ⚠️ Configured | N/A | ❌ **Required** | `tauri.conf.json` set. No verified AppImage/deb run yet. |
| **CI/CD** | ❌ **Outdated** | N/A | N/A | Runs Python v1 tests. Needs update for Rust/Node. |

## Release Blockers

1.  **Dedup Test Coverage:** `src-tauri/tests/dedup.rs` has 21 empty test functions. We cannot claim safety for a destructive (move) workflow without automated verification.
2.  **Real-Device Validation:** MTP and Wi-Fi flows have never been tested with a physical Android phone in this V2 build.
3.  **Packaging Verification:** No clean-install test of a built artifact (AppImage/deb).
4.  **CI Configuration:** CI does not run V2 checks.
5.  **Documentation:** README/ARCHITECTURE currently describe the wrong technology stack (Python).

## Known Limitations (V2.0)

*   **Linux Only:** Windows/macOS builds are not produced or supported.
*   **Wi-Fi Security:** HTTP only. No TLS. Vulnerable to LAN snooping. Use on trusted networks only.
*   **No Resumable Uploads:** Interrupted Wi-Fi uploads must restart from zero.
*   **EXIF Support:** Limited to JPEG/TIFF. HEIC/WebP/RAW fall back to filename/mtime.
*   **Cross-FS Dedup:** Moves across filesystems are copy+delete, not strictly atomic.

## Deferred / Out of Scope

*   iOS Support
*   Native Android Companion App (uses mobile browser)
*   Cloud Storage Integration
*   EXIF Write-back
*   HTTPS for Wi-Fi (requires native app for pinning)

## Sign-off Criteria

- [ ] `cargo test` passes (including real `dedup` tests).
- [ ] `npm run check` && `npm run build` passes.
- [ ] Real Android MTP backup successful.
- [ ] Real Android Wi-Fi backup successful.
- [ ] Real Takeout ZIP organization successful.
- [ ] Duplicate scan/move/undo successful on real files.
- [ ] AppImage/deb installs and runs on clean Linux profile.
- [ ] README/ARCHITECTURE accurately describe V2.
- [ ] CI runs V2 checks.
