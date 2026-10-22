# SyncUp

Safe Android photo/video backup and Google Takeout organization for Linux.

SyncUp is built around one rule: **never replace or remove a destination file
until the new copy has been written and verified**. It is a desktop app written
in Python with Flet, with separate tabs for phone backup, Takeout organization,
and duplicate cleanup.

## What is new in v1.3.0

- SQLite backup state replaces the old whole-file JSON state. Progress is
  committed in small transactions, and legacy JSON files are migrated without
  being deleted.
- Copies are atomic and crash-safe: data is written to a destination-local
  `.part-*` file, verified, flushed with `fsync`, and published with
  `os.replace()`. The old destination is never unlinked first.
- Google Takeout ZIP archives are streamed member-by-member. SyncUp does not
  call `extractall()` or create a second full extraction of the archive.
- The duplicate finder supports preview before moving, a manifest written
  before any move, and an undo action that can recover a partial move.
- The test suite contains **74 unit tests** and uses only the Python standard
  library `unittest`.
- Packaging now includes AppStream metadata, a reverse-DNS desktop entry, a
  hicolor application icon, and a bundled Flet desktop runtime.
- Worker-thread UI updates are marshalled through a queue so Flet controls are
  updated only on the UI thread.

## Workflows

### Phone Backup (MTP)

- Discovers Android devices exposed by `gvfs`/`gio`, with manual refresh.
- Browses phone folders in the app and supports multi-folder selection.
- Remembers destination, sorting, media filter, conflict mode, and folders per
  device.
- Detects changed files by path, size, and modification time. A replaced file
  with the same name and size is copied again.
- Deduplicates overlapping selected folders, so a parent and child folder do
  not cause the same source file to be copied twice.
- Checks destination free space before copying.
- Stops after repeated errors instead of repeatedly retrying a disconnected
  phone.
- Reports copied, skipped, and error counts truthfully.

### Special Organizer (Google Takeout and folders)

- Streams ZIP members directly to their destinations.
- Uses Google Takeout sidecars (`photoTakenTime` / `creationTime`) when
  available, then filename dates, then file modification times.
- Supports `.json` and `.supplemental-metadata.json` sidecars.
- Optionally preserves metadata sidecars next to organized media.
- Supports day, month, year, and flat layouts.
- Re-running is idempotent: matching destinations are skipped rather than
  producing `__1`, `__2`, and so on.
- Supports keep-both and overwrite conflict modes. Overwrite only replaces a
  file from a previous run; collisions within one run use unique names.
- Inputs inside the destination are processed without recursively scanning the
  destination itself.

### Duplicate Finder

- Uses a quick size plus head/tail hash check before a full BLAKE2b content
  hash.
- Shows a preview before anything is moved.
- Keeps the shallowest copy and moves duplicates into
  `_Duplicates_<timestamp>/`.
- Writes `_manifest.json` before the first move.
- Provides **Undo Last Move**, which restores a completed or partially
  completed move without overwriting an existing original.
- Reports unreadable files as errors instead of aborting the whole scan.

## Installation

### Run from source

Requirements:

- Python 3.10 or newer.
- A Linux desktop with `gio`/`gvfs` for MTP access.
- `flet[desktop]==1.0.4` from `requirements.txt`.

Create a virtual environment and install the runtime dependencies:

    git clone https://github.com/Lotus-Projects-3421/syncup.git
    cd syncup
    python3 -m venv .venv
    source .venv/bin/activate
    python -m pip install -r requirements.txt
    python main.py

### Run the built AppImage

The v1.3.0 build output is `dist/SyncUp_v1.3.0.AppImage`:

    chmod +x dist/SyncUp_v1.3.0.AppImage
    ./dist/SyncUp_v1.3.0.AppImage

The AppImage bundles the Flet desktop runtime when it is available in the
build cache, so first launch does not require a network download.

## Building the AppImage

Install the build dependencies and run the versioned build script:

    python -m pip install -r requirements-build.txt
    ./build.sh

`build.sh` uses `.venv/bin/python` when it exists, or the `PYTHON` environment
variable. It runs the test suite, regenerates the icon, builds with
PyInstaller, assembles `AppDir`, and writes
`dist/SyncUp_v<version>.AppImage`.

The build requires an executable `appimagetool` in the project root. It also
prefers a cached Flet runtime at
`~/.flet/client/flet-desktop-light-<flet-version>/flet` and bundles it into the
AppImage.

The packaging files are:

- `packaging/io.github.lotus_projects_3421.SyncUp.desktop`
- `packaging/io.github.lotus_projects_3421.SyncUp.appdata.xml`
- `assets/syncup.png`

The desktop filename, AppStream `<id>`, and AppStream filename use the same
reverse-DNS name. The build installs the desktop entry, AppStream metadata, and
hicolor icon under `AppDir/usr/share`, and `appimagetool` validates the
AppStream document with `appstreamcli`.

## Tests

Run the full standard-library test suite:

    python -m unittest discover -s tests -t . -v

As of v1.3.0 there are **74 tests**, covering:

- date parsing, idempotent naming, and generated-prefix removal;
- atomic copy verification, failure cleanup, and JSON writes;
- settings, profiles, SQLite state, and legacy JSON migration;
- phone backup skips, changed-file detection, filters, cancellation, overlap,
  and error paths;
- Takeout ZIP streaming, sidecars, metadata, conflicts, and idempotency;
- duplicate preview, BLAKE2b hashing, manifest, undo, and cancellation;
- packaging icon, AppStream, version, and desktop-file consistency.

## Project layout

    SyncUp/
    ├── main.py
    ├── core/
    │   ├── constants.py
    │   ├── device.py
    │   ├── fileops.py
    │   ├── engine.py
    │   ├── logging_setup.py
    │   ├── naming.py
    │   ├── storage.py
    │   ├── takeout_engine.py
    │   └── dedup_engine.py
    ├── views/
    │   ├── common.py
    │   ├── phone_view.py
    │   ├── organizer_view.py
    │   └── dedup_view.py
    ├── tests/
    ├── packaging/
    ├── docs/
    ├── .github/
    ├── assets/
    ├── scripts/
    ├── build.sh
    ├── SyncUp.spec
    ├── pyproject.toml
    ├── requirements.txt
    ├── requirements-build.txt
    ├── requirements-dev.txt
    ├── README.md
    ├── CHANGELOG.md
    ├── LICENSE
    └── .gitignore

`archive/` is a local-only, gitignored folder for old AppImages, personal notes,
and local profile data. It is not part of the public project.

## Data locations

SyncUp stores its own data under the platform configuration directory. On
Linux:

| Path | Contents |
| --- | --- |
| `~/.config/SyncUp/settings.json` | Theme and window size |
| `~/.config/SyncUp/profile_<device>.json` | Per-device backup profile |
| `~/.config/SyncUp/state_<device>.db` | SQLite processed-file state |
| `~/.config/SyncUp/state_<device>.json` | Legacy state, read only for migration |
| `~/.config/SyncUp/logs/syncup.log` | Rotating application log |

Old JSON state files are imported into SQLite and are not deleted. Duplicate
manifests live beside the moved files inside `_Duplicates_*`.

## Safety model

- Destination-local temporary files prevent partial files from appearing at the
  final path.
- A copy is size-checked and flushed before `os.replace()` publishes it.
- No code path unlinks a media source. Duplicate cleanup moves files aside
  instead of deleting them.
- SQLite uses WAL mode and commits progress incrementally. A locked database is
  treated as a lock error, not as corruption. A genuinely corrupt database is
  preserved as a `.corrupt-<timestamp>` backup before a fresh database is
  created.
- Duplicate manifests are written before moves so a crash can still be
  diagnosed and undone.
- ZIP processing uses `zipfile` streams and never calls `extractall()`.
- Worker threads communicate with the Flet UI through `post_ui()` and queues.
- Errors are counted and reported; the app does not claim success for files it
  could not process.

## Limitations

- MTP support is Linux-specific and depends on `gvfs`; the phone must be
  unlocked and in file-transfer mode.
- Windows and macOS builds are not currently produced or tested.
- The organizer reads dates from sidecars, filenames, and file timestamps; it
  does not rewrite EXIF timestamps. Existing EXIF data is preserved.
- Overwrite uses an atomic replacement, but the previous file is gone after a
  successful replacement. Use keep-both to retain both versions.
- Duplicate cleanup is a reversible move operation, not a permanent deletion
  tool.

## Architecture and contributing

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the safety invariants, data
flow, SQLite schema, and threading model. The CI workflow is in
`.github/workflows/ci.yml`; it runs compilation and the full unittest suite on
Python 3.10 and 3.12, plus Ruff linting.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
