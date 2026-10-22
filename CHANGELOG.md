# Changelog

All notable changes to SyncUp are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
semantic versioning.

## [1.3.0] — 2026-10-22

### Added

- Atomic, size-verified copies for phone backup and organizer operations.
  Temporary files live next to the destination and are renamed with
  `os.replace()` only after verification.
- SQLite backup state with legacy JSON migration. State now records source size
  and modification time, so replaced files are copied again.
- Per-device profiles saved atomically.
- Rotating file log at `~/.config/SyncUp/logs/syncup.log`, plus “open log folder”
  and “clear log” controls.
- Duplicate finder preview, `_manifest.json` (written before moving), and
  “Undo Last Move”.
- Streamed Google Takeout ZIP processing with optional JSON sidecar preservation.
- Idempotent destination naming that strips old SyncUp prefixes.
- Real 256×256 PNG icon, plus SVG source.
- `build.sh` versioned AppImage build script, AppStream metadata, and a cleaned-up
  desktop entry.
- `pyproject.toml`, split runtime/build/dev requirements, architecture docs,
  changelog, CI workflow, and a standard-library `unittest` suite.

### Changed

- Phone backup no longer requires or uses a staging folder; staging caused a
  second full copy and was not crash-safe.
- Duplicate hashing uses BLAKE2b with a size/head/tail pre-check instead of MD5.
- Takeout organizer no longer calls `extractall()`.
- All worker-thread UI updates are marshalled through a single UI queue.
- Theme and window size are persisted.
- Start buttons across all tabs are disabled while any job is running.
- The organizer now reports truthful processed/skipped/error counts.
- Version bumped to `1.3.0`.

### Fixed

- Destination files are no longer unlinked before a replacement copy succeeds.
- Backup state is no longer saved solely at the end, so a cancel or crash keeps
  completed progress.
- Legacy state is migrated instead of silently ignored.
- `.part` temporary files are ignored by scanners.
- The AppImage icon is no longer a 404 text file.
- The AppDir and tracked desktop entries use the correct icon, version metadata,
  and reverse-DNS filename required by AppStream.
- Duplicate finder skips its own `_Duplicates_*` folders on later scans.

## [1.2.1] — 2026-10-22

- Previous release: AppImage packaging, duplicate finder, Takeout organizer.
