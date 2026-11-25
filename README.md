# SyncUp v2

Safe Android photo/video backup and Google Takeout organization for Linux.

SyncUp v2 is a complete rewrite of the original Python/Flet application, built with **Tauri 2**, **Rust**, and **Svelte 5**. It prioritizes data safety: no file is ever overwritten without prior verification, duplicates are moved to quarantine (not deleted), and all destructive operations are crash-safe and reversible.

## Status: Pre-Release (Linux Only)

This is **v2.0.0-pre**. It is functional but undergoing final validation.

*   **Platform:** Linux only (tested on Ubuntu/Zorin). Windows/macOS are not supported in v2.0.
*   **Wi-Fi Security:** Uses HTTP on the local LAN. **Not encrypted.** Use only on trusted networks.
*   **Testing:** Core logic is unit-tested. Real-device MTP and Wi-Fi flows require manual validation (see `docs/RELEASE_STATUS.md`).

## Features

*   **Phone Backup (MTP):** Back up photos/videos from Android devices via USB. Uses Linux GVFS/GIO.
*   **Wi-Fi Backup:** Wireless backup via QR code. Phone uploads to a temporary staging area; files are verified and published atomically.
*   **Google Takeout Organizer:** Stream-processes Takeout ZIPs. Organizes by date (Year/Month/Day) using JSON sidecars, EXIF, or filenames.
*   **Duplicate Finder:** Scans for duplicates using size + BLAKE2b hash. Moves duplicates to a `_Duplicates_` quarantine folder with a manifest for safe **Undo**.
*   **Legacy Migration:** Imports settings and backup history from SyncUp v1 (Python) safely.

## Prerequisites

*   **Rust:** 1.100.0+ (`rustup`)
*   **Node.js:** v24.x LTS (`nvm` recommended)
*   **System Libraries (Ubuntu/Debian):**
    ```bash
    sudo apt update
    sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
      libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
      libglib2.0-bin pkg-config git
    ```
    *Note: `libglib2.0-bin` provides `gio`, required for MTP device discovery.*

## Development

Clone the repository and install dependencies:

```bash
git clone https://github.com/Lotus-Projects-3421/syncup.git
cd syncup
npm install
```

Run in development mode (hot-reload):

```bash
npm run tauri dev
```

## Building

To produce a release binary (AppImage, deb):

```bash
npm run tauri build
```

Artifacts will be in `src-tauri/target/release/bundle/`.

## Project Structure

*   `src/`: Svelte 5 frontend (UI, routing, state).
*   `src-tauri/`: Rust backend (IPC, services, storage, platform logic).
*   `src-tauri/src/services/`: Core workflows (backup, wifi, organizer, dedup, fileops).
*   `src-tauri/src/platform/`: OS-specific code (Linux MTP).
*   `docs/`: Architecture, plans, and progress logs.

## Safety Principles

1.  **Atomic Publish:** Files are written to a temporary location, verified (size/hash), and only then moved to the final destination.
2.  **No Silent Overwrites:** Existing files are preserved unless explicit "Overwrite" mode is selected (and even then, via atomic replace).
3.  **Reversible Dedup:** Duplicate cleanup moves files to a quarantine folder with a manifest. Undo restores them.
4.  **Crash Safety:** Database updates happen only after file durability is confirmed. Manifests are fsynced before moves.

## Legacy Support (v1)

The original Python/Flet application is preserved in the repository history and `archive/` (if present). SyncUp v2 can import your existing backup database and settings from v1 via the **Settings > Legacy Data Import** screen.

## License

MIT License. See [LICENSE](LICENSE).
