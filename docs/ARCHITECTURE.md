# SyncUp Architecture

This document describes the design decisions behind SyncUp 1.3. It is intended
for future maintenance, not as end-user documentation.

## Module map

| Module | Responsibility |
| --- | --- |
| `main.py` | Flet app shell, UI queue, progress queue, theme/window persistence, rotating log |
| `core/constants.py` | Version, supported media extensions, defaults, duplicate-folder prefix |
| `core/fileops.py` | Atomic copies, streamed copies, size verification, unique names, atomic JSON |
| `core/naming.py` | Date/time parsing, generated-prefix stripping, destination paths |
| `core/storage.py` | Atomic settings/profiles, SQLite state, legacy JSON migration |
| `core/device.py` | Linux `gvfs` MTP discovery and folder listing |
| `core/engine.py` | Phone backup orchestration and per-file error handling |
| `core/takeout_engine.py` | Loose-file and streamed-ZIP organizer |
| `core/dedup_engine.py` | Duplicate detection, movement, manifest, undo |
| `core/logging_setup.py` | Rotating file logger |
| `views/common.py` | Shared cards, folder opening, job-button lifecycle |
| `views/phone_view.py` | Phone Backup UI |
| `views/organizer_view.py` | Special Organizer UI |
| `views/dedup_view.py` | Duplicate Finder UI |

## Non-negotiable safety invariants

1. **Never unlink a destination before the replacement is complete.**
   All copies are written to a temporary file in the *destination directory* and
   atomically renamed with `os.replace()`.
2. **Verify before publish.**
   `copy_file_atomic` compares the temporary file size with the source size.
   `copy_stream_atomic` compares the number of bytes written with the ZIP member
   size. A mismatch raises and the destination is untouched.
3. **Commit state only after a successful copy.**
   The SQLite state is updated after the atomic rename. Cancellation commits the
   state accumulated so far.
4. **Duplicate operations are reversible.**
   The duplicate finder only moves files into `_Duplicates_<timestamp>/`, writes
   `_manifest.json`, and exposes undo.
5. **No UI control is touched from a worker thread.**
   Workers put callables into a `queue.Queue`; `main.py` drains it on the Flet
   event loop.

### Atomic copy flow

```text
source file/stream
   |
   v
.destination.name.part-<pid>-<random>   (same directory as destination)
   |
   +-- copy bytes
   +-- verify size
   +-- preserve mtime
   |
   v
os.replace(temp, destination)            (atomic same-filesystem rename)
```

If anything fails, the temporary file is removed and the existing destination is
left in place. Temporary files are dot-files and are ignored by scanners.

## Naming and idempotency

`core/naming.py` parses dates such as:

```text
2024-01-01
20240101
2024_01_01
2024-01-01_12-34-56
IMG_20240101_123456
Screenshot_2024-01-01 12.34.56
```

A generated destination name is:

```text
YYYY-MM-DD_HH-MM-SS_<original-stem><lowercase-extension>
```

Before generating a name, any existing generated prefix is stripped. This makes
reprocessing idempotent:

```text
2024-01-01_12-34-56_IMG_20240101_123456.jpg
    -> strip prefix -> IMG_20240101_123456
    -> same date    -> 2024-01-01_12-34-56_IMG_20240101_123456.jpg
```

If an old prefix has `00-00-00` but the original stem contains a real time on the
same date, the real time is preferred.

Sort modes:

- `year`: `<dest>/2024/<name>`
- `month`: `<dest>/2024/2024-05/<name>`
- `day`: `<dest>/2024/2024-05-06/<name>`
- `flat`: `<dest>/<name>`

## Phone backup state

SQLite database: `~/.config/SyncUp/state_<safe-device-id>.db`

```sql
CREATE TABLE processed (
    rel_path  TEXT PRIMARY KEY,
    size      INTEGER,
    mtime     INTEGER,
    copied_at TEXT
);
```

`rel_path` is relative to the phone storage root. `size`/`mtime` are the source
values at the time of the verified copy.

Skip rules:

- No row: copy.
- Row with `size IS NULL`: legacy row from the old JSON state; trust it (avoids
  re-copying an existing library after upgrade).
- Row with matching size and mtime (within 2 seconds): skip.
- Row with different size or mtime: copy again.

The old `state_<id>.json` file is imported with `INSERT OR IGNORE` when the
database is first opened. The JSON file is intentionally **not deleted**.

## Takeout / organizer flow

1. Collect loose media and `.zip` paths from the selected inputs.
2. Open each ZIP central directory once to count matching members.
3. For each loose file:
   - read sidecar date if present,
   - else parse the filename,
   - else use the file mtime,
   - resolve conflicts,
   - atomic copy,
   - optionally copy the sidecar next to the target.
4. For each ZIP:
   - keep the archive open,
   - stream one member at a time with `ZipFile.open`,
   - read the sidecar member for the date,
   - atomic copy directly to the destination,
   - optionally copy the sidecar member.
5. Return counts and per-file error details.

No `extractall()` and no intermediate extraction directory are used. This avoids
doubling disk usage and lets progress advance during a large archive.

## Duplicate finder flow

```text
walk files
  -> group by size
  -> quick hash (size + first/last 64 KiB) inside size groups
  -> full BLAKE2b hash inside quick groups
  -> sort each duplicate group by (path depth, mtime, path)
  -> keep the first; mark the rest as duplicates
  -> preview: return counts/paths only
  -> move: write _manifest.json first, then shutil.move into
           _Duplicates_<timestamp>/<relative-path>
```

The manifest is written *before* the first move. If the process is interrupted,
undo can still read the planned entries and restore whichever files actually
moved; missing moved files are counted as skipped.

Undo reads the newest manifest under the scan root and moves files back when the
original path is still free. If the original path already exists, the file is
left in place and counted as skipped.

## Error model

Engines never raise for ordinary per-file problems. They return:

```python
{
    "total": int,
    "processed": int,
    "skipped": int,
    "errors": int,
    "error_details": [str, ...],
    "cancelled": bool,
    "time": "1m 23s",
}
```

A fatal problem (missing phone, no free space, invalid destination) adds an
`"error"` key with a human-readable message. The UI logs both fatal errors and
per-file details, then shows a summary.

## Threading model

- All long operations run in `page.run_thread(...)`.
- Worker code may call:
  - `progress_callback(current, total, message)` — puts an item in
    `progress_queue`.
  - `error_callback(name, message)` — calls the UI `log()` wrapper.
  - `post_ui(callback)` — puts a callable in `ui_queue`.
- `main.py` runs two async consumers:
  - `ui_consumer` executes queued callables and calls `page.update()` in batches.
  - `progress_consumer` drains the progress queue, keeps only the latest item,
    and updates the progress text.
- `state["cancel_event"]` is a `threading.Event`; `on_close` sets it together
  with `shutdown_event`.

## Packaging

- `assets/syncup.png` is generated by `scripts/make_icon.py` using only the
  standard library (`zlib` + `struct`).
- `packaging/io.github.lotus_projects_3421.SyncUp.desktop` is the tracked
  desktop-entry template. `build.sh` substitutes `@VERSION@`.
- `SyncUp.spec` collects Flet data, includes the hidden `flet_desktop` launcher,
  and bundles `assets/syncup.png`.
- `build.sh` copies the cached Flet desktop runtime into
  `AppDir/usr/lib/syncup/flet` and generates an `AppRun` wrapper that sets
  `FLET_VIEW_PATH`, avoiding a first-run download when the runtime is available.
- The desktop entry is
  `packaging/io.github.lotus_projects_3421.SyncUp.desktop`; its reverse-DNS
  filename matches the AppStream `<id>` and the `.appdata.xml` filename.
- `AppStream` metadata lives in
  `packaging/io.github.lotus_projects_3421.SyncUp.appdata.xml`; the desktop
  entry and hicolor icon are installed under `AppDir/usr/share` as well as at
  the AppDir root.
- `AppDir/`, `build/`, and `dist/` are generated and gitignored.

## Compatibility notes

- Flet 1.0.4 uses `ft.Button(content=...)` and `ft.Colors.WHITE_70`-style names.
- No code path unlinks a media source; the duplicate finder moves files aside
  instead of deleting them.
- Windows/macOS are not yet implemented; `core/device.py` is Linux/MTP-specific.
- The organizer does not rewrite EXIF; it preserves the original media bytes and
  copies Google’s JSON sidecars when requested.
