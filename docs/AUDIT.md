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
