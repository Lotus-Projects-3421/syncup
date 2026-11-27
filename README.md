# SyncUp V2

**Safe Android photo/video backup and Google Takeout organization for Linux.**

SyncUp V2 is a desktop application built with Tauri 2, Rust, Svelte 5, and TypeScript. It provides local Android media backup, wireless uploads, Google Takeout organization, and reversible duplicate cleanup.

**Current version: 2.0.0**

## Platform support

- Linux desktop, x86_64 release packages.
- Debian package (`.deb`) and AppImage distributions.
- Windows and macOS builds are not currently provided.

The release has been built and launched on Zorin OS 18.1 (Ubuntu 24.04-based).

## Features

### Android phone backup (USB / MTP)

- Discover and browse Android devices through Linux GVFS/GIO.
- Select media for backup.
- Verify copied files using size and BLAKE2b checks.

### Wi-Fi backup

- Pair a phone using a QR code.
- Upload photos and videos through the phone's browser.
- Receive files into a staging area before verification and publication.

**Security limitation:** Wi-Fi transfers use HTTP without TLS encryption. Data is not encrypted in transit. Use this feature only on a trusted local network. Do not use it on public or untrusted Wi-Fi networks.

### Google Takeout Organizer

- Process Google Takeout ZIP archives.
- Organize media by date.
- Use available date information from Takeout metadata, EXIF data, and filenames.
- Preserve source archives while processing.

### Duplicate Finder

- Identify duplicate files using size and BLAKE2b hashing.
- Move selected duplicates into quarantine rather than permanently deleting them.
- Maintain manifests to support restoration through Undo.

### Legacy data import

- Import supported settings and backup history from the original SyncUp application.
- The original Python/Flet v1.4.1 source is retained in the repository for legacy data migration, compatibility, and historical reference. It is not the V2 application runtime.

## Installation

### Debian / Ubuntu-based distributions

Download the `SyncUp V2` `.deb` package, then run this command from the directory containing the downloaded file:

```bash
sudo apt install "./SyncUp V2_2.0.0_amd64.deb"
```

### AppImage

Download the AppImage, then run:

```bash
chmod +x "SyncUp V2_2.0.0_amd64.AppImage"
./SyncUp\ V2_2.0.0_amd64.AppImage
```

The AppImage and Debian package are separate distribution formats. Use the one appropriate for your system.

## Development setup

### Requirements

- Rust toolchain (rustup).
- Node.js 24.
- npm.
- Linux development libraries required by Tauri 2 and WebKitGTK.
- GVFS/GIO support for USB MTP device access.

On Ubuntu 24.04-based distributions, development dependencies include:

```bash
sudo apt update
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  build-essential curl wget file \
  libxdo-dev libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev libglib2.0-bin \
  pkg-config git
```

### Clone and install

```bash
git clone https://github.com/Lotus-Projects-3421/syncup.git
cd syncup
npm ci
```

### Development mode

```bash
npm run tauri dev
```

### Checks

```bash
npm run check
npm run build
npm run specta:check

cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

### Build release packages

```bash
npm run tauri -- build --bundles deb,appimage
```

Build outputs are placed under:

```text
src-tauri/target/release/bundle/
```

## Architecture

- `src/` — Svelte 5 frontend.
- `src-tauri/` — Rust and Tauri backend.
- `src-tauri/src/commands/` — frontend-facing commands.
- `src-tauri/src/services/` — backup, Wi-Fi, organizer, duplicate detection, and file operations.
- `src-tauri/src/platform/` — platform-specific functionality.
- `docs/` — architecture, specifications, audit, plans, and operational documentation.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the system design.

## Data safety

SyncUp is designed around the following principles:

1. Verify newly copied data before publishing it.
2. Avoid silently overwriting existing files.
3. Use staging and atomic publication where supported.
4. Keep duplicate cleanup reversible through quarantine manifests.
5. Preserve source data during Takeout organization and legacy import.

These safeguards reduce risk but do not replace maintaining independent backups of important files.

## Known limitations

- Linux only; no Windows or macOS release.
- Wi-Fi transfers use unencrypted HTTP.
- Interrupted Wi-Fi uploads may need to be restarted.
- Metadata extraction depends on supported formats and available metadata.
- Cross-filesystem moves cannot guarantee the same atomic rename behavior as same-filesystem operations.
- Installation and launch were verified on the development machine, not on a separate clean Linux installation.

## License

SyncUp V2 is distributed under the MIT License. See [LICENSE](LICENSE).
