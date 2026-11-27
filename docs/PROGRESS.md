# SyncUp V2 — Progress and Release Log

**Release status:** Published
**Version:** 2.0.0
**Branch:** `main`

## Completed implementation

### Foundation and safety

- Tauri 2 desktop application.
- Rust backend and Svelte 5 frontend.
- SQLite persistence.
- Atomic file publication and verification.
- Job tracking and cancellation.

### Phase 5 — Android MTP backup

- Linux GVFS/GIO device discovery and browsing.
- Backup pipeline.
- Physical Android test completed: 610 files copied, zero reported as not copied, with BLAKE2b verification.

### Phase 6 — Wi-Fi backup

- QR-based pairing.
- Local HTTP upload server.
- Staging, verification, and publication.
- Physical phone upload test completed.

### Phase 7 — Google Takeout Organizer

- ZIP streaming and metadata/date handling.
- Tested with an approximately 1 GB Takeout archive from 2018.
- Photos organized by month.

### Phase 8 — Duplicate Finder

- Duplicate scan using size and hashing.
- Quarantine move with manifests.
- Undo and restoration.
- Physical test completed with three identical files; two were moved and verified, then restored.
- Recorded automated verification: 110 passed, 4 ignored.

### Phase 9 — User interface

- Dashboard.
- Phone Backup.
- Wi-Fi Backup.
- Organizer.
- Duplicate Finder.
- Activity.
- Settings.

## Packaging completed

- Linux Debian package built.
- Linux AppImage built.
- AppImage runtime and launch verified.
- Debian package installed and launched.
- Product display name changed to SyncUp V2.
- Legacy SyncUp 1.4.1 desktop entry preserved.
- Final package identity: `sync-up-v2`.
- Application identifier retained: `io.github.lotus-projects-3421.syncup`.

## Release completed

- GitHub Actions run `45983933149` passed.
- Annotated tag `v2.0.0` created and pushed.
- Debian package and AppImage published.
- SHA256 checksums published and verified.
- GitHub release: https://github.com/Lotus-Projects-3421/syncup/releases/tag/v2.0.0
- Default branch: `main`.


## Scope control

Completed feature workflows are not to be reopened without a reproducible defect, data-safety concern, or release-blocking issue. Further feature expansion and cosmetic refinement are outside this release scope.
