# SyncUp v1.4.1 — Project Audit

> Generated during Phase 0. Appended in chunks A1–A4. Do not edit manually.
## A1.1 Repository state

- Branch: `v2-migration`
- Working tree: clean
- HEAD: `9622df3 test: add sanitized legacy fixture folder`
- Ahead of `main@55bc4a8` by: 9622df3, 73937d1, 7443890 (three new commits; 55bc4a8 shared)
- Branches:
  - `main` → 55bc4a8 (tag `v1.4.1`)
  - `v2-migration` → 9622df3 (current)
  - `backup/before-major-upgrade` → 55bc4a8 (local + `origin/backup/before-major-upgrade`)
  - `origin/main` → 55bc4a8
- Repo root contents (depth 1):
  - Directories: `AppDir/`, `archive/`, `assets/`, `build/`, `core/`, `dist/`, `docs/`, `packaging/`, `tests/`, `views/`, `.venv/`, `.ruff_cache/`, `.github/`, `__pycache__/`
  - Files: `appimagetool`, `build.sh`, `CHANGELOG.md`, `.gitignore`, `LICENSE`, `main.py`, `pyproject.toml`, `README.md`, `requirements.txt`, `requirements-build.txt`, `requirements-dev.txt`, `SyncUp.spec`
- Depth-2 tree — notable structure:
  - `core/`: 15 modules (`cli`, `constants`, `dedup_engine`, `device`, `engine`, `fileops`, `__init__`, `logging_setup`, `naming`, `storage`, `takeout_engine`, `telemetry`, `wifi_models`, `wifi_server`, `wifi_telemetry`)
  - `views/`: 6 modules (`common`, `dedup_view`, `__init__`, `organizer_view`, `phone_view`, `wifi_view`)
  - `tests/`: 11 `test_*.py` files + `fixtures/` + `__init__.py`
  - `docs/`: `ARCHITECTURE.md`, `AUDIT.md`, `PLAN.md`, `PROGRESS.md`, `SPEC.md`
  - `packaging/`: `io.github.lotus_projects_3421.SyncUp.appdata.xml`, `io.github.lotus_projects_3421.SyncUp.desktop`
  - `assets/`: `banner.png`, `social-preview.jpg`, `syncup.png`
  -  **Archive**: `archive/` holds historical AppImages v1.0 to v1.4.1 plus `github-recovery-codes.txt` (206 bytes, do not read), `SyncUp till v1.3.0 project.txt`, and `VIRTUAL environmentr start.txt`.
- Anomaly: `archive/github-recovery-codes.txt` present in working tree. Contents NOT read. **Not tracked** by git (`git ls-files archive/` empty). Covered by `.gitignore` (`archive/`). Treated as sensitive; never executed, never opened.

## A1.2 Entry point

- File: `main.py` (16,159 bytes, 516 lines)
- Framework: Flet (`import flet as ft`)
- Dispatch: `if hasattr(ft, "run"): ft.run(main) else: ft.app(target=main)` — supports both older and newer Flet entry APIs.
- Startup sequence in `main(page)`:
  1. Top-level `sys.argv[:] = sanitize_argv(sys.argv)` strips AppImage `%U`/`%F` field codes **before** Flet or any folder-open handler sees them (`core.cli.sanitize_argv`).
  2. `setup_logging()` → `(logger, log_dir)`.
  3. `load_global_settings()` → dict of persisted preferences.
  4. `page.title`, `theme_mode` (from `settings["theme"]`), `theme`/`dark_theme` seed `BLUE`, window size/icon (best-effort, wrapped in try/except for Flet-version variance).
  5. Creates shared `state` dict: `is_running`, `shutdown_event`, `cancel_event`, `start_buttons`, `cancel_buttons`.
  6. Creates two `queue.Queue`: `progress_queue` (worker → UI progress), `ui_queue` (worker → UI callbacks via `post_ui`).
  7. Builds progress UI: `phase_text`, `progress_ring`, `progress_bar`, `progress_text`, plus `log_col` capped at **800 entries** (oldest dropped), colour-coded, auto-scroll.
  8. Theme toggle persists `settings["theme"]` via `save_global_settings` on click; window geometry persisted on `page.on_close`.
  9. Builds 4 tab views: `build_phone_view`, `build_organizer_view`, `build_dedup_view`, `build_wifi_view`. Every `build_*_view` receives the same 7-tuple: `(page, state, log, file_picker, progress_queue, play_ding, post_ui)`.
 10. Layout: header (app name + `v{VERSION}` + theme button) above a `Row` of (`Tabs` with 4 tabs) + (`report_card` with progress + log).
 11. Starts two asyncio tasks via `page.run_task`: `ui_consumer()` (drains `ui_queue`, calls `page.update()`) and `progress_consumer()` (drains `progress_queue`, coalesces to "latest progress wins").

- Non-trivial behaviours the v2 app **must preserve** (behavioural oracle):
  - **AppImage field-code stripping** before any argument consumer.
  - **Cross-platform completion sound** via `_play_ding`: `winsound.Beep` on Windows, `afplay` on macOS, `canberra-gtk-play` on Linux — run in a daemon thread so Flet UI never blocks. (Code branches exist for all 3 OSes; only Linux was validated for v1.4.1.)
  - **Two-queue threading model**: worker threads never touch the Flet page; they push callbacks via `post_ui` / progress via `progress_queue`. Honours `shutdown_event`.
  - **Telemetry tolerance**: `_unpack_progress` accepts both 3-tuple `(current, total, message)` and 4-tuple `(current, total, message, event_dict)`.
  - **Latest-progress-wins coalescing** in `progress_consumer`, with an explicit **flush-latest-progress-before-control-event** step so a `kind="summary"`/"done" can never be overwritten by a stale in-flight progress item.
  - **Log column capped at 800 rows**, auto-scrolled.
  - **Persisted preferences**: theme, window width/height.
  - **Graceful shutdown**: `page.on_close` sets `shutdown_event` and `cancel_event`, then saves window geometry best-effort.
  - **Optional sounds and optional log-folder button must not crash the UI if the OS binary (`canberra-gtk-play`, `afplay`) is missing** — all wrapped in try/except.

## A1.3 Dependencies

Declared in `pyproject.toml` `[project].dependencies` and mirrored verbatim in `requirements.txt`. Build-only (`requirements-build.txt`) and dev-only (`requirements-dev.txt`) layered via `-r`.

| Dependency | Declared constraint | Class | Purpose |
|---|---|---|---|
| `flet[desktop]` | `==1.0.4` (exact pin) | UI | Flet desktop UI framework + bundled runtime |
| `fastapi` | `>=0.115,<1` (range) | HTTP | Wi-Fi upload server framework |
| `uvicorn` | `>=0.30,<1` (range) | HTTP | ASGI server for FastAPI |
| `pydantic` | `>=2.7,<3` (range) | HTTP | Request/response model validation |
| `python-multipart` | `>=0.0.9` (min) | HTTP | Multipart upload parsing |
| `qrcode` | `>=7.4,<9` (range) | QR | Wi-Fi pairing QR generation |
| `pyinstaller` (build) | `==6.22.3` (exact pin) | packaging | AppImage build binary bundling |
| `ruff` (dev) | `>=0.6` (min) | other | Linter (dev only) |

- Runtime dependencies: 6 pip packages, all range-pinned except `flet` (exact `==1.0.4`).
- Build-only: 1 pip package (`pyinstaller`, exact).
- Dev-only: 1 pip package (`ruff`).
- **DB**: no external DB package. Persistence uses Python stdlib `sqlite3`. The v2 migration will replace this with `rusqlite` (per STANDING RULES: use `Connection::backup` or `VACUUM INTO`, never a plain file copy; handle WAL).
- **EXIF**: none. No `Pillow`, `piexif`, `exifread`. README confirms: "does not rewrite EXIF timestamps." Date extraction uses Takeout sidecar JSON, filename patterns, or filesystem mtime only.
- **image-metadata**: none. Sizes are read via `os.stat`; images are not decoded by the app.
- **QR**: `qrcode` package only.
- **Stdlib load-bearing**: `sqlite3` (DB), `hashlib` (BLAKE2b dedup), `queue` + `threading` (worker/UI bridge), `asyncio` (Flet tasks), `subprocess` (sound + folder-open), `pathlib`, `os`, `datetime`.
- **System-provided (not pip)**: `gio`/`gvfs` (MTP device access — hard dependency), `canberra-gtk-play` (Linux completion sound — optional, wrapped in try/except).
- **Not present** (valuable negatives): no EXIF library, no image-decoding library, no external DB library, no external HTTP client (server-only).

- Version agreement: `core/constants.py: VERSION = "1.4.1"`, `pyproject.toml: version = "1.4.1"`, tag `v1.4.1` — consistent.
- Python requirement: `>=3.10`; classifiers list 3.10, 3.11, 3.12.
- Packaging targets: `py-modules = ["main"]`, `packages.find include = ["core*", "views*"]`.
- `.gitignore` correctly excludes: `__pycache__/`, `*.py[cod]`, `.venv/`, `.pytest_cache/`, `.ruff_cache/`, `build/`, `dist/`, `*.AppImage`, `AppDir/`, `*.spec.bak`, `*.db`, `*.db-wal`, `*.db-shm`, `*.db.corrupt-*`, `*.part-*`, `archive/`, `appimagetool`, `profiles-data/`, and personal note files.
- Tracked file count: **56**. `git ls-files archive/` → empty (archive not tracked). `git ls-files __pycache__ build dist .venv .ruff_cache` → empty (all untracked). `archive/github-recovery-codes.txt` exists on disk but is **not tracked** and is covered by `.gitignore`; contents not read, not printed, not executed.

## A1.4 Claimed features (README/CHANGELOG)

### README.md (v1.4.1 repo, text describes v1.4.0 release)

**Safety rule stated verbatim:** "never replace or remove a destination file until the new copy has been written and verified."

**Claimed v1.4.0 features:**
- Wi-Fi Backup tab (send from phone over LAN, QR-code pairing)
- Smart Mobile UI (zero-install web page on phone, folder selection, progress)
- Crash-Safe Sync Engine (uploads through the atomic + verified copy pipeline)
- Automatic State Management (SQLite-driven diff; only missing/new files requested)
- Atomic and crash-safe copies (`.part-*` next to destination, verified, `fsync`, `os.replace()`)
- SQLite backup state (replaces whole-file JSON state; small transactions)
- Streamed Takeout ZIPs (member-by-member, no `extractall()`)
- Duplicate Finder (preview, manifest-before-move, undo)
- "105 unit tests" using stdlib `unittest`
- Professional packaging (AppStream metadata, reverse-DNS desktop entry, hicolor icon, bundled Flet runtime)

**Claimed workflows:**
- Phone Backup (MTP) — gvfs/gio discovery, browse + multi-folder select, remembers destination/sort/filter/conflict/folders per device, changed-file detection by path+size+mtime, dedupe of overlapping folders, free-space check, stop-after-repeated-errors.
- Wi-Fi Backup — local HTTP server + QR, mobile page, missing-only upload, same crash-safe pipeline.
- Special Organizer — streamed Takeout ZIPs, sidecar (`photoTakenTime`/`creationTime`), `.json` and `.supplemental-metadata.json`, day/month/year/flat layouts.
- Duplicate Finder — size + head/tail hash pre-check → full BLAKE2b, preview, keeps shallowest copy, moves to `_Duplicates_/`, writes `_manifest.json` before first move, Undo Last Move.

**Claimed data locations** (never to be touched in development): `~/.config/SyncUp/settings.json`, `profile_<id>.json`, `state_<id>.db`, `state_<id>.json` (legacy, migration only), plus `~/.config/SyncUp/logs/syncup.log`.

**Claimed limitations** (verbatim excerpts):
- "MTP support is Linux-specific and depends on `gvfs`."
- "Wi-Fi backup requires the phone and PC to be on the same local network. **No authentication or TLS is used; the server is trusted on the LAN.**"
- "**Windows and macOS builds are not currently produced or tested.**"
- "The organizer... **does not rewrite EXIF timestamps.**"
- "Overwrite uses an atomic replacement, but the previous file is gone after a successful replacement."
- "Duplicate cleanup is a reversible move operation, not a permanent deletion tool."

**Claimed CI:** `.github/workflows/ci.yml` runs "compilation and the full unittest suite on Python 3.10 and 3.12, plus Ruff linting."

### CHANGELOG.md — **top entry is `[1.3.0] — 2026-10-22`**

The full file is ~50 lines. There is **no `[1.4.0]` and no `[1.4.1]` entry**, despite both tags existing in git and README describing v1.4.0 features as "new". The v1.3.0 entry details the atomic copy / SQLite / streaming / dedup safety work that the v2 migration must preserve.

### Confirmed discrepancies (README/CHANGELOG vs files on disk)

1. **CHANGELOG missing 1.4.0 and 1.4.1 entries.** Top is `[1.3.0]`.
2. **README project layout references `scripts/`** — top-level `scripts/` directory is absent in the working tree (A1.1).
3. **README project layout omits** `core/cli.py`, `core/__init__.py`, `core/telemetry.py` — all present in the tree.
4. **README says AppImage output is `dist/SyncUp_v1.4.0.AppImage`** — current version is 1.4.1; `archive/` holds `8. SyncUp_v1.4.1.AppImage`. Stale after the 1.4.1 bump.
5. **README "105 unit tests" (as of v1.4.0) vs v1.3.0 commit message "74 tests"** — gap of ~31; actual count must be measured in A4, not assumed.
6. **pyproject classifiers list 3.10/3.11/3.12** but README claims CI runs on 3.10 and 3.12 only — minor drift; verify in A1.5.
7. **`README.md` "What is new in v1.4.0"** while the tag is `v1.4.1` — README was not updated at the 1.4.1 bump.

### Behavioural-oracle facts captured from `core/constants.py`

These are load-bearing for the v2 rewrite and must be preserved unless a documented deviation is approved in PLAN.md:

- `APP_NAME = "SyncUp"`, `VERSION = "1.4.1"`
- `IMAGE_EXT` includes all of: `.jpg .jpeg .png .gif .bmp .tif .tiff .heic .heif .webp .avif .dng .cr2 .nef .arw .orf .rw2 .raf .srw .pef` — **RAW formats are deliberately included so backups do not silently skip them**.
- `VIDEO_EXT`: `.mp4 .mov .avi .mkv .3gp .m4v .webm .wmv .flv .ts .mts .m2ts .mpg .mpeg`
- `MEDIA_EXT = IMAGE_EXT | VIDEO_EXT`
- `DEFAULT_SETTINGS = {"window_width": 1200, "window_height": 950, "theme": "dark"}`
- `DUP_PREFIX = "_Duplicates_"` — the exact folder name the duplicate finder uses.

## A1.5 Test configuration and CI

- **No pytest/tox/setup.cfg**: `pytest.ini`, `tox.ini`, and `setup.cfg` are absent. Tests are run with the standard library `unittest` runner.
- **CI workflow**: `.github/workflows/ci.yml` is the only workflow. It triggers on pushes to `main`/`master` and on pull requests.
  - **test job**: matrix over Python 3.10 and 3.12 on `ubuntu-latest`. Steps: checkout, setup-python with pip cache, install `requirements.txt`, compile `main.py core views` via `python -m compileall -q`, run `python -m unittest discover -s tests -t . -v`.
  - **lint job**: Python 3.12, installs `ruff>=0.6`, runs `ruff check .`.
- **No release/build workflows**: `release.yml` and `build.yml` do not exist; packaging is manual via `build.sh`.

## A1.6 Packaging and build pipeline

- **PyInstaller spec** (`SyncUp.spec`): entry point `main.py`; collects Flet data files and `assets/syncup.png`; hidden imports include `flet_desktop`, `uvicorn` submodules, `qrcode.image.svg`, plus `collect_submodules('fastapi')` and `collect_submodules('pydantic')`. Produces a windowed executable named `SyncUp` (console=False) with icon `assets/syncup.png`.
- **Build script** (`build.sh`): `set -euo pipefail`; uses `.venv/bin/python` if present else `python3`; requires `./appimagetool` and PyInstaller. Reads `VERSION` from `core.constants` and `FLET_VERSION` from `flet_desktop.version`. Runs `unittest` discovery, then `PyInstaller --noconfirm --clean SyncUp.spec`. Assembles `AppDir` with binary, icon, desktop file (version substituted from `packaging/`), appdata XML, and optionally bundles the cached Flet desktop runtime into `AppDir/usr/lib/syncup/flet` with an `AppRun` wrapper that sets `FLET_VIEW_PATH`. Packages `dist/SyncUp_v${VERSION}.AppImage` via `ARCH=x86_64 ./appimagetool --appimage-extract-and-run`.
- **Packaging assets** (`packaging/`): contains `io.github.lotus_projects_3421.SyncUp.appdata.xml` and `io.github.lotus_projects_3421.SyncUp.desktop` (template with `@VERSION@` substituted at build time).
- **Existing build output**: `dist/SyncUp_v1.4.1.AppImage` (approx. 37 MB) and `dist/SyncUp/` directory from a prior build.

---

# Audit chunk A2 — `core/` behavioural oracle

> Scope: every file under `core/` was read in full (15 modules, 5,583 lines). `tests/` was **not** audited here (A4); it was only grepped to prove whether helpers are dead. No Python file was modified. This chunk records what v2 must reproduce, not what v2 should copy verbatim.

## A2.1 `core/` — module inventory, layering, and dead surface

### Module table

| Module | LOC | Responsibility | Internal imports | Consumed by |
|---|---:|---|---|---|
| `core/__init__.py` | 5 | Re-exports `VERSION` | `constants` | package surface |
| `core/constants.py` | 28 | Media extension sets, defaults, `DUP_PREFIX` | — | engine, takeout, dedup, storage, wifi_server, tests |
| `core/cli.py` | 36 | AppImage `%U`/`%F` argv sanitisation | — | `main.py` |
| `core/logging_setup.py` | 33 | Rotating file logger (`syncup.log`) | `storage` | `main.py` |
| `core/device.py` | 128 | Linux GVFS/MTP discovery + folder listing | — | `views/phone_view.py` |
| `core/fileops.py` | 222 | Atomic publish, size verify, unique names, JSON writes | — | engine, takeout, dedup, storage, wifi_server, `views/common.py` |
| `core/naming.py` | 325 | Date parsing, EXIF probe, destination naming | — | engine, takeout, tests |
| `core/storage.py` | 289 | Settings, profiles, SQLite backup state, legacy JSON import | `constants`, `fileops` | `main.py`, engine, wifi_server, views |
| `core/telemetry.py` | 191 | Versioned progress-event schema + `ProgressEmitter` | — | all engines, `wifi_telemetry` |
| `core/takeout_engine.py` | 738 | Takeout ZIP / loose-folder organizer | `fileops`, `constants`, `naming`, `telemetry` | `views/organizer_view.py` |
| `core/dedup_engine.py` | 693 | Size → quick-hash → BLAKE2b duplicate finder, move, undo | `fileops`, `constants`, `telemetry` | `views/dedup_view.py` |
| `core/engine.py` | 1059 | Phone (MTP) backup + Wi-Fi staging backup | `fileops`, `constants`, `naming`, `storage`, `telemetry` | `views/phone_view.py`, `wifi_server` |
| `core/wifi_models.py` | 149 | Pydantic wire models + TypedDicts | — | `wifi_server` |
| `core/wifi_telemetry.py` | 255 | Engine-event → `progress_queue` bridge | `telemetry` | `wifi_server` |
| `core/wifi_server.py` | 1432 | FastAPI routes, mobile HTML, session state, Uvicorn lifecycle | `constants`, `engine`, `fileops`, `storage`, `telemetry`, `wifi_models`, `wifi_telemetry` | `views/wifi_view.py` |

### Layering (one-way; no cycles)

```
constants ──> storage ──> logging_setup
    │            │
    └──> fileops ┘
naming ──> engine ──> wifi_server ──> views
telemetry ──> engine / takeout / dedup / wifi_telemetry
```

- `core/` never imports `views/` or `main.py`; the dependency direction is strictly inward. This is worth preserving in the Rust crate split (domain/storage/services/commands).
- `wifi_server` imports `engine`, and both open `StateStore`; in one process there can be an engine-owned connection plus per-request connections to the same DB (see A2.2).
- `fileops` and `naming` are dependency-free apart from stdlib — the cleanest direct port targets.

### Load-bearing public surface

- `constants`: `IMAGE_EXT`, `VIDEO_EXT`, `MEDIA_EXT`, `DEFAULT_SETTINGS`, `DUP_PREFIX`, `APP_NAME`, `VERSION`. Values already captured in A1.4.
- `cli`: `sanitize_argv`, `is_placeholder_arg` — strips standalone `%u`/`%f` case-insensitively, always keeps `argv[0]`, leaves paths containing `%` alone.
- `fileops`: `PART_MARKER`, `is_partial_name`, `is_hidden_file`, `unique_path`, `copy_file_atomic`, `copy_stream_atomic`, `is_same_file`, `is_same_size_and_mtime`, `atomic_write_json`, `human_size`.
- `naming`: `strip_generated_prefix`, `parse_datetime_from_name`, `date_for_path`, `read_exif_datetime`, `date_with_source`, `build_target_name`, `get_target_path`.
- `storage`: `get_app_data_dir`, `get_log_dir`, `load_global_settings`, `save_global_settings`, `load_profile`, `save_profile`, `get_state_file`, `StateStore`, `load_state`, `save_state`.
- `telemetry`: phase/kind/level constants, `SCHEMA_VERSION = 1`, `ProgressEmitter`, `iso_now`, `empty_detected`, `empty_date_sources`, `with_timings`, `date_sources_label`.
- `engine`: `run_backup`, `run_wifi_backup`, `get_media_files`, plus compatibility re-exports (`parse_date`, `get_target_path`, extension sets).
- `takeout_engine`: `run_takeout`, `get_date_from_json`, `get_date_from_json_data`, `get_date_from_name`, `get_target_path`.
- `dedup_engine`: `run_dedup`, `undo_dedup`, `find_latest_manifest`, `get_file_hash`.
- `wifi_server`: `create_app`, `compute_missing_files`, `safe_relative_path`, `matches_media_filter`, `BackupSession`, `WiFiServer`, `find_available_port`, `MOBILE_HTML`, `DEFAULT_PORT = 8555`.

### Dead, unused, or stale surface (verified by repo-wide grep)

| Location | Finding |
|---|---|
| `core/fileops.py:27` | `is_hidden_file()` has **no callers** anywhere. Call sites use `is_partial_name()` + `startswith(".")` directly. |
| `core/wifi_models.py:21,29` | `WiFiManifestEntry` / `WiFiMissingFile` TypedDicts are exported but never imported. The section comment says they are "used by the state-diff helper (`core.storage.get_missing_files`)" — **that function does not exist**. The real helper is `core/wifi_server.py:703 compute_missing_files`. Stale docstring. |
| `core/wifi_models.py:107,115` | `UploadInitRequest` / `UploadInitResponse` are explicitly "reserved for a future resumable-upload flow"; no route or client uses them. **No resumable upload exists.** |
| `core/storage.py:266,274` | `load_state` / `save_state` are imported only by `tests/test_storage.py`; production code uses `StateStore` directly. |
| `core/engine.py:45-54` | `__all__` re-exports `parse_date`, `get_target_path`, extension sets "for backwards compatibility"; views import only `run_backup`. |
| `core/telemetry.py:38` | `SCHEMA_VERSION = 1` is stamped into every event, but no consumer validates it (the UI only unpacks the 4-tuple shape). Preserve as a wire-contract marker, not as a working version gate. |

### Duplicated logic (divergence risk)

- **2-second mtime tolerance is defined three times**: `fileops.py:19`, `engine.py:125`, `wifi_server.py:51`, and is separately re-encoded inside `fileops.is_same_file`, `fileops.is_same_size_and_mtime`, and `engine._is_processed`.
- **`run_backup` and `run_wifi_backup` are ~450 near-identical lines.** Differences: Wi-Fi variant skips the `Internal shared storage` probe and the destination-inside-phone guard, adds a staging-relative guard, falls back to the staging root when no folders match, de-duplicates folders by resolved path, and wraps everything in a never-raise `try/except`.
- `_matches_filter` exists verbatim in `engine.py:60` and `takeout_engine.py:57`.
- `_format_duration` exists verbatim in `engine.py:128`, `dedup_engine.py:47`, `takeout_engine.py:212`.
- "is this file already backed up" semantics are implemented twice: `engine._is_processed` and `wifi_server.compute_missing_files` (the latter explicitly claims to mirror the former).

## A2.2 Storage, settings, profiles, and backup-state migration

### Data locations (`core/storage.py:25-42`)

| Purpose | Path | Writer | Reader |
|---|---|---|---|
| Global settings | `<config>/SyncUp/settings.json` | `save_global_settings` | `load_global_settings` |
| Device profile | `<config>/SyncUp/profile_<safe_id>.json` | `save_profile` | `load_profile` |
| Backup state | `<config>/SyncUp/state_<safe_id>.db` (+ `-wal`, `-shm`) | `StateStore.mark` | `StateStore.load` |
| Legacy backup state | `<config>/SyncUp/state_<safe_id>.json` | **never written by v1.4.1** | `StateStore._migrate_legacy_json` |
| Log file | `<config>/SyncUp/logs/syncup.log` | `RotatingFileHandler` | user support |

`<config>` resolution: Windows `%APPDATA%\SyncUp`; macOS `~/Library/Application Support/SyncUp`; otherwise `$XDG_CONFIG_HOME/SyncUp` or `~/.config/SyncUp`. Windows/macOS branches exist but were never validated (A1.4).

### SQLite schema and pragmas (`core/storage.py:120-137`)

```sql
CREATE TABLE IF NOT EXISTS processed (
    rel_path TEXT PRIMARY KEY,
    size     INTEGER,
    mtime    INTEGER,
    copied_at TEXT
);
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
-- sqlite3.connect(..., timeout=30)
```

- **One table only.** No schema-version table, no `PRAGMA user_version`, no indexes beyond the primary key, no job-history table.
- `rel_path` is the source path relative to the phone root (MTP) or to the Wi-Fi staging root.
- `size` and `mtime` are the source values at copy time; `mtime` is **nanoseconds**.
- `copied_at` is a local ISO-8601 string with second precision (`datetime.now().isoformat(timespec="seconds")`).
- Concurrency: each `StateStore` owns one connection guarded by an `RLock`; the class docstring says "thread-confined". Wi-Fi routes construct a **new** `StateStore` per request because FastAPI's thread pool is not stable; the engine keeps one open for the whole run. Cross-instance safety therefore relies on SQLite's own locking plus the 30 s busy timeout.

### Row semantics (the compatibility contract)

| Stored row | Meaning | Engine decision |
|---|---|---|
| absent | never processed | candidate |
| `size IS NULL` | legacy row written by a path-only version | **trusted as backed up** (`engine.py:117-119`) |
| `size` set, `mtime IS NULL` | intermediate row | backed up if size matches |
| `size` + `mtime` set | modern row | backed up if size matches and `abs(delta) <= 2 s` |

`compute_missing_files` (`wifi_server.py:736-753`) reproduces the same rules and returns `new` / `modified` / skip. This is deliberate: upgrading must not re-copy an entire existing library.

### Write path and commit timing

1. `StateStore.__init__` → connect, set pragmas, create table, then `_migrate_legacy_json()`.
2. Engine calls `store.mark(rel_path, size, mtime, commit=(processed % 10 == 0))` **after** `copy_file_atomic` returns.
3. Final `store.commit()` after the loop; `close()` commits again.
4. A crash can therefore lose up to 9 state rows. That fails **safe** (files are re-copied, never falsely skipped), but only if the published file itself is durable — see the fsync concern in A2.3.

### Legacy JSON import (`core/storage.py:180-202`)

- Runs on **every** `StateStore` construction, including every Wi-Fi request.
- Expects a JSON **list of strings**; inserts `(path, NULL, NULL, NULL)` with `INSERT OR IGNORE`.
- Idempotent by primary key. Unreadable/non-list JSON is silently ignored.
- The legacy JSON file is **never deleted, renamed, or truncated**.
- Rows created here have no `copied_at`, so `oldest_copied_at()` ignores them.

### Corruption vs lock handling (`core/storage.py:138-178`)

- `sqlite3.OperationalError` (lock, permission, busy) → close and **re-raise**. The comment is explicit: "A lock/permission problem is not corruption."
- Other `sqlite3.DatabaseError` → close, rename `state_<id>.db` to `state_<id>.db.corrupt-<time_ns>`, rename `-wal`/`-shm` alongside, then create a fresh DB. Rename failures are swallowed.
- This is a **rename**, not a SQLite backup. The v2 standing rules require `Connection::backup` or `VACUUM INTO`; do not port this mechanism as-is.

### Settings and profiles

- Settings: JSON object; malformed/unreadable → `{}`; merged as `{**DEFAULT_SETTINGS, **data}`, so unknown keys survive and new defaults appear automatically.
- Profiles: JSON object; malformed → `None`. Device IDs are reduced by `_safe_id` (keep alphanumerics, `-`, `_`; fallback `"default"`).
- Both are written with `atomic_write_json` (temp file + `fsync` + `os.replace`) under a module-level `RLock`.
- Profile contents are **not validated** in storage; `sort_mode` falls back to `"year"` in naming, and selected folders pass through `engine._safe_relative`.

### Invariants to preserve (A2.2)

1. The state DB is the only record of "already backed up". It must never be silently deleted, reset, or recreated on a mere lock/permission error.
2. A row may only be written after the destination has been published; v1 enforces this by calling `mark()` strictly after `copy_file_atomic()` returns.
3. `size IS NULL` legacy rows must keep counting as backed up, or an upgrade would re-copy whole libraries.
4. Legacy `state_<id>.json` is imported and preserved, never destroyed.
5. Settings/profile writes are atomic so a crash cannot leave a truncated JSON document.
6. `copied_at` is best-effort metadata; absence must not mean "not backed up".

### Concerns (A2.2)

- **Legacy NULL rows are trusted forever.** A file edited in place, with the same path, will never be re-copied once it has a path-only row. This is the documented compatibility trade-off, but it is a permanent blind spot for pre-SQLite users.
- **`_safe_id` is lossy and can collide.** `mtp://phone/` and `mtp_phone_` both reduce to `mtpphone`; an all-punctuation ID becomes `"default"`. Two devices can silently share a profile/state DB.
- **All Wi-Fi sessions share `device_id="wifi-backup"`** (`wifi_server.create_app` default). Two phones, or the same phone uploading different top-level folders, share one `rel_path` namespace. A file can be skipped because a different phone previously backed up a same-path, same-size, same-mtime file.
- **No schema versioning.** Future column/table changes have no `user_version` or migration ledger to build on.
- **Durability is weaker than the invariant suggests.** `synchronous=NORMAL` + WAL can lose recent commits on power loss (it stays consistent, but rolls back), and the file-side `fsync` is best-effort (A2.3). A crash between publish and durable metadata can leave a state row that claims success for a file that did not survive. v2 must make publish + durability strictly precede the DB commit.
- **Wi-Fi re-parses the legacy JSON on every request** (cheap, but unnecessary I/O and a surprising side effect inside a GET/POST handler).
- **Corrupt-DB recovery renames the DB while another process may hold it.** The code only guards against `OperationalError` from its own connection; a second SyncUp instance is not coordinated.
- **No state DB compaction/checkpoint policy**, and `oldest_copied_at()` is a full-table `MIN()` with no index (fine at current scale, worth an index in v2).

## A2.3 File-publication safety, naming, and the backup engine

### `fileops` — the atomic publish pipeline

`copy_file_atomic(source, destination, preserve_mtime=True)` (`fileops.py:72-110`):

1. `destination.parent.mkdir(parents=True, exist_ok=True)`.
2. `source.stat()` for size + mtime.
3. Temp path = `.<destination.name>.part-<pid>-<uuid10>` **in the destination directory**, so the final step is a same-filesystem rename.
4. `shutil.copyfile(source, temp)` (contents only; **mode/permissions/xattrs are not preserved**).
5. `os.utime(temp, ns=(atime_ns, mtime_ns))` — best effort; `OSError` is swallowed for FUSE/MTP mounts.
6. `_verify_size(temp, source_stat.st_size)` — **size only, no content hash**.
7. `os.fsync(temp fd)` — **`OSError` is swallowed**.
8. `os.replace(temp, destination)` — atomic publish; replaces an existing destination only at this point.
9. On any exception: unlink the temp file and re-raise.

`copy_stream_atomic(stream, destination, expected_size=None, mtime_ns=None)` (`fileops.py:113-156`): same shape for ZIP members and HTTP uploads, but writes with a 1 MiB loop, flushes, fsyncs (best effort), compares the **written byte count** to `expected_size`, applies mtime, then `os.replace`.

`atomic_write_json` (`fileops.py:194-210`): temp file in the same directory, `json.dump(indent=2, ensure_ascii=False)`, flush, best-effort fsync, `os.replace`.

`is_partial_name(name)` requires a leading dot **and** `.part-`; hidden-file detection is `name.startswith(".") or is_partial_name(name)`.

`unique_path(destination)` produces `stem__1.ext`, `stem__2.ext`, ... by existence probing (check-then-create, not atomic).

`is_same_file` / `is_same_size_and_mtime` require equal size **and** `abs(mtime delta) <= 2 s`.

### File-safety invariants to preserve (A2.3)

1. **Never touch the destination before the new copy is verified.** All writes go to a dot-prefixed `.part-` temp in the destination directory; the destination is only changed by a single `os.replace` after the size check.
2. **Temp files live next to the destination**, guaranteeing a same-filesystem rename rather than a cross-device copy.
3. **A failed copy leaves the old destination intact** and removes its temp file.
4. **State is recorded only after `copy_file_atomic` returns**, i.e. after the publish.
5. **Same-run collisions never overwrite each other**: `created_targets` forces `unique_path()` even in overwrite mode.
6. **Hidden and partial files are never treated as media**, so `.part-*` debris is not re-imported.

### Gaps and concerns (A2.3)

- **`fsync` failure is ignored** (`fileops.py:100-105`, `136-139`, `203-206`). A disk that cannot fsync is treated as success. This directly weakens the "verified + durable before DB commit" invariant.
- **No directory fsync after `os.replace`.** The rename itself may be lost on power failure even when the file contents were fsynced.
- **Verification is size-only.** No checksum/hash confirms that the bytes written match the source; a truncated-then-padded or corrupted write of the same length would pass.
- **Metadata preservation is limited to mtime.** File mode, ownership, xattrs, and creation time are lost; `shutil.copyfile` does not copy the executable bit.
- **No explicit symlink policy.** `copyfile` follows a symlinked source; `os.replace` replaces a symlinked destination entry rather than its target. Neither behaviour is documented or tested here.
- **`unique_path` is TOCTOU-racy** across processes (probe then write). Single-process engine usage makes it acceptable today, not in a multi-window v2.
- **Orphan `.part-*` files are never cleaned at startup.** A crash mid-copy leaves debris next to the destination forever.
- **No per-file free-space check.** The engines check aggregate bytes once before the loop; a concurrent writer can still fill the disk mid-run.
- **`copy_file_atomic` does not guard `source == destination`.** Callers rely on `is_same_file` first.

### `naming` — dates, EXIF, and idempotent destination names

- Generated prefix: `%Y-%m-%d_%H-%M-%S_` (`_GENERATED_PREFIX_RE`), e.g. `2024-01-01_12-34-56_`.
- `strip_generated_prefix` removes **one or more** stacked prefixes; `build_target_name` strips before re-adding, which is what makes the organizer idempotent.
- `_DATE_TIME_RE` accepts `2024-01-01`, `20240101`, `2024_01_01`, optional time in `123456`, `12-34-56`, `12.34.56` forms. It searches anywhere in the stem, so arbitrary names containing a date are intentionally accepted.
- Midnight repair: if the generated prefix is `00-00-00` and the stripped remainder contains a same-day time, the remainder's time wins (fixes old files).
- `build_target_name`: truncates the base stem to **140 chars**, lowercases the suffix, preserves the rest verbatim.
- `get_target_path` layouts: `year` → `YYYY/`; `month` → `YYYY/YYYY-MM/`; `day` → `YYYY/YYYY-MM-DD/`; `flat` → root. **Unknown mode falls back to `year`** (never to the original bare name).

Date precedence actually implemented:

| Workflow | Order | Code |
|---|---|---|
| Phone backup (MTP) | filename → EXIF → mtime | `naming.date_with_source`, called by `engine.run_backup` |
| Wi-Fi backup | filename → EXIF → mtime | same engine path, reading from staging |
| Takeout, loose folder | JSON sidecar → filename → EXIF → mtime | `takeout_engine.run_takeout` loose branch |
| Takeout, ZIP member | JSON sidecar → filename → **ZIP member timestamp** | `takeout_engine.py:598-608` |
| `get_target_path` default (no date passed) | filename → mtime (**no EXIF**) | `naming.date_for_path` |

- EXIF reader supports **JPEG APP1 and TIFF only**, reads at most 2 MiB, and tries `DateTimeOriginal` → `DateTimeDigitized` → `DateTime`. It never raises. HEIC/HEIF/PNG/WebP/RAW EXIF is **not** read, despite those extensions being accepted media.
- Dates are naive local time; there is no timezone or UTC normalisation.

### `engine` — phone and Wi-Fi backup

`run_backup(phone_id, phone_path, selected_folders, dest_path, sort_mode, media_filter, conflict_mode, progress_callback, cancel_event, error_callback)`:

1. Resolve `phone_root = phone_path/"Internal shared storage"` if it exists, else `phone_path`.
2. **Refuse a destination inside the phone root** (`dest_root.resolve().is_relative_to(phone_root.resolve())`) before creating anything.
3. Create the destination; reject if no selected folder resolves under the phone root.
4. Open `StateStore(phone_id)` and load processed rows.
5. Scan each selected folder with `get_media_files` (recursive, hidden and `.part-` skipped), **de-duplicating overlapping folders by `rel_path`**.
6. Skip rows already backed up; build the candidate list and total bytes.
7. Aggregate free-space check (`shutil.disk_usage`); `OSError` is ignored.
8. Per file, at a file boundary: honour `cancel_event`, compute date + target, then apply the conflict matrix below, then `copy_file_atomic`, then update in-memory state and `mark()`.
9. Abort after **25 consecutive errors** (`MAX_CONSECUTIVE_ERRORS`) with a "phone may have disconnected" message.
10. Commit state, emit organize/done events, return a result dict.

Conflict matrix (exact v1 behaviour):

| Destination state | `conflict_mode="keep"` (default) | `conflict_mode="overwrite"` |
|---|---|---|
| absent | copy | copy |
| exists, same size + mtime within 2 s, **not written this run** | mark state, count skipped | same |
| exists, different, **not written this run** | `unique_path()` then copy | **verified atomic replace** |
| exists, **already written this run** | `unique_path()` then copy | `unique_path()` then copy (never clobber a same-run sibling) |

Result dict keys: `total`, `processed`, `skipped`, `errors`, `error_details` (max 100), `warnings`, `cancelled`, `total_bytes`, `bytes_copied`, `oldest_backup`, `time`, plus `detected`, `date_sources`, `started_at`, `finished_at`, `duration_seconds`, and optional fatal `error`.

`run_wifi_backup` mirrors this against `staging_root`: no `Internal shared storage` probe, no MTP guard, a staging-relative guard, a fallback to the staging root itself for flat uploads, resolved-path folder de-duplication, and an outer never-raise wrapper. **It does not delete the staging tree**; `wifi_server.run_backup_job` owns cleanup.

### Invariants to preserve (engine)

1. **Destination-inside-source is rejected before any write** (phone root and Wi-Fi staging).
2. **A file is counted as processed only after the verified copy returns**, and only then is it `mark()`ed.
3. **Overlapping folder selections copy each source path at most once per run.**
4. **Same-run target collisions are always renamed, never overwritten**, even in overwrite mode.
5. **Overwrite mode still publishes via temp + verify + replace**, so the previous file survives a failed copy.
6. **Cancellation is only honoured at file boundaries**; a partially written temp is cleaned by `fileops`, never published.
7. **Per-file failures do not abort the run** until the 25-consecutive-error threshold.
8. **A pre-existing identical file is adopted into state rather than re-copied.**

### Concerns (engine)

- **`run_backup` has no outer `try/except`.** `StateStore(phone_id)` is constructed outside the `try` (`engine.py:231`), so a DB open/migration failure propagates to the UI worker instead of becoming a result dict. `run_wifi_backup` is the safer shape.
- **Aggregate free-space check only.** A run can still hit ENOSPC mid-file; that becomes a per-file error, which is safe but noisy.
- **`is_same_file` can false-skip.** Same size + mtime within 2 s is a heuristic; content is never compared. A same-size edit that preserves mtime is silently skipped.
- **Date/EXIF probing opens every source file** before the copy, adding I/O on slow MTP mounts.
- **No verification of the destination after replace** (no re-stat/re-hash), so a filesystem that lies about the rename is not detected.
- **The two engines will drift.** Any fix (e.g. durability, conflict semantics) must be applied twice today.
- **`oldest_backup` is only advisory** and comes from `MIN(copied_at)`, which is `NULL` for all legacy rows.

## A2.4 Takeout organizer, duplicate finder, Wi-Fi server, device, telemetry, CLI

### A2.4.1 Takeout organizer (`core/takeout_engine.py`)

- Accepts a single path or a list of folders/ZIPs. Recursively walks folders, skipping dot-entries and `.part-*`, and classifies `.zip` separately from media by extension filter.
- **ZIPs are streamed member-by-member; `extractall()` is never used.** It reads the central directory once to count media members, then reopens and streams each member.
- Member paths are never used as filesystem paths: only `Path(member_name).stem` / `.suffix` feed `build_target_name`, which strips directories. This structurally prevents ZIP-slip.
- Sidecars supported: `<name>.json` and `<name>.supplemental-metadata.json`, for loose files and for ZIP members.
- Takeout JSON timestamp read from `photoTakenTime` then `creationTime`, integer `timestamp` → local `datetime.fromtimestamp`.
- Idempotency: `_resolve_destination` treats an existing destination with equal size and mtime within 2 s as already organised and skips it. `written_targets` protects same-run destinations from being overwritten even in overwrite mode.
- Conflict modes: keep → `unique_path()`; overwrite → verified atomic replace.
- Destination-inside-input is partially guarded: when walking, a directory equal to `dest_resolved` is pruned. Nested destinations are therefore not re-ingested.
- Cancellation is checked per loose file and per ZIP member.
- Loose sidecars are copied best-effort; failures are reported but do not fail the media copy.
- ZIP sidecars are compared by size + full bytes when a destination exists, then copied with `copy_stream_atomic`.
- **Date precedence deviation:** ZIP members fall back to the ZIP member timestamp, not EXIF, because EXIF cannot be probed from a stream without extracting. Loose files do use EXIF. This must be a documented v2 deviation or improved deliberately.
- **Memory:** `archive.read(sidecar_name)` loads the whole sidecar into memory (small, bounded by sidecar size, but unbounded by policy). Media is streamed.
- **No ZIP bomb / member-count / expansion-ratio limits.** `info.file_size` is trusted for the free-space estimate and as `expected_size`; a lying header fails the size check safely, but a decompression bomb can still consume CPU/disk up to the declared size.
- **Central directory is read twice** (count pass + processing pass) — a performance cost on large Takeout archives.
- **No EXIF write-back**, consistent with README.

### A2.4.2 Duplicate finder (`core/dedup_engine.py`)

Pipeline:

1. Walk the scan root, skipping dot-entries, `.part-*`, and any directory starting with `_Duplicates_`. Ignore zero-byte files.
2. Group by exact size; groups of one are discarded.
3. **Quick hash** = BLAKE2b-128 over `size` + first 64 KiB + last 64 KiB (same 64 KiB block twice when the file is smaller than 128 KiB).
4. **Full hash** = BLAKE2b over the entire file, only for quick-hash survivors.
5. Group by `(size, full_hash)`; keep policy = **shallowest path** (`len(parts)`), then **oldest mtime**, then lexicographic path.
6. Emit an exact duplicate count and byte total **before** any move.
7. `dry_run` returns preview data and moves nothing.
8. Move directory = `unique_path(scan_root / "_Duplicates_<epoch_seconds>")`.
9. Build the planned entry list (`original`, `moved_to`, `size`, `hash`), then **write `_manifest.json` atomically before the first move**.
10. Move each duplicate with `shutil.move`, refusing a destination that already exists.
11. Return counts plus `manifest` and `duplicate_dir` paths.

Manifest schema (version 1): `{version, created_at, scan_dir, entries:[{original, moved_to, size, hash}]}`. `_serialize_groups` caps the UI preview at 200 groups.

Undo (`undo_dedup`):

- Uses an explicit manifest path or `find_latest_manifest()` (newest `_Duplicates_*/_manifest.json` by mtime).
- For each entry: skip if `moved_to` is gone; skip if `original` already exists (recorded as "left in place"); otherwise recreate the parent and `shutil.move` back.
- Cancellation at entry boundaries. Malformed entries are counted as errors.

Invariants to preserve:

1. **Never permanently delete.** Duplicates are moved, not removed.
2. **Manifest first, move second.** A crash mid-move leaves a complete plan that undo can reconcile.
3. **Preview/dry-run before destructive action** is supported by the engine (the UI enforces confirmation).
4. **Keep the shallowest/oldest copy**, not an arbitrary one.
5. **Undo is idempotent and non-clobbering**: it never overwrites an existing original.

Concerns:

- **Cross-filesystem `shutil.move` is copy+delete, not atomic**, and has no fsync. If `_Duplicates_` lands on another mount (possible when the scan root spans mounts), a crash can lose the file between copy and unlink.
- **Undo trusts manifest paths blindly.** A tampered manifest can move any file the user can write, including files outside `scan_dir`. There is no containment check on `moved_to`.
- **Undo does not re-verify the hash** before restoring; it trusts the manifest.
- **No manifest update per move.** The manifest lists all planned entries; undo handles missing/present cases, which is safe but means the manifest does not record which moves actually completed.
- **Quick-hash survivors still get a full read**, so the "cheap filter" saves hashing only for size-group outliers.
- **No hash cache across runs**; repeated scans re-read everything.
- **Empty files are never considered duplicates** (`st_size > 0` filter) — intentional, but worth documenting.
- **`_Duplicates_<epoch>` uses second resolution**; two runs in the same second rely on `unique_path()` to avoid collision.

### A2.4.3 Wi-Fi server (`core/wifi_server.py`, `wifi_models.py`, `wifi_telemetry.py`)

Architecture: FastAPI app served by Uvicorn in a daemon thread; all routes are **sync** functions, so FastAPI runs them in its worker pool where blocking SQLite/disk I/O is acceptable. No route touches Flet; UI work goes through `post_ui`, progress through `progress_queue`.

Endpoints:

| Method | Path | Purpose | Validation | Response |
|---|---|---|---|---|
| GET | `/` | Mobile HTML page | none | `text/html`, `Cache-Control: no-store` |
| GET | `/favicon.ico` | Silence favicon | none | 204 |
| POST | `/manifest` | Diff phone manifest vs SQLite | Pydantic `ManifestRequest`; 409 if processing | `ManifestResponse{missing,total_bytes}` |
| POST | `/upload` | Stream one file into staging | path allowlist + size checks + free space | `UploadResponse` |
| POST | `/complete` | Start the PC-side backup | 409 if another job is busy; 400 if nothing uploaded | `StatusResponse` |
| GET | `/status` | Phone polling | none | `StatusResponse` |

Path validation (`safe_relative_path`, `wifi_server.py:650-674`): normalises `\` → `/`, rejects empty, >4096 chars, NUL, absolute paths, `~`, any `..`, any component starting with `.`, any `:` (drive letters/ADS), and SyncUp partial names. The upload route additionally resolves `(staging / parts)` and requires `is_relative_to(staging_root)`, which catches symlinked parents at check time.

Upload integrity: the file must be present in the manifest-derived `_pending`/`_expectations` map, the declared size must equal the manifest size, the multipart `UploadFile.size` (when available) must match, and the written byte count is verified by `copy_stream_atomic`. **Content is not hashed.**

Manifest diff (`compute_missing_files`): ignores unsafe/hidden/duplicate entries and entries outside the media filter; returns `new` when no row exists, `modified` when size differs or mtime differs by more than 2 s; treats `size IS NULL` rows as backed up; totals bytes for the phone.

Session state machine (`BackupSession`): `waiting → uploading → processing → done | error`. `begin_manifest` resets counters and pending sets; `note_upload` accepts each path once (a retry returns False but still overwrites the staged copy); `begin_processing` returns `started|already|done|empty`; `finish` sets `done` unless cancelled/errored and clears pending expectations so late uploads are rejected; `fail` forces `error`.

Lifecycle: `/complete` clears a stale `cancel_event`, then starts `run_backup_job` on a daemon thread. That job runs `run_wifi_backup`, calls `session.finish`, notifies the UI, and `shutil.rmtree(staging_root, ignore_errors=True)`. `WiFiServer.start/stop/wait_until_ready` wraps Uvicorn; `find_available_port` probes `0.0.0.0` from 8555 upward for 20 attempts and returns the preferred port if none is free.

**Security posture (v1.4.1):**

- **No authentication, no TLS, no pairing token, no session expiry.**
- **Binds `0.0.0.0`** — reachable by every device on the LAN.
- **No Host/Origin validation** → DNS-rebinding and CSRF-style access to `127.0.0.1:8555` are possible; multipart `/upload` is a "simple" cross-origin request.
- **No request-size, upload-count, or rate limits.**
- **Single process-global session.** Any client can `POST /manifest` and reset `_pending` mid-session, or `POST /complete`.
- **`device_id` in the manifest is accepted but ignored**; all Wi-Fi traffic shares the `"wifi-backup"` state DB.
- **No staging cleanup on startup**; a crash leaves uploaded files behind indefinitely.
- The mobile page uses `textContent` (no HTML injection) but sends no CSP header.

This is the clearest conflict with SPEC Layer 2 §5B and the standing rule "Never expose an unrestricted upload endpoint." v2 must not reproduce it.

Threading/telemetry contract worth preserving:

- Uvicorn and the backup worker are daemon threads; the UI thread never blocks.
- `WiFiTelemetry` enqueues the exact 4-tuple `(current, total, message, event)` that `main.progress_consumer` already understands.
- `BackupSession.note_progress` only tracks `phase == "transfer"` events, so progress stays meaningful even while the phone is uploading.

### A2.4.4 Device discovery (`core/device.py`)

- Linux-only, GVFS/GIO. `gio mount -li` is invoked at most once per 10 s (module-level cooldown + lock) to wake lazy MTP mounts; each discovered `mtp://` URI is then mounted with an 8 s timeout. Failures are swallowed.
- Search roots: `/run/user/<uid>/gvfs`, `/media/<user>`, `/run/media/<user>`. A directory is accepted when its name contains `"mtp"` (case-insensitive) and is a directory; duplicates by path are removed.
- Device dict: `{name, id, path}`. `name` is a heuristic: strip `mtp:host=`, URL-decode, split on `_`, take up to three parts, title-case.
- `get_folders` prefers `<device>/Internal shared storage`, falls back to the device root, lists immediate subdirectories only, excludes names starting with `.` or `_`, sorted case-insensitively.
- No Windows/macOS implementation, no real device probing beyond directory names, no MTP capability check.

### A2.4.5 Telemetry (`core/telemetry.py`, `wifi_telemetry.py`)

Event shape (schema version 1):

```json
{"version":1,"phase":"transfer","kind":"progress","level":"organize",
 "message":"Copying IMG_0001.jpg...","current":12,"total":340,"percent":3.5,
 "indeterminate":false,"timestamp":"2026-11-21T12:00:00","metrics":{...}}
```

- Phases: `scan`, `state`, `organize`, `transfer`, `hash`, `move`, `undo`, `done`.
- Kinds: `phase`, `log`, `progress`, `summary`.
- Levels: `info`, `success`, `warning`, `organize`, `error`.
- `ProgressEmitter` introspects the callback signature; 4+ positional parameters or `*args` receive the event, 3-parameter callbacks receive the legacy tuple. This compatibility shim is load-bearing for the Flet UI.
- `indeterminate` defaults to `total <= 0`; `percent` is 0 when total is 0.
- **No throttling at the emitter**; the UI's "latest progress wins" coalescer absorbs bursts.
- `empty_detected()` includes `zip_members`; `empty_date_sources()` has `filename/exif/json/mtime`.

### A2.4.6 CLI and logging

- `sanitize_argv` strips only **standalone** `%u`/`%f` tokens (case-insensitive, surrounding whitespace ignored); `argv[0]` is always kept. Embedded placeholders such as `--open=%U` are not stripped.
- Logging: `syncup` logger, level INFO, `propagate=False`, a single `RotatingFileHandler` at `logs/syncup.log` with `maxBytes=2_000_000`, `backupCount=3`, UTF-8, format `%(asctime)s %(levelname)-8s %(message)s`. No console handler.

### A2.4.7 Consolidated invariant register (v2 acceptance criteria)

| ID | Invariant | v1 evidence |
|---|---|---|
| I-01 | Never delete a user photo/video. | No code path deletes source media; dedup only moves. The sole recursive delete is `shutil.rmtree(staging_root)` for Wi-Fi upload copies, never the originals. |
| I-02 | Never replace a destination before the new copy is written and verified. | `fileops.copy_file_atomic` temp → size verify → `os.replace`. |
| I-03 | Temp files are destination-local so publication is a same-FS rename. | `_temp_path_for` in `fileops.py:46-53`. |
| I-04 | Failed copies clean their temp and leave the old destination intact. | `except: _cleanup_temp(temp); raise`. |
| I-05 | State is written only after a verified publish. | `mark()` after `copy_file_atomic` in both engines. |
| I-06 | Legacy NULL-size state rows remain "backed up". | `engine._is_processed`, `compute_missing_files`. |
| I-07 | Legacy `state_<id>.json` is imported, never destroyed. | `storage._migrate_legacy_json`. |
| I-08 | A lock/permission error is not treated as corruption. | `OperationalError` re-raised in `StateStore._connect`. |
| I-09 | Same-run target collisions never overwrite each other. | `created_targets` + `unique_path`. |
| I-10 | Overwrite is atomic and verified. | `os.replace` only after size check. |
| I-11 | Cancellation is honoured at file boundaries; partial temps are never published. | `cancel_event` checks + temp cleanup. |
| I-12 | Overlapping source folders copy each file once. | `seen_paths` de-duplication. |
| I-13 | Media filters honour the full extension taxonomy, including RAW. | `constants.MEDIA_EXT` + `_matches_filter`. |
| I-14 | Destination inside the source is rejected before writing. | `engine.py:182`, `engine.py:632`. |
| I-15 | ZIPs are streamed; `extractall()` is never used. | `zipfile` member streaming only. |
| I-16 | ZIP member names cannot escape the destination. | Only `stem`/`suffix` are used for naming. |
| I-17 | Organizer re-runs are idempotent. | prefix stripping + size/mtime skip. |
| I-18 | Duplicate cleanup never deletes; it moves to `_Duplicates_`. | `shutil.move`, no unlink. |
| I-19 | Duplicate manifest exists before the first move. | `atomic_write_json` precedes the loop. |
| I-20 | Undo never clobbers an existing original. | `if original.exists(): skipped`. |
| I-21 | Upload paths are validated server-side. | `safe_relative_path` + `is_relative_to`. |
| I-22 | Uploads are restricted to manifest-requested paths and declared sizes. | `session.expectation` + size checks. |
| I-23 | The UI thread is never blocked by workers. | queue + `post_ui` contract; Uvicorn daemon thread. |
| I-24 | Progress events remain 3-arg compatible. | `ProgressEmitter._accepts_event`. |
| I-25 | AppImage `%U`/`%F` never reaches folder-open logic. | `sanitize_argv` at `main.py:28`. |

### A2.4.8 Not found in `core/` (absent capabilities v2 must add or explicitly defer)

- **No Wi-Fi authentication, pairing token, session expiry, TLS, rate limiting, or Host/Origin validation.**
- **No resumable uploads** (`UploadInit*` models are unused stubs).
- **No schema versioning / migration ledger** for the state DB.
- **No SQLite backup via the SQLite API**; corruption recovery is a file rename.
- **No directory fsync and no enforced fsync success** on publish.
- **No content hash verification** on copy (size only).
- **No EXIF support for HEIC/HEIF/PNG/WebP/RAW**, and none for ZIP members.
- **No Windows/macOS device layer.**
- **No staging cleanup on startup**, and no recovery for interrupted uploads.
- **No `.part-*` orphan cleanup on startup.**
- **No file mode/ownership/xattr preservation.**
- **No per-file free-space reservation.**
- **No job-history persistence** beyond the `processed` table.
- **No throttling in the telemetry emitter** (relies on UI coalescing).
- **No dedicated `scripts/` directory** (already noted in A1.4; not referenced by `core/`).

---

# Audit chunk A3 — persistence internals (schema, WAL, config formats, migration surface)

> Scope: deeper persistence detail than A2.2. Every fact below was derived **only** from source code that reads/writes these files (`core/storage.py`, `core/engine.py`, `core/wifi_server.py`, `core/constants.py`, `core/fileops.py`, `main.py`, `views/phone_view.py`, `views/wifi_view.py`, `tests/test_storage.py`). No real file under `~/.config/SyncUp/` was read, listed, copied, or opened. All JSON examples are **SYNTHETIC**.
>
> Relationship to A2.2: A2.2 gives the storage summary and compatibility contract. A3.1–A3.7 add full DDL, exact file paths, exact synthetic shapes, migration mechanics, and the literal queries with line numbers. Where A2.2 already states a fact, A3 references it rather than repeating the rationale.

## A3.1 SQLite schema (full DDL)

### Where the schema is created

- **Only one schema site exists in the entire repo**: `core/storage.py:120-137` (`StateStore._connect`). Repo-wide search for `CREATE TABLE`, `executescript`, `sqlite3.connect`, `PRAGMA`, and `user_version` found no other schema owner.
- The same DDL is duplicated verbatim in the corruption-recovery branch at `core/storage.py:166-178`. There is no `CREATE INDEX`, no `ALTER TABLE`, no trigger, no view, and no `executescript`.
- One database **per device/profile**: `state_<safe_id>.db` (`core/storage.py:115`). There is no shared/global database.

### Full DDL — every table, every column

```sql
-- core/storage.py:126-135 (normal path) and :167-176 (corrupt-DB fallback path)
CREATE TABLE IF NOT EXISTS processed (
    rel_path TEXT PRIMARY KEY,
    size INTEGER,
    mtime INTEGER,
    copied_at TEXT
)
```

That is the **complete** schema. There are **no** other tables.

| Column | Declared type | Constraints / defaults | Semantics |
|---|---|---|---|
| `rel_path` | `TEXT` | `PRIMARY KEY` (implies `NOT NULL` + unique) | Source path relative to the phone storage root (MTP) or Wi-Fi staging root, POSIX-style for Wi-Fi (`as_posix()`), OS-native string for MTP (`str(...relative_to...)`) |
| `size` | `INTEGER` | nullable, no default | Source byte size at copy time. `NULL` = legacy path-only row |
| `mtime` | `INTEGER` | nullable, no default | Source `st_mtime_ns` at copy time (**nanoseconds**). `NULL` = legacy/intermediate row |
| `copied_at` | `TEXT` | nullable, no default | Local ISO-8601 with second precision, `datetime.now().isoformat(timespec="seconds")` (`core/storage.py:243`) |

- No `NOT NULL` constraints, no `DEFAULT` clauses, no `CHECK`, no `FOREIGN KEY`, no `STRICT` table, no `WITHOUT ROWID`.
- SQLite type affinity only: `TEXT`, `INTEGER`, `TEXT`.

### Full DDL — every index

There are **no explicit `CREATE INDEX` statements**. The only index is the implicit unique index SQLite creates for `rel_path TEXT PRIMARY KEY` (`sqlite_autoindex_processed_1`). Consequences:

- Exact `rel_path` equality lookups are index-backed.
- `MIN(copied_at)` (`core/storage.py:217-219`) is a full-table scan (no index on `copied_at`).
- `load()` reads **every row** (`core/storage.py:206-208`); callers then build an in-memory dict and do exact-key lookups. The PK does not reduce that full read.

### Schema-versioning mechanism

- **None.** No `PRAGMA user_version` read or write anywhere. No `migrations` / `schema_version` table. No version constant for the DB. Repo-wide grep for `user_version` returned zero hits.
- The only idempotent upgrade primitive is `CREATE TABLE IF NOT EXISTS`. There is no mechanism to add a column to an existing `processed` table.
- Consequence for v2: there is no way to distinguish "v1 schema, fully migrated" from "v1 schema, never touched" or "partially migrated". A3.5 and A3.7 expand on this.

### Complete SQL statement inventory (v1.4.1)

| Purpose | Statement | Location |
|---|---|---|
| Open connection | `sqlite3.connect(str(self.db_path), timeout=30)` | `core/storage.py:123`, `:166` |
| Journal mode | `PRAGMA journal_mode=WAL` | `core/storage.py:124` |
| Sync level | `PRAGMA synchronous=NORMAL` | `core/storage.py:125` |
| Create schema | `CREATE TABLE IF NOT EXISTS processed (...)` | `core/storage.py:126-135`, `:167-176` |
| Legacy import | `INSERT OR IGNORE INTO processed(rel_path, size, mtime, copied_at) VALUES(?, ?, ?, ?)` | `core/storage.py:197-201` |
| Load all | `SELECT rel_path, size, mtime FROM processed` | `core/storage.py:206-208` |
| Oldest timestamp | `SELECT MIN(copied_at) FROM processed WHERE copied_at IS NOT NULL` | `core/storage.py:217-219` |
| Upsert one row | `INSERT INTO processed(...) VALUES(?, ?, ?, ?) ON CONFLICT(rel_path) DO UPDATE SET size=excluded.size, mtime=excluded.mtime, copied_at=excluded.copied_at` | `core/storage.py:234-244` |
| Commit | `Connection.commit()` | `core/storage.py:136`, `:202`, `:246`, `:250`, `:255` |

## A3.2 WAL and journal mode findings

**WAL is enabled: YES**, on the normal connection path, at `core/storage.py:124`:

```python
conn = sqlite3.connect(str(self.db_path), timeout=30)
conn.execute("PRAGMA journal_mode=WAL")
conn.execute("PRAGMA synchronous=NORMAL")
```

Details that matter for a safe v2 migration:

1. **WAL is a persistent database property.** Once written, it is recorded in the DB header. Opening the DB read-only still shows `journal_mode=wal`.
2. **`synchronous=NORMAL`** (not `FULL`) accompanies WAL. With WAL, `NORMAL` can lose the most recent committed transactions after a power loss (the DB stays consistent; it rolls back). A migration must not assume the newest `copied_at` rows survived a hard crash.
3. **The corruption-recovery reconnect does NOT set the pragmas** (`core/storage.py:166-178`): it calls `sqlite3.connect(...)`, runs only `CREATE TABLE IF NOT EXISTS`, and returns. A freshly recreated DB after corruption therefore uses SQLite defaults (`journal_mode=delete`, `synchronous=FULL`) until the next normal `StateStore` construction applies WAL again.
4. **The return value of `PRAGMA journal_mode=WAL` is not checked.** If the filesystem refuses WAL (e.g. some network/FUSE mounts), the code does not notice or report it.
5. **No checkpoint policy, no `wal_checkpoint`, no `VACUUM`.** `-wal`/`-shm` files persist for the life of the DB.
6. **Backup implication (definitional):** a plain file copy of `state_<id>.db` alone can miss committed data still in `-wal`. v2 must use `rusqlite::Connection::backup` / `VACUUM INTO` on a live connection, or checkpoint + copy the DB **and** its `-wal`/`-shm` as a set. The v1 corruption path (`os.replace` of the DB and sidecars) is a rename, not a SQLite backup, and must not be ported as-is (already flagged in A2.2).
7. **Concurrency:** each `StateStore` owns one connection with an `RLock`; Wi-Fi creates a new `StateStore` per request (`core/wifi_server.py:687-700`), while the engine keeps one open for a run. Cross-connection safety depends on SQLite locking and the 30 s busy timeout. There is no `PRAGMA busy_timeout`; the timeout is supplied by `sqlite3.connect(timeout=30)`.

## A3.3 Configuration file formats (synthetic examples)

All three formats are plain UTF-8 JSON. Settings and profiles are written by `core/fileops.py:194-210` (`atomic_write_json`): temp file `.<name>.part-<pid>-<10hex>` in the same directory, `json.dump(..., indent=2, ensure_ascii=False)`, flush, best-effort `fsync`, then `os.replace`. There is no schema validation on write.

### A3.3.1 `settings.json`

Path: `<app_data_dir>/settings.json` (`core/storage.py:51`, `:66`).

| Key | Type | Default | Read by | Written by |
|---|---|---|---|---|
| `theme` | string, exactly `"dark"` or `"light"` (any other value is treated as dark) | `"dark"` | `main.py:61,65-69` | `main.py:176-189` (theme toggle) |
| `window_width` | integer (Flet window width) | `1200` | `main.py:74` | `main.py:496-506` (`on_close`) |
| `window_height` | integer (Flet window height) | `950` | `main.py:75` | `main.py:496-506` (`on_close`) |

- Defaults live in `core/constants.py:21-25` (`DEFAULT_SETTINGS`).
- `load_global_settings` (`core/storage.py:49-61`) starts from `{}`, reads the file only if it exists and parses to a dict, and returns `{**DEFAULT_SETTINGS, **data}`. **Unknown keys survive**; malformed JSON / non-dict / `OSError` silently yields `{}` then defaults.
- `save_global_settings` (`core/storage.py:64-66`) writes the **entire in-memory dict** (including unknown keys) under a module `RLock`, atomically. The app holds one dict for the process lifetime, so a concurrent external edit can be overwritten.
- No per-device settings exist; `settings.json` is global.

**SYNTHETIC example** (not from any real user file):

```json
{
  "window_width": 1200,
  "window_height": 950,
  "theme": "dark"
}
```

### A3.3.2 `profile_<safe_id>.json`

Path: `<app_data_dir>/profile_<safe_id>.json` (`core/storage.py:74-75`). `safe_id` is `_safe_id(device_id)` (`core/storage.py:69-71`): keep only alphanumerics, `-`, `_`; fall back to `"default"`. The Wi-Fi tab always uses the literal device id `"wifi-backup"` (`views/wifi_view.py:45`), i.e. `profile_wifi-backup.json`.

| Key | Type | Default when absent/invalid | Read by | Written by |
|---|---|---|---|---|
| `destination` | string (absolute destination folder) | `""` | `views/phone_view.py:279`; `views/wifi_view.py:187` | Phone: `views/phone_view.py:402-411`; Wi-Fi: `views/wifi_view.py:403-413` |
| `sorting` | string enum: `"day"`, `"month"`, `"year"`, `"flat"` | `"year"` | `views/phone_view.py:280-283`; `views/wifi_view.py:188-190` | same as above |
| `filter` | string enum: `"both"`, `"photos"`, `"videos"` | `"both"` | `views/phone_view.py:284-287`; `views/wifi_view.py:191-193` | same as above |
| `conflict` | string enum: `"keep"`, `"overwrite"` | `"keep"` | `views/phone_view.py:288-291`; `views/wifi_view.py:194-196` | same as above |
| `folders` | array of strings (phone-relative folder names, leading `/` stripped on read) | `[]` (empty set) | `views/phone_view.py:292-300` | **Phone only**: `views/phone_view.py:402-411`. Wi-Fi never writes `folders` |

- `load_profile` (`core/storage.py:78-87`) returns `None` for missing file, malformed JSON, `OSError`, or non-dict. It performs **no key/type validation**.
- Enum validation happens in the views, not in storage: invalid `sorting`/`filter`/`conflict` fall back to `year`/`both`/`keep`; `folders` is coerced with `str(...).strip("/")` and empties are dropped.
- `save_profile` (`core/storage.py:90-92`) writes exactly the keys the caller passes and **replaces the whole file** atomically. Phone saves all five keys; Wi-Fi saves four (no `folders`).
- `destination` is stored verbatim; it is **not** validated or made absolute at persistence time.

**SYNTHETIC phone profile** (`profile_mtp-example-device.json`; not from any real user file):

```json
{
  "destination": "/home/example/Pictures/PhoneBackup",
  "sorting": "day",
  "filter": "both",
  "conflict": "keep",
  "folders": [
    "DCIM/Camera",
    "Pictures/Screenshots"
  ]
}
```

**SYNTHETIC Wi-Fi profile** (`profile_wifi-backup.json`; not from any real user file):

```json
{
  "destination": "/home/example/Pictures/WiFiBackup",
  "sorting": "year",
  "filter": "photos",
  "conflict": "overwrite"
}
```

### A3.3.3 `state_<safe_id>.json` (legacy)

Path: `<app_data_dir>/state_<safe_id>.json` (`core/storage.py:99-101`).

| Property | Value |
|---|---|
| Shape | A JSON **array of strings** (path-only processed list). Anything that is not a list is ignored |
| Element type | Coerced with `str(entry)` at import (`core/storage.py:193`) |
| Default | No file = nothing imported |
| Written by | **Nothing in v1.4.1.** No writer exists in the repo; it is read-only legacy input |
| Read by | `StateStore._migrate_legacy_json` (`core/storage.py:180-202`), invoked from `StateStore.__init__` (`core/storage.py:118`) on **every** construction, including every Wi-Fi request |

- Imported rows get `size = NULL`, `mtime = NULL`, `copied_at = NULL` (`core/storage.py:193`) and are inserted with `INSERT OR IGNORE` (`core/storage.py:197-201`).
- The file is **never deleted, renamed, or truncated**.
- Unreadable/malformed JSON and non-list values are silently ignored (`core/storage.py:187-191`).
- Duplicate strings collapse to one row via the primary key.

**SYNTHETIC example** (not from any real user file):

```json
[
  "DCIM/Camera/IMG_0001.jpg",
  "DCIM/Camera/IMG_0002.mp4",
  "Pictures/Screenshots/Screenshot_20260101.png"
]
```

### A3.3.4 Related non-JSON persistence

| File | Shape | Writer |
|---|---|---|
| `state_<safe_id>.db` | SQLite, `processed` table (A3.1) | `StateStore.mark` / `commit` |
| `state_<safe_id>.db-wal`, `-shm` | SQLite WAL sidecars | SQLite (WAL mode) |
| `state_<safe_id>.db.corrupt-<time_ns>` | Renamed damaged DB | `StateStore._connect` (`core/storage.py:155-165`) |
| `state_<safe_id>.db.corrupt-<time_ns>-wal` / `-shm` | Renamed sidecars | same |
| `logs/syncup.log` (+ rotations) | Text log, `RotatingFileHandler` | `core/logging_setup.py` |
| `_Duplicates_<epoch>/_manifest.json` | Dedup undo manifest, **in the scan destination tree**, not in the config dir | `core/dedup_engine.py` |

## A3.4 Legacy file locations (for migration)

### The exact directory

All persistent user files live in **one directory**, created by `get_app_data_dir()` (`core/storage.py:25-35`):

```
<app_data_dir> = <platform config base> / "SyncUp"
```

| Platform | `<platform config base>` | Resulting `<app_data_dir>` |
|---|---|---|
| Linux | `$XDG_CONFIG_HOME` if set, else `~/.config` | `$XDG_CONFIG_HOME/SyncUp` or `~/.config/SyncUp` |
| Windows | `%APPDATA%` if set, else `~/AppData/Roaming` | `%APPDATA%\SyncUp` |
| macOS | `~/Library/Application Support` | `~/Library/Application Support/SyncUp` |

- The directory is created eagerly (`mkdir(parents=True, exist_ok=True)`, `core/storage.py:34`), so a bare `settings.json` load creates it even before any backup.
- Linux/macOS/Windows branches exist; only Linux was validated for v1.4.1 (A1.4).

### Paths relative to the SQLite DB

For a given device `safe_id`, the DB is `state_<safe_id>.db` **directly inside `<app_data_dir>`** (`core/storage.py:115`). Every other file is a **sibling in the same directory**:

```
<app_data_dir>/
├── settings.json                         # global settings
├── profile_<safe_id>.json                # one per device/profile
├── state_<safe_id>.db                    # SQLite backup state
├── state_<safe_id>.db-wal                # WAL sidecar (when WAL active)
├── state_<safe_id>.db-shm                # shared-memory sidecar
├── state_<safe_id>.db.corrupt-<ns>       # renamed damaged DB, if recovery ran
├── state_<safe_id>.db.corrupt-<ns>-wal   # renamed sidecar, if recovery ran
├── state_<safe_id>.json                  # LEGACY path-only JSON (import only)
├── .<name>.part-<pid>-<10hex>            # possible leftover atomic-write temp
└── logs/
    └── syncup.log                        # + rotated backups
```

Migration-relevant facts:

1. **`state_<safe_id>.json` sits next to `state_<safe_id>.db`**, not in a subdirectory. A migration that enumerates `<app_data_dir>/state_*.json` and `<app_data_dir>/state_*.db` finds both.
2. **`profile_<safe_id>.json` shares the same `safe_id` namespace** as the state files, so `profile_X.json`, `state_X.db`, and `state_X.json` can be grouped by the same `X`. `settings.json` is unqualified/global.
3. **The original device id cannot be reconstructed from the filename.** `_safe_id` is lossy (`core/storage.py:69-71`): `mtp://phone/` and `mtp_phone_` both become `mtpphone`; an all-punctuation id becomes `default`. Two devices can already share one file. Import must treat the filename suffix as the only device key and must detect collisions.
4. **A legacy-only user may have `state_<safe>.json` with no `.db`.** `StateStore` creates the DB and imports the JSON on first open, so v2 discovery must look for the JSON independently of the DB.
5. **Wi-Fi always uses `safe_id = "wifi-backup"`** (`views/wifi_view.py:45`), so its files are `profile_wifi-backup.json` / `state_wifi-backup.db` / `state_wifi-backup.json`.
6. **Do not copy the DB alone while WAL is active** (A3.2). Migrate via a SQLite connection/backup API.
7. **Temp-file debris** (`.settings.json.part-*`, `.profile_*.part-*`, `.state_*.part-*`) may remain after a crash; they are not valid inputs and must be ignored/cleaned, not parsed.
8. **`state_*.db.corrupt-*` files are not the live DB.** They are preserved damaged copies; a migration must not treat them as current state.

## A3.5 Existing migration logic

### A3.5.1 JSON → SQLite state import

The **only** migration code is `StateStore._migrate_legacy_json` (`core/storage.py:180-202`), called unconditionally from `StateStore.__init__` (`core/storage.py:118`).

Sequence:

1. Compute `state_<safe_id>.json` via `get_state_file` (`core/storage.py:181`).
2. If absent, return.
3. Read and `json.load`; on `OSError`/`JSONDecodeError`, return silently.
4. If the parsed value is not a `list`, return silently.
5. Build `rows = [(str(entry), None, None, None) for entry in entries]`.
6. If empty, return.
7. `executemany("INSERT OR IGNORE ...", rows)` then `commit()` (`core/storage.py:196-202`).

Assessment:

| Property | Finding |
|---|---|
| Transactional | **Yes, effectively.** `executemany` + `commit` is one implicit transaction; an interruption before commit rolls the batch back. `INSERT OR IGNORE` makes a retry safe. |
| Idempotent | **Yes**, by `rel_path` primary key. Re-running inserts nothing new. |
| Safe on interruption | **Yes for data loss** (the JSON is never modified), but **not detectable**: there is no completion marker. If the process dies mid-import, the next open simply retries. If the JSON is corrupt/unreadable, it is skipped **every** time with no error surfaced. |
| Re-runs forever | **Yes.** It re-reads and re-parses the JSON on every `StateStore` construction, including every Wi-Fi HTTP request. Cheap for small files, unbounded for large ones. |
| Preserves source | **Yes.** The JSON file is never deleted or rewritten. |
| Validates rows | **No.** Elements are blindly `str()`-coerced; a dict becomes its repr, `None` becomes `"None"`. |
| Schema upgrade | **None.** No column/table changes; only `CREATE TABLE IF NOT EXISTS`. |
| Version gate | **None.** No `user_version`, no ledger. |

### A3.5.2 Schema upgrades

- There are **no** schema upgrade functions, no `ALTER TABLE`, and no version constants. A v1.4.1 DB is created with the schema in A3.1 and never altered.
- The corrupt-DB branch (`core/storage.py:138-178`) is **not** a migration: it closes the connection, renames `state_<id>.db` (and any `-wal`/`-shm`) to `.corrupt-<time_ns>`, then creates a **fresh empty** DB. Old rows are not copied forward; the legacy JSON (if any) is re-imported on the next call. Rename failures are swallowed, and `OperationalError` (lock/permission/busy) is explicitly re-raised instead (`core/storage.py:141-147`).

### A3.5.3 Settings and profile migration

- **None.** `settings.json` and `profile_<id>.json` are read directly each time; there is no versioning, no conversion, and no default-backfill step beyond the read-time merges/fallbacks in A3.3.
- Unknown keys in `settings.json` survive because of `{**DEFAULT_SETTINGS, **data}`; unknown profile keys are ignored by the views but not removed by storage unless a view rewrites the file (which replaces it with only its known keys).

### A3.5.4 What v2 must add (per SPEC §6 and standing rules)

- A schema version (`PRAGMA user_version` or a ledger table) and a resumable, idempotent migration with a completion record.
- A verified SQLite backup before migration, using the SQLite API (WAL-aware), never a plain file copy.
- Explicit handling/reporting of unreadable legacy JSON, instead of silent skip (SPEC: "must never silently skip files because the old state could not be read").
- Preservation of `NULL` size/mtime semantics and of the original files.

## A3.6 Backup-state queries (file:line)

### (a) "Is this file already backed up?" queries

**In-memory decision function (authoritative for phone + Wi-Fi engine runs)** — `core/engine.py:111-125`:

```python
def _is_processed(entry: dict | None, size: int, mtime_ns: int) -> bool:
    """Decide whether a state row means this exact file is already safe."""
    if entry is None:
        return False
    stored_size = entry.get("size")
    if stored_size is None:
        # Legacy row: the old app only stored paths.  Trust it to avoid
        # re-copying an entire existing library.
        return True
    if stored_size != size:
        return False
    stored_mtime = entry.get("mtime")
    if stored_mtime is None:
        return True
    return abs(stored_mtime - mtime_ns) <= 2_000_000_000
```

Call sites: phone scan `core/engine.py:273-279`; Wi-Fi scan `core/engine.py:732-737`.

**The query that supplies that function's data** — `core/storage.py:204-212`:

```python
def load(self) -> dict[str, dict]:
    with self._lock:
        rows = self._conn.execute(
            "SELECT rel_path, size, mtime FROM processed"
        ).fetchall()
    return {
        rel_path: {"size": size, "mtime": mtime}
        for rel_path, size, mtime in rows
    }
```

Call sites: `core/engine.py:232` (phone), `core/engine.py:692` (Wi-Fi), `core/wifi_server.py:693-695` (per-request `_read_state`), `core/storage.py:269` (`load_state` compatibility helper).

**Wi-Fi manifest diff equivalent** — `core/wifi_server.py:703-764`, specifically `:736-753`:

```python
        stored = state.get(rel)
        status: str | None = None
        if stored is None:
            status = "new"
        else:
            stored_size = stored.get("size")
            if stored_size is None:
                # Legacy path-only row: the engine trusts it too.
                continue
            if int(stored_size) != entry.size:
                status = "modified"
            else:
                stored_mtime = stored.get("mtime")
                if (
                    stored_mtime is not None
                    and abs(int(stored_mtime) - entry.mtime_ns) > MTIME_TOLERANCE_NS
                ):
                    status = "modified"
```

`MTIME_TOLERANCE_NS = 2_000_000_000` (`core/wifi_server.py:51`). Route call site: `/manifest` at `core/wifi_server.py:1103-1108`.

**Advisory "oldest backup" query** — `core/storage.py:214-222`:

```python
row = self._conn.execute(
    "SELECT MIN(copied_at) FROM processed WHERE copied_at IS NOT NULL"
).fetchone()
```

### (b) "Insert/update a backed-up record" queries

**Normal upsert (after a verified publish)** — `core/storage.py:225-246`, SQL at `:234-244`:

```python
            self._conn.execute(
                """
                INSERT INTO processed(rel_path, size, mtime, copied_at)
                VALUES(?, ?, ?, ?)
                ON CONFLICT(rel_path) DO UPDATE SET
                    size = excluded.size,
                    mtime = excluded.mtime,
                    copied_at = excluded.copied_at
                """,
                (rel_path, size, mtime, datetime.now().isoformat(timespec="seconds")),
            )
            if commit:
                self._conn.commit()
```

Call sites and timing:

| Call site | When | Commit policy |
|---|---|---|
| `core/engine.py:471-476` | After `fileops.copy_file_atomic` returns (phone copy) | `commit=(processed % STATE_COMMIT_EVERY == 0)`, i.e. every 10th copied file |
| `core/engine.py:428-432` | Existing destination adopted as identical (phone), **before** the skip | Default `commit=False` (flushed later) |
| `core/engine.py:923-928` | After `copy_file_atomic` returns (Wi-Fi copy) | Every 10th copied file |
| `core/engine.py:887-891` | Existing destination adopted as identical (Wi-Fi) | Default `commit=False` |
| `core/storage.py:281,283,286` | `save_state` compatibility helper (tests only) | `store.commit()` after the loop |
| `core/storage.py:248-257` | `commit()` / `close()` | Final flush; `close()` commits then closes |

**Legacy bulk insert (path-only, `NULL` metadata)** — `core/storage.py:196-202`:

```python
        with self._lock:
            self._conn.executemany(
                "INSERT OR IGNORE INTO processed(rel_path, size, mtime, copied_at) "
                "VALUES(?, ?, ?, ?)",
                rows,
            )
            self._conn.commit()
```

`STATE_COMMIT_EVERY = 10` is defined at `core/engine.py:57`. Consequence: a crash can lose up to 9 uncommitted state rows. That fails **safe** (re-copy) but means the v2 migration cannot assume the v1 DB records every file that was actually copied.

## A3.7 Data-safety risks for v2 migration

Ordered by severity; each is grounded in the code above.

### R1 — WAL-aware backup is mandatory; a file copy is unsafe

`PRAGMA journal_mode=WAL` (`core/storage.py:124`) means committed rows can live in `state_<id>.db-wal` at the moment of migration. A naive copy of the `.db` alone can silently omit recent backup history, and then v2 would re-copy (or, worse, mis-reconcile) files. **Required:** open the legacy DB and use `Connection::backup` / `VACUUM INTO`; verify the backup (integrity check + row count + representative rows); never modify the source; never treat a locked DB as corrupt (`OperationalError` is explicitly re-raised at `core/storage.py:141-147`).

### R2 — The three-state row contract must be preserved bit-for-bit

The skip logic distinguishes:

- no row → not backed up;
- `size IS NULL` → **trusted as backed up forever** (`core/engine.py:116-119`, `core/wifi_server.py:742-744`);
- `size` set, `mtime IS NULL` → backed up if size matches (`core/engine.py:122-124`);
- both set → size equal and `|mtime_ns delta| <= 2 s` (`core/engine.py:125`).

If v2 normalises `NULL`s to `0`, adds `NOT NULL`, or "repairs" legacy rows, it will either re-copy entire libraries (safe but disruptive) or falsely mark changed files as backed up (violates the safety invariant). The migration must copy the exact tri-state values, including `mtime` in **nanoseconds**, and must not synthesize `copied_at` for legacy rows.

### R3 — `_safe_id` is lossy and can collide; device identity is not recoverable

`_safe_id` (`core/storage.py:69-71`) strips everything except alphanumerics/`-`/`_` and falls back to `"default"`. Distinct device IDs can already map to the same `profile_*`/`state_*` file, and the original ID is not stored anywhere. v2 must:

- import by the filename suffix as an opaque legacy key (or preserve the filename verbatim as a legacy namespace),
- detect two source files that would collide in a new consolidated schema,
- avoid merging rows from different devices under one `(device, rel_path)` key,
- remember that all Wi-Fi rows share `"wifi-backup"` and therefore are **not** attributable to a specific phone (A2.2 concern).

### R4 — Legacy JSON import is silent on failure

`_migrate_legacy_json` ignores `OSError`, `JSONDecodeError`, and non-list JSON (`core/storage.py:187-191`), and coerces every element with `str()` (`:193`). SPEC §6 explicitly forbids silently skipping files when old state cannot be read. v2 must log/report malformed legacy files, count imported vs skipped entries, and surface an actionable migration report — while still never deleting or rewriting the JSON.

### R5 — No schema version means partial migration is undetectable

There is no `user_version`, no ledger, and no completion marker (A3.1/A3.5). A crash mid-migration is indistinguishable from "never migrated". v2 needs a versioned, transactional, idempotent migration with an explicit completion record and a way to detect/repair an interrupted run (SPEC §6: "Detect incomplete migration").

### R6 — Commit batching can under-report copied files

The engine commits every 10th copy (`core/engine.py:475`, `:927`) and `synchronous=NORMAL` can lose the last WAL commits on power loss. Migration must treat the legacy DB as a **lower bound** on what was backed up, not an exact journal. Conversely, because `size IS NULL` rows are trusted, the migration must not "fill in" sizes from the current filesystem — that would silently convert a legacy trusted row into a size/mtime claim v1 never verified.

### R7 — Profiles/settings are unvalidated and whole-file rewrites

`destination` is stored verbatim, enums are only validated on read, `folders` is coerced in the view, and `save_*` replaces the entire file (`core/storage.py:64-66`, `:90-92`). A migration that rewrites these files must preserve unknown keys (settings merge behaviour) and must not lose `folders` on the Wi-Fi profile (v1 never wrote it there, so absence is normal). Paths in `destination` may no longer exist or may point at another user/mount; migrate the string, do not resolve or "fix" it.

### R8 — Concurrency and multi-process access during migration

Wi-Fi opens a new `StateStore` per request (`core/wifi_server.py:687-700`) while the engine holds another; v1 also never coordinates a second SyncUp instance. v2 must acquire an exclusive/consistent view (SQLite backup API gives a consistent snapshot) and must not rename or delete a DB that another process may hold. The v1 corrupt-recovery rename is unsafe under concurrency and must not be reused.

### R9 — Corrupt/partial artifacts in the same directory

The config directory may contain `state_*.db.corrupt-<ns>`, `-wal`/`-shm` sidecars, and `.part-*` temp files. Migration discovery must select only live `state_<safe>.db` / `state_<safe>.json` / `profile_<safe>.json`, ignore temp debris, and treat `.corrupt-*` as evidence for a recovery report — not as current state.

### R10 — No persistent dedup state to migrate

The duplicate finder keeps no DB rows: dedup state is in-memory hashes plus an on-disk `_Duplicates_<epoch>/_manifest.json` in the scan tree (`core/dedup_engine.py`, A2.4.2). There is nothing to import into the new schema for dedup; do not invent a migration. Only `processed` is persistent backup state.

### Open questions / not determinable from source

- **Real-world row counts, `NULL`-row prevalence, and whether any live DB is actually in WAL mode** — cannot be known without reading user data, which is forbidden. All conclusions above are code-derived.
- **Whether any production user has `state_*.json` without a matching `.db`** — possible by code path, but not observable here.
- **Original device IDs behind sanitized filenames** — unrecoverable by design.
- **Windows/macOS path behaviour** — code branches exist but were never validated (A1.4).

---

# Audit chunk A4 — views, Wi-Fi routes/session/security, platform code, test baseline, feature inventory

> Scope: `views/` (all six modules read in full), `core/wifi_server.py` (re-read for routes/auth/session/upload limits), platform branches across the repo, the legacy test suite, and a live test-baseline run. A2.4 already covered Wi-Fi **path validation, staging, and missing-file logic**; this chunk goes deeper on **routes, authentication, transport, upload resource limits, and session lifecycle** and references A2.4 rather than repeating it. No application file was modified; `docs/AUDIT.md` is the only write target. No real file under `~/.config/SyncUp/` was read, listed, copied, or opened.

## A4.1 Views inventory

`views/__init__.py` is empty (package marker only) and is omitted. Flet layout boilerplate is not listed.

| File | Responsibility (one line) | User-visible features exposed |
|---|---|---|
| `views/common.py` | Shared UI helpers: log-level colours, folder opener, snackbars, job button state, progress-callback bridge, end-of-run scoreboard dialog. | Colour-coded activity log; “Open Destination Folder”; error snackbars; Cancel button state; run-summary dialog with timings, detected/skipped/processed counts, date-source breakdown, duplicate stats, error list. |
| `views/phone_view.py` | USB/MTP tab: device polling, folder browser, multi-folder selection, destination/filter/conflict/sort controls, profile load/save, backup launch. | Device refresh + selection; breadcrumb folder navigation; multi-folder checkboxes; destination picker/opener; photos/videos filter; Keep Both vs Overwrite; Day/Month/Year/Flat sorting; Start/Cancel; completion summary. |
| `views/organizer_view.py` | Takeout / loose-folder tab: input folder or multi-ZIP selection, destination, filter/conflict/sort, optional sidecar preservation, organizer launch. | Browse folder or multiple ZIPs; destination picker/opener; media filter; conflict mode; sorting; “Preserve JSON metadata sidecars” checkbox; Start/Cancel; summary with detected ZIPs/loose files/folders and date sources. |
| `views/dedup_view.py` | Duplicate-finder tab: folder selection, preview gate, move, undo-last-move, cancellation. | Preview-before-move enforcement; duplicate count/group/size preview; “Move Duplicates”; “Undo Last Move”; manifest path in the log; per-mode summary dialogs. |
| `views/wifi_view.py` | Wi-Fi tab: owns server lifecycle, QR/status UI, destination/settings, staging-dir creation/pruning, profile persistence, snapshot rendering, shutdown watchdog. | Start/Stop Wi-Fi server; scannable QR + LAN URL; phone-connected/upload counters; staging path display; destination picker/opener; filter/conflict/sort; Cancel Processing; live “receiving/copying” status; completion summary. |

### A4.1.1 Coupling observations (migration-relevant)

- **Business logic lives in the view layer.** `views/wifi_view.py` owns `create_staging_dir()` and `_prune_stale_directories()` (lines 132–167), the QR helpers (lines 62–129), the Uvicorn start/stop orchestration (lines 709–822), and Wi-Fi profile persistence (lines 403–415). These are service/domain concerns that the Rust port must relocate, not reimplement inside Svelte components.
- **`views/phone_view.py` owns device polling and profile semantics** — a 20-second `auto_scan` loop (lines 450–459), profile read/normalisation (lines 277–305), and profile write on Start (lines 401–413). The engine is only called with a pre-snapshotted tuple.
- **Confirmation and pre-condition policy lives in the views**, not the engines: duplicate move requires a matching preview (`dedup_view.py:254–264`), and phone/organizer Start refuse when inputs are missing (`phone_view.py:383–389`, `organizer_view.py:205–207`). A Rust backend must enforce these invariants server-side; the SPEC forbids trusting frontend validation.
- **`views/common.py` contains platform branches** (`open_folder`, lines 84–103) and the scoreboard result-shape assumptions. It is the only view module covered by tests (`tests/test_common.py`), and only for helpers.

## A4.2 Wi-Fi server audit (routes, auth, session, upload limits)

> A2.4.3 already documented the endpoint list and `safe_relative_path` rules. This section adds the auth/transport/session/limit analysis and route-level test evidence.

### A4.2.1 Route inventory (auth column is the key addition)

| Method | Path | Purpose | Auth / token required | Notes |
|---|---|---|---|---|
| GET | `/` | Serve the mobile HTML page | **None** | `Cache-Control: no-store`; no CSP header. |
| GET | `/favicon.ico` | Silence favicon requests | **None** | Returns 204. |
| POST | `/manifest` | Diff the phone’s file list against SQLite state and open an upload session | **None** | 409 only while `session.is_processing()`; any client may otherwise reset the session. |
| POST | `/upload` | Stream one file into staging | **None** (only “was this path in the last manifest?”) | 409 if processing; 400/507 for path/size/space failures; 500 on write failure. |
| POST | `/complete` | Start the PC-side backup job | **None** | 409 if another Flet job is running; 400 if nothing was uploaded; otherwise spawns the engine thread. |
| GET | `/status` | Phone polling for stage/counters/current file | **None** | Leaks current filename and error text to any LAN client. |

All routes are synchronous functions (`def`, not `async def`), so FastAPI runs them in its worker pool; this is deliberate for blocking SQLite/disk I/O.

### A4.2.2 Authentication / token / session mechanism

**Absent.** There is no authentication dependency, middleware, header check, bearer token, pairing code, cookie, or per-client session identifier anywhere in `core/wifi_server.py` (repo-wide grep for `Depends`, `middleware`, `Authorization`, `Bearer`, `token`, `CORS`, `TrustedHost` returns nothing in that module).

The app is created with no auth layer and one process-global session:

```python
app = FastAPI(title="SyncUp Wi-Fi Backup", version=VERSION)
telemetry = WiFiTelemetry(progress_queue, post_ui)
session = BackupSession()
```
(`core/wifi_server.py:1000-1002`)

`/manifest` performs no identity check; it immediately rewrites the shared session:

```python
if session.is_processing():
    raise HTTPException(status_code=409, detail="A backup is already being processed.")
ip = client_ip(request)
state = _read_state(device_id)
missing, total_bytes, ignored = compute_missing_files(payload.files, state, media_filter)
...
session.begin_manifest(missing, ip, folders)
```
(`core/wifi_server.py:1096-1118`)

The only upload gate is “this path appeared in the most recent manifest”:

```python
expected = session.expectation(rel)
if expected is None:
    raise HTTPException(status_code=409, detail="This file was not requested by the manifest.")
```
(`core/wifi_server.py:1161-1166`)

Because any client can post its own manifest first, that gate is **self-service**, not authentication. `client_ip` is used only for display/telemetry (`client_ip()` at lines 1013–1017); it is never used for authorisation. `ManifestRequest.device_id` is accepted (`wifi_models.py:59-63`) but **ignored** — the route uses the `create_app(device_id=...)` closure, which is always `"wifi-backup"` from `views/wifi_view.py:45`. Uvicorn access logging is disabled (`access_log=False`, line 1348), so there is no request audit trail.

`BackupSession` has no session id, token, creation time, last-activity time, or expiry. Its `_pending` / `_expectations` sets are plain process-global state (`wifi_server.py:774-958`).

### A4.2.3 Transport encryption

**Absent — the server is plain HTTP.** `WiFiServer` defaults to `host="0.0.0.0"` (line 1323) and passes no TLS material:

```python
config = uvicorn.Config(
    app=app,
    host=self.host,
    port=self.port,
    log_level=self.log_level,
    access_log=False,
    lifespan="on",
)
```
(`core/wifi_server.py:1343-1350`)

No `ssl_keyfile`, `ssl_certfile`, or `https` appears anywhere in the module. The QR payload is explicitly `http://`:

```python
url = f"http://{address}:{server.port}"
```
(`views/wifi_view.py:464`)

There is no Host-header validation, no Origin/Referer check, no HSTS, and no CSP on the mobile page. The server is reachable by every device on the LAN, including a browser on the same machine via `127.0.0.1`.

### A4.2.4 Upload handling

- **Destination directory:** a per-server-run staging directory created by `views/wifi_view.py:150-167` under `tempfile.gettempdir()/syncup-wifi/wifi-<8hex>` (or `SYNCUP_WIFI_STAGING_DIR`), with mode `0o700`. The final destination is a separate `dest_root` supplied to `create_app`.
- **Filename sanitisation:** `safe_relative_path()` (`core/wifi_server.py:650-674`) normalises `\` to `/`, rejects empty/oversize/NUL/absolute/`~`/`..`/dot-leading/`:`/partial names, then the upload route resolves `(staging / parts)` and requires `is_relative_to(staging_root)` (`:1183-1186`). **A2.4.3 already documents these rules; not repeated here.** Note the rules are POSIX-oriented: Windows reserved device names (`CON`, `NUL`, `COM1`, trailing dot/space) are not rejected.
- **Size checks that exist:** declared `size` must equal the manifest size (`:1170-1174`); `UploadFile.size`, when present, must equal the declared size (`:1176-1181`); `copy_stream_atomic` verifies the written byte count equals `expected_size` (`core/fileops.py:141-144`); a free-space check compares the **declared** size against staging free space (`:1188-1196`).
- **Size limits that are absent:** no maximum upload size, no maximum total session size, no maximum number of manifest entries (`ManifestRequest.files` is an unbounded `list`, `wifi_models.py:64-67`), no request-body limit, no rate limit, no concurrent-upload cap. The only bound is the declared size and current free disk.
- **Over-declared stream hazard:** `copy_stream_atomic` writes until EOF **before** comparing the byte count (`core/fileops.py:128-144`). A client that declares a small size can therefore stream far more bytes to disk; the mismatch is detected only after the write, and the temp file is then deleted. The free-space check used the declared size, not the streamed size.
- **Repeat uploads are allowed:** `expectation()` reads `_expectations`, and `note_upload()` only discards from `_pending` (`wifi_server.py:824-836`). A retry after success returns `False` but still overwrites the staged copy (`:1219-1229`). There is no per-file or per-session upload-count cap.
- **Integrity:** content is **not hashed**; verification is size-only. The staged file is later processed by `run_wifi_backup`, which publishes through the atomic temp/verify/replace pipeline (A2.3).
- **Cleanup:** the engine’s `finally` does `shutil.rmtree(staging_root, ignore_errors=True)` (`wifi_server.py:1066-1073`); the view also removes staging on a clean stop when no job is active (`views/wifi_view.py:691-697`). `create_staging_dir()` prunes directories named `wifi-*`/`syncup-wifi-*` older than 24 hours **at the next server start** (`views/wifi_view.py:132-147`). There is no startup/startup-crash recovery for interrupted uploads beyond that 24-hour prune.

### A4.2.5 Manifest-comparison flow

1. Phone posts `{device_id, files:[{rel_path,size,mtime_ns}]}` to `POST /manifest`.
2. Route rejects with 409 only if `session.is_processing()`.
3. `_read_state(device_id)` opens a **fresh `StateStore` per request**, reads the full `processed` table, closes it (`wifi_server.py:687-700`). This also re-imports legacy `state_*.json` on every request (A2.2/A3.5).
4. `compute_missing_files()` (`:703-766`) drops unsafe/hidden/duplicate entries and media-filter misses, returns `new` for absent rows, `modified` for size mismatch or mtime delta > 2 s, and **trusts `size IS NULL` legacy rows as already backed up**. `total_bytes` sums the missing entries.
5. `session.begin_manifest()` stores `_pending` (paths to accept) and `_expectations` (path → `(size, mtime_ns)`), resets counters, and sets stage `uploading`. It derives `selected_folders` from the **first path component only when the path has more than one component** (`:1110-1117`), so root-level uploads rely on the engine’s staging-root fallback (A2.3).
6. Response is `{missing:[...], total_bytes}`. The phone uploads each missing path, then calls `/complete`.

`device_id` from the request is not used for state selection; every Wi-Fi client shares the `create_app` device id (`"wifi-backup"`).

### A4.2.6 Session lifecycle and timeout behaviour

**Who starts it:** the user clicks “Start Wi-Fi Server” in `views/wifi_view.py:709-799`. That worker calls `create_staging_dir()`, `find_available_port(8555)`, `create_app(...)`, `WiFiServer(port).start(app)`, then `wait_until_ready(5.0)`. The server runs in a daemon Uvicorn thread (`wifi_server.py:1337-1357`).

**State machine (`BackupSession`):** `waiting → uploading` (`begin_manifest`) `→ processing` (`begin_processing`, triggered by `/complete`) `→ done | error` (`finish`), with `fail()` forcing `error`. `begin_processing()` returns `already`, `done`, or `empty`; `/complete` returns 400 on `empty` and spawns the engine only on `started` (`:1252-1277`).

**How it stops:**
- User “Stop Server” sets `cancel_event`, then calls `server.stop()` (`views/wifi_view.py:801-822`); the engine aborts at its next file boundary.
- App close: `_shutdown_watch()` waits on `shutdown_event`, stops the server with a 2 s timeout, and removes staging (`views/wifi_view.py:844-864`).
- `WiFiServer.stop()` sets `should_exit`, joins for 5 s, then sets `force_exit` and joins 1 s more (`wifi_server.py:1388-1404`).

**Timeout behaviour:**
- Server readiness: 5 s (`wait_until_ready`).
- **No server-side session expiry, upload idle timeout, or per-client timeout.**
- Client-side only: uploads retry 3 times with 800 ms × attempt backoff (`wifi_server.py` mobile JS `uploadWithRetry`); `/complete` retries 20 times at 3 s intervals while 409 (`completeWithRetry`); `/status` polls every 1.5 s and gives up after 40 consecutive errors (`pollStatus`).
- `/complete` clears a stale `cancel_event` before starting a new engine run (`:1265-1266`).
- `finish()` and `fail()` clear `_pending`/`_expectations`, so late uploads are rejected only after the job ends.

### A4.2.7 Security gaps (ranked)

**Three biggest gaps**

1. **No authentication or pairing at all, and the manifest gate is self-service.** Any LAN device can `POST /manifest` with its own file list, receive its own `_expectations`, `POST /upload` arbitrary media-extension files up to disk capacity, and `POST /complete` to make the PC publish them into the user’s destination. This is an unauthenticated remote write primitive, not just an “unrestricted endpoint”.
2. **Plain HTTP on `0.0.0.0` with no Host/Origin validation.** Traffic (paths, filenames, metadata) is readable on the LAN; a malicious web page can reach `127.0.0.1:8555` (DNS rebinding), and `/upload` (multipart) plus `/complete` (bodyless POST) are “simple” cross-origin requests, so browser-side CSRF-style triggering is possible. There is no TLS, no CSP, and no HSTS.
3. **No resource limits or session expiry.** The app enforces no maximum upload size, total bytes, manifest length, upload count, or rate. `copy_stream_atomic` writes to EOF before the size check, so an over-declared stream can transiently fill the disk; a huge manifest can consume memory; repeated `/manifest` calls reset the single global session and orphan staged files. There is no idle/absolute timeout.

**Full gap register**

| ID | Gap | Evidence | Impact |
|---|---|---|---|
| W-01 | No auth/token/pairing; one global `BackupSession` | `wifi_server.py:1000-1002`, `1096-1118` | Any LAN client can start/steer a backup. |
| W-02 | No TLS; `http://` QR; binds `0.0.0.0` | `wifi_server.py:1323`, `1343-1350`; `wifi_view.py:464` | LAN eavesdropping; no confidentiality. |
| W-03 | No Host/Origin/CORS validation; no CSP/HSTS | no middleware in `create_app`; index only sets `Cache-Control` | DNS rebinding / CSRF-style triggering. |
| W-04 | No upload/body/manifest size or rate limits | `wifi_models.py:64-67`; `fileops.py:128-144`; upload route | Disk-fill / memory DoS; transient over-write. |
| W-05 | No session expiry or idle timeout | `BackupSession` has no time fields | Server stays actionable indefinitely. |
| W-06 | Single global session; any client can reset it | `begin_manifest` unconditionally resets; only processing blocks it | Cross-client interference; last client wins. |
| W-07 | `device_id` ignored; all phones share state | `wifi_models.py:59-63`; route uses closure `device_id` | Wrong-device skips; state namespace collision (A2.2/A3 R3). |
| W-08 | `/status` leaks current filename and error text | `StatusResponse.current_file`, `error` | Information disclosure to any LAN client. |
| W-09 | No request/access audit log | `access_log=False` | No forensic trail. |
| W-10 | POSIX-only path rules | `safe_relative_path` rejects `:` but not Windows reserved names/trailing dot-space | Cross-platform path hazard for v2. |
| W-11 | No content hash | size-only verification | Integrity depends on transport + size. |
| W-12 | Interrupted-upload cleanup only at next start, 24 h cutoff | `wifi_view.py:132-147` | Stale staging debris can persist. |

This is the clearest conflict with SPEC Layer 2 §5B and the standing rule “Never expose an unrestricted upload endpoint.” v2 must not reproduce W-01–W-06.

## A4.3 Platform-specific code

No `platform.system()` call exists anywhere in the repo. Platform branching uses `sys.platform` and `os.name` only. Repo-wide search found these sites:

| Site | Branch | What it does | Validated? |
|---|---|---|---|
| `main.py:32-48` (`_play_ding`) | `os.name == "nt"` → `winsound.Beep`; `sys.platform == "darwin"` → `afplay`; else → `canberra-gtk-play` | Completion sound in a daemon thread; all failures swallowed. | Linux path only (per A1.4/README). Windows/macOS branches exist but are unvalidated. |
| `views/common.py:83-104` (`open_folder`) | `sys.platform.startswith("darwin")` → `open`; `os.name == "nt"` → `os.startfile`; else → `xdg-open` | Opens the destination in the file manager without blocking. | Linux path only; `tests/test_common.py:84-107` mocks `sys.platform="linux"` and `xdg-open`. |
| `core/storage.py:25-35` (`get_app_data_dir`) | `os.name == "nt"` → `%APPDATA%`; `sys.platform == "darwin"` → `~/Library/Application Support`; else `$XDG_CONFIG_HOME`/`~/.config` | Chooses the config directory and creates it. | Linux path only. |
| `core/device.py:1-128` | None — hard-coded Linux | `gio mount`, `os.getuid()`, `getpass.getuser()`, `/run/user/<uid>/gvfs`, `/media/<user>`, `/run/media/<user>`, names containing `mtp`. | Linux only; **no Windows/macOS implementation**. |
| `build.sh` / `SyncUp.spec` | Linux packaging | Requires `./appimagetool`, builds `AppDir`, bundles the Flet desktop runtime, emits `dist/SyncUp_v<VERSION>.AppImage`. | Linux only; no MSI/NSIS/.app/.dmg. |

### What is Linux-only today

1. **USB/MTP device discovery and folder browsing** (`core/device.py`) — GVFS/GIO and Linux mount paths. This is the largest platform gap; SPEC requires Windows/macOS investigation and a separate platform layer.
2. **Packaging and release output** — AppImage only (`build.sh`, `SyncUp.spec`); no Windows or macOS artifacts.
3. **Default completion sound** — `canberra-gtk-play` on the non-Windows/non-macOS branch; optional and failure-tolerant.
4. **Folder opener default** — `xdg-open` on the non-Windows/non-macOS branch.
5. **CI** — `.github/workflows/ci.yml` runs only `ubuntu-latest` (Python 3.10/3.12); no Windows/macOS runners.

### Branches that exist but are unvalidated

- Windows/macOS config directory selection (`core/storage.py:27-31`).
- Windows `os.startfile` and macOS `open` folder opening (`views/common.py:84-92`).
- Windows `winsound` and macOS `afplay` completion sound (`main.py:33-40`).
- `os.replace()` atomic publish and `shutil.move` duplicate moves on Windows/macOS filesystems (case-insensitive, open-file, antivirus, cross-volume semantics) — **no tests**.

### Cross-platform hazards for the Rust port

- **State-key path form diverges by workflow:** MTP rows store `str(relative_to(...))` (OS-native separators) while Wi-Fi rows store `as_posix()` (A3.1). A consolidated Rust schema needs one canonical form and a migration rule.
- **`safe_relative_path` is POSIX-centric:** `:` is rejected (good for drive letters/ADS), but Windows reserved device names and trailing dot/space are not.
- **Case-insensitive filesystems** make `unique_path()` collision probing and duplicate-name handling behave differently on Windows/macOS; untested.
- **No cross-platform CI or device tests**; Linux success must not be reported as Windows/macOS support (standing rule).

## A4.4 Test baseline

### Command actually used

The project does **not** use pytest: `pytest.ini`, `tox.ini`, and `setup.cfg` are absent, and `pytest` is **not installed** in the project venv. README (`## Tests`) and CI (`.github/workflows/ci.yml:36-37`) define the command as:

```
python -m unittest discover -s tests -t . -v
```

`python` is not on `PATH` in this environment; `python3` is, and the project venv (`.venv/bin/python` → `/usr/bin/python3`, Python 3.12.3) contains all runtime dependencies (`flet`, `fastapi`, `uvicorn`, `pydantic`, `multipart`, `qrcode`). The baseline was therefore run with the project’s venv interpreter, preserving `pipefail` and `tee` exactly as required:

```
set -o pipefail
.venv/bin/python -m unittest discover -s tests -t . -v 2>&1 \
  | tee /tmp/syncup-pytest-baseline.log | tail -40
```

Nothing was installed. The pipeline completed with the shell reporting success (exit status 0).

### Baseline summary — verbatim from the full log

Full log: `/tmp/syncup-pytest-baseline.log` (110 lines). Extracted with `grep -n -E "^(Ran |OK|FAILED|ERROR)|passed|failed|error[s]?="` over the **entire** log, not the tail:

```
Ran 105 tests in 8.752s

OK
```

`grep -c "\.\.\. ok$"` on the full log returns `105`. The README claim of “105 unit tests” is accurate at this commit.

### Test inventory (11 files, 105 tests)

| File | Count | Test function names |
|---|---:|---|
| `tests/test_cli.py` | 3 | `test_placeholder_tokens_are_removed`, `test_real_paths_are_preserved`, `test_case_insensitive_and_empty_input` |
| `tests/test_common.py` | 11 | `test_imports_do_not_start_the_app`, `test_card_builds`, `test_job_button_lifecycle`, `test_error_lines_are_capped`, `test_human_size_is_reexported`, `test_level_colors_switch_with_theme`, `test_open_folder_missing_path_calls_error_callback`, `test_open_folder_launches_opener_without_blocking`, `test_open_folder_failure_is_reported_not_raised`, `test_progress_callback_forwards_and_logs_phase_events`, `test_progress_callback_does_not_log_every_file` |
| `tests/test_dedup.py` | 9 | `test_preview_does_not_move_files`, `test_move_creates_manifest_and_undo_restores`, `test_manifest_is_written_before_moves`, `test_no_duplicates`, `test_cancel_during_hashing`, `test_undo_malformed_entry_is_reported`, `test_undo_without_manifest_reports_error`, `test_duplicate_folder_is_not_rescanned`, `test_undo_does_not_overwrite_an_existing_original` |
| `tests/test_engine.py` | 14 | `test_basic_backup`, `test_second_run_skips_already_backed_up_file`, `test_keep_both_when_source_changes`, `test_overwrite_replaces_only_after_verified_copy`, `test_overwrite_does_not_lose_files_with_colliding_targets`, `test_same_size_change_is_detected_by_mtime`, `test_overlapping_selected_folders_copy_each_file_once`, `test_media_filter`, `test_cancel_before_copy`, `test_legacy_state_is_respected`, `test_destination_inside_phone_is_rejected`, `test_missing_phone_is_reported`, `test_missing_folders_is_reported`, `test_sort_modes` |
| `tests/test_fileops.py` | 8 | `test_copy_file_atomic_copies_content_and_mtime`, `test_copy_file_atomic_overwrites_only_after_success`, `test_failed_copy_preserves_destination_and_cleans_temp`, `test_copy_stream_atomic_verifies_size`, `test_unique_path`, `test_atomic_write_json`, `test_is_same_file_uses_size_and_mtime`, `test_human_size` |
| `tests/test_naming.py` | 14 | `test_parses_compact_date_and_time`, `test_parses_dashed_time`, `test_parses_dotted_time`, `test_whatsapp_style_name_has_no_fake_time`, `test_repairs_old_midnight_prefix`, `test_no_date_returns_none`, `test_invalid_date_returns_none`, `test_strip_multiple_generated_prefixes`, `test_target_name_is_idempotent`, `test_target_path_modes`, `test_date_for_path_falls_back_to_mtime`, `test_exif_datetime_is_read_and_counted`, `test_filename_still_wins_over_exif`, `test_date_with_source_falls_back_to_mtime` |
| `tests/test_packaging.py` | 6 | `test_icon_is_a_real_png`, `test_desktop_file_points_at_the_icon`, `test_desktop_file_has_a_single_main_category`, `test_appstream_metadata_exists_and_is_built`, `test_spec_includes_icon_and_flet_data`, `test_version_is_consistent` |
| `tests/test_storage.py` | 8 | `test_settings_defaults_and_merge`, `test_profile_round_trip`, `test_state_round_trip`, `test_state_is_persistent_across_instances`, `test_legacy_json_state_is_migrated_without_deleting_it`, `test_save_state_accepts_a_set`, `test_locked_database_is_not_treated_as_corrupt`, `test_app_data_dir_is_created` |
| `tests/test_takeout.py` | 13 | `test_loose_file_uses_sidecar_timestamp_and_preserves_metadata`, `test_supplemental_metadata_sidecar_is_found`, `test_zip_supplemental_metadata_sidecar_is_found`, `test_rerun_is_idempotent`, `test_zip_is_streamed_and_sidecar_is_preserved`, `test_overwrite_does_not_lose_files_with_colliding_targets`, `test_metadata_can_be_disabled`, `test_input_folder_inside_destination_is_processed`, `test_invalid_zip_is_reported_without_crashing`, `test_sidecar_failure_is_counted`, `test_no_matching_media`, `test_cancel_before_processing`, `test_bad_json_is_ignored` |
| `tests/test_telemetry.py` | 6 | `test_event_shape_and_percentage`, `test_legacy_three_argument_callback_still_works`, `test_indeterminate_defaults_for_zero_totals`, `test_backup_emits_phases_and_additive_result_fields`, `test_takeout_emits_zip_counts_and_date_sources`, `test_dedup_logs_exact_duplicate_bytes_before_moving` |
| `tests/test_wifi_server.py` | 13 | `test_accepts_normal_relative_path`, `test_rejects_traversal`, `test_rejects_absolute_and_drive`, `test_rejects_hidden_and_partial`, `test_diffs_manifest_against_state`, `test_legacy_size_none_row_is_trusted`, `test_mtime_tolerance`, `test_media_filter_and_unsafe_ignored`, `test_duplicates_ignored`, `test_moves_file_from_staging_to_dest`, `test_second_run_skips`, `test_flat_staging_fallback`, `test_missing_staging_returns_error` |

### Privacy note for the run

Every test that touches settings/profiles/state patches `XDG_CONFIG_HOME` to a `tempfile.TemporaryDirectory` (`tests/test_storage.py:27-35`, `test_engine.py:18-23`, `test_takeout.py:18-23`, `test_wifi_server.py:106-114`, `test_telemetry.py:64-72`). `tests/test_dedup.py`, `test_naming.py`, `test_fileops.py`, `test_cli.py`, and `test_packaging.py` use temp dirs or pure functions. No test read or wrote the real `~/.config/SyncUp/`. `tests/fixtures/legacy/` currently contains only a README stating the directory is for synthetic/redacted fixtures.

### Coverage gaps exposed by the inventory

- **No HTTP route tests.** There is no `TestClient` usage anywhere; `/manifest`, `/upload`, `/complete`, `/status`, and the mobile page are untested at the HTTP layer (`grep` for `TestClient|/manifest|/upload|/complete|/status|create_app` in `tests/` returns nothing).
- **No `BackupSession` or `WiFiServer` lifecycle tests** — state transitions, 409/400 branches, port scanning, readiness, stop/force-exit, and staging cleanup are untested.
- **No QR / LAN-IP helper tests** — `generate_qr_svg`, `qr_image_source`, `get_local_ip`, `is_lan_address`, `create_staging_dir`, `_prune_stale_directories` are untested.
- **No `core/device.py` tests** — MTP discovery and folder listing have no coverage at all.
- **No view-behaviour tests** beyond `views/common.py` helpers; phone/organizer/dedup/wifi view flows are untested.
- **No `main.py`, `core/logging_setup.py`, or `core/cli` integration tests** beyond argv sanitisation.
- **No Windows/macOS tests** and no real-device, real-phone, or real-LAN tests.
- **No security tests** because auth/TLS/expiry do not exist.

Lint (`ruff check .`) and the packaged build (`build.sh`) were **not run** in this chunk; A4’s required verification was the test baseline. CI defines both, so Phase 1 should run them as its first gate.

## A4.5 Feature inventory (final table)

Evidence categories: **Automated** = covered by a named legacy test; **Manual** = not run in this audit unless stated; **Unknown** = cannot be determined without hardware/user data.

| Feature | Files | Evidence it works | Notes |
|---|---|---|---|
| USB/MTP phone backup | `core/device.py`, `core/engine.py`, `views/phone_view.py` | Automated: `tests/test_engine.py` (14 tests) with a synthetic phone tree covers copy, skip, modified, conflict modes, overlapping folders, filters, cancel, legacy state, destination guard, missing inputs, sort modes. Device discovery itself has **no tests**. Manual: not run (no device). | Linux-only GVFS/GIO discovery; real MTP transfer, lazy mount, and disconnect handling are **unverified** here. |
| Wi-Fi backup | `core/wifi_server.py`, `core/wifi_models.py`, `core/wifi_telemetry.py`, `core/engine.py` (`run_wifi_backup`), `views/wifi_view.py` | Automated: `tests/test_wifi_server.py` (13 tests) covers path validation, manifest diff, legacy rows, mtime tolerance, filters, duplicates, staging→dest move, second-run skip, flat fallback, missing staging. HTTP routes, session lifecycle, QR, server start/stop, and real-phone flow are **untested**. Manual: not run. | Plain HTTP, no auth/TLS/expiry (A4.2); all clients share `"wifi-backup"` state. |
| Google Takeout / loose-folder organizer | `core/takeout_engine.py`, `views/organizer_view.py` | Automated: `tests/test_takeout.py` (13 tests) covers sidecar timestamps, supplemental metadata, ZIP streaming, idempotent re-run, sidecar preservation, overwrite collisions, metadata off, input-inside-destination, invalid ZIP, sidecar failure, no media, cancel, bad JSON. Manual: not run with a real Takeout archive. | ZIP members fall back to member timestamp (no EXIF); no ZIP-bomb/expansion limits; no EXIF write-back. |
| Duplicate finder | `core/dedup_engine.py`, `views/dedup_view.py` | Automated: `tests/test_dedup.py` (9 tests) covers preview-no-move, manifest-before-move, undo, no duplicates, cancel, malformed manifest, missing manifest, `_Duplicates_` exclusion, no-overwrite undo. Manual: not run on a large library. | Moves, never deletes; undo trusts manifest paths; cross-filesystem move is not atomic. |
| Settings | `core/storage.py`, `core/constants.py`, `main.py` | Automated: `tests/test_storage.py::test_settings_defaults_and_merge`; atomic JSON write via `tests/test_fileops.py::test_atomic_write_json`. Manual: not run. | Global `settings.json`; unknown keys preserved; only `theme`, `window_width`, `window_height` are consumed. |
| Device profiles | `core/storage.py`, `views/phone_view.py`, `views/wifi_view.py` | Automated: `tests/test_storage.py::test_profile_round_trip`. View-level folder/selection persistence is **untested**. Manual: not run. | `destination`/`sorting`/`filter`/`conflict`/`folders`; Wi-Fi never writes `folders`; `_safe_id` is lossy (A3 R3). |
| Theming | `main.py` (`toggle_theme`, startup `theme_mode`), `views/common.py` (`level_color`) | Automated: `tests/test_common.py::test_level_colors_switch_with_theme`; theme toggle/persistence is **untested**. Manual: not run. | Only dark/light; a non-`"light"` value maps to dark. **No “System” theme mode**, despite SPEC §E asking for it. |
| Window persistence | `main.py` (`page.window.*`, `on_close`), `core/storage.py` | Automated: only key defaults/merge; `on_close` geometry save is **untested**. Manual: not run. | 1200×950 default, min 1000×700; save is best-effort inside `try/except`. |
| Activity log / progress UI | `main.py` (`log_col`, queues, consumers), `views/common.py` | Automated: `tests/test_common.py` covers job-button lifecycle, error-line cap, progress callback bridging; telemetry shape in `tests/test_telemetry.py`. Manual: not run. | Log capped at 800 rows; latest-progress-wins coalescing; worker/UI queues (A1.2). |
| Completion sound | `main.py` (`_play_ding`/`play_ding`) | No tests. Manual: not run. | Windows `winsound`, macOS `afplay`, Linux `canberra-gtk-play`; failures swallowed; only Linux path validated historically. |
| Packaging | `SyncUp.spec`, `build.sh`, `packaging/`, `assets/` | Automated: `tests/test_packaging.py` (6 tests) checks icon, desktop entry, appdata, spec contents, version consistency. Build itself **not run**. | Linux AppImage only; no Windows/macOS artifacts. |
| Telemetry | `core/telemetry.py`, `core/wifi_telemetry.py` | Automated: `tests/test_telemetry.py` (6 tests) covers event shape, 3-arg compatibility, indeterminate totals, engine/takeout/dedup events. | Schema v1 is a wire marker; no consumer validates `version`; no throttling. |
| Logging | `core/logging_setup.py` | No tests. Manual: not run. | Rotating `syncup.log`, 2 MB × 3, no console handler. |

## A4.6 Top 10 migration risks

Ordered by severity. Each risk is grounded in the evidence above; related A2/A3 IDs are cross-referenced rather than restated.

1. **Wi-Fi is an unauthenticated, plaintext, unrestricted write path (A4.2).** No auth/token/pairing, no TLS, `0.0.0.0`, self-service manifest, no size/rate/expiry limits, single global session. Porting it as-is violates SPEC §5B and the standing “never expose an unrestricted upload endpoint” rule. v2 must implement pairing tokens, short-lived sessions, expiry, Host/Origin checks, request/upload limits, and safe staging before claiming Wi-Fi support.
2. **SQLite migration is WAL-sensitive and has no schema version (A3 R1/R5).** A plain `.db` copy can miss committed rows in `-wal`; a crash mid-migration is indistinguishable from “never migrated”. v2 must use the SQLite backup API (`Connection::backup` / `VACUUM INTO`), add `user_version`/a ledger, make migration idempotent and resumable, and verify the backup.
3. **The tri-state `processed` row contract must survive bit-for-bit (A3 R2).** `size IS NULL` means “trusted forever”; `size`-only means “size match”; both set means “size + 2 s mtime”. Normalising `NULL` to `0`, adding `NOT NULL`, or synthesising `copied_at` will either re-copy whole libraries or falsely mark changed files as backed up. Preserve nanoseconds and exact `NULL`s.
4. **The atomic-publish pipeline is weaker than the stated invariant (A2.3).** `fsync` failures are swallowed, there is no directory fsync after `os.replace`, and verification is size-only. v2 must enforce durable publish (file + parent directory) and, where required, content verification **before** the DB commit, per the safety invariants.
5. **USB/MTP is Linux-only and untested at the device layer (A4.3).** `core/device.py` hard-codes `gio`/GVFS and Linux mount paths; there is no Windows/macOS implementation and no test for discovery. v2 needs a platform abstraction and independent verification; Linux success must not be reported as cross-platform support.
6. **Business logic is embedded in the views (A4.1).** Staging creation/pruning, QR generation, server lifecycle, profile persistence, confirmation gating, and device polling live in `views/`. A Svelte/Rust split must move these into services/domain and re-enforce them in Rust; otherwise invariants will depend on the frontend.
7. **The two backup engines will drift (A2.1/A2.3).** `run_backup` and `run_wifi_backup` are ~450 near-identical lines; the 2-second mtime tolerance is defined in three places; `_matches_filter` and `_format_duration` are duplicated. A single Rust backup pipeline should own conflict, staging, state, and durability semantics.
8. **Device identity is lossy and Wi-Fi state is shared (A3 R3, A4.2).** `_safe_id` can collide, the original ID is unrecoverable, and every Wi-Fi client writes the `"wifi-backup"` namespace. Migration must treat filenames as opaque legacy keys, detect collisions, and isolate per-device/per-session state; otherwise files can be wrongly skipped.
9. **The green baseline does not cover the riskiest surfaces (A4.4).** 105 tests pass, but there are no HTTP route tests, no `BackupSession`/`WiFiServer` tests, no MTP tests, no view-flow tests, and no Windows/macOS tests. v2 needs equivalent Rust tests for file ops, state, migration, organizer, dedup, and Wi-Fi auth/session before parity can be claimed.
10. **Date/EXIF behaviour has documented limits that must be an explicit decision (A2.3/A2.4.1).** EXIF is read only for JPEG APP1/TIFF; HEIC/HEIF/PNG/WebP/RAW and ZIP members are not EXIF-probed; dates are naive local time. Preserve the current precedence for parity, or document each improvement as an intentional deviation with tests.

### A4 open questions / not determinable from source

- **Real MTP device behaviour** (mount timing, disconnect recovery, folder permissions) — no Android device was attached; `core/device.py` has no tests.
- **Real phone/browser behaviour over a LAN** — no phone was used; the mobile page’s `webkitdirectory` support, retry behaviour, and firewall interaction are unverified.
- **Production data shape** (row counts, `NULL`-row prevalence, live WAL state, legacy-JSON-only users) — cannot be known without reading user data, which is forbidden.
- **Windows/macOS behaviour** — code branches exist but were never run; no CI or packaging exists for those platforms.
- **Exact request-body limits imposed by the installed Starlette/Uvicorn versions** — the application defines none, and defaults were not measured.
- **Whether `UploadFile.size` is always populated** by the installed FastAPI/Starlette version — the code treats it as optional (`getattr(file, "size", None)`).
- **Lint and packaged-build status** — `ruff check .` and `build.sh` were not run in A4; only the test baseline was required.
