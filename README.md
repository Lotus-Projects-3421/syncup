# SyncUp — Photo & Video Backup & Organizer

> A modern Python desktop app that replaces clunky terminal scripts for backing up Android phones and organizing Google Takeout archives. Built with Flet.

[![Python](https://img.shields.io/badge/Python-3.12-3776AB?logo=python&logoColor=white)](https://www.python.org/)
[![Flet](https://img.shields.io/badge/Flet-1.0.4-00A8E8?logo=flutter&logoColor=white)](https://flet.dev/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Linux-lightgrey)]()

## ✨ Why SyncUp?

Backing up photos from an Android phone on Linux is notoriously painful. SyncUp solves this with a clean, non-blocking GUI, smart memory, and a safe-by-default philosophy: **it will never delete your original files.**

## 🚀 Features

- **Phone Backup (MTP):** Auto-detects Android devices, remembers your choices with profiles, and skips already-backed-up files using smart memory.
- **Google Takeout Organizer:** Processes ZIPs and loose folders, parses JSON/EXIF data to perfectly date and sort your media.
- **Safe Duplicate Finder:** Uses MD5 hashing to find exact duplicates and moves them to a _Duplicates folder, never deletes.
- **Modern UI:** Dark/Light mode, window size memory, async progress updates, and conflict resolution.

## 🧠 Architecture

- `main.py` — UI entry point, tabs, async progress queue.
- `core/` — Heavy lifting: MTP detection, file copying, Takeout parsing, deduplication.
- `views/` — Flet UI code for the three main tabs.

## 📦 Installation & Usage (Linux)

1. Download the latest SyncUp_v1.2.AppImage from the Releases page.
2. Make it executable: chmod +x SyncUp_v1.2.AppImage
3. Double-click to run. No installation needed.

### Building from Source

    git clone https://github.com/YOUR_USERNAME/syncup.git
    cd syncup
    pip install -r requirements.txt
    python main.py

To package as an AppImage:

    pyinstaller --noconfirm --windowed --name SyncUp --collect-data flet main.py
    mkdir -p AppDir/usr/bin && cp -r dist/SyncUp/* AppDir/usr/bin/
    ARCH=x86_64 ./appimagetool --appimage-extract-and-run AppDir SyncUp_v1.2.AppImage

## 🗺️ Roadmap

- [x] v1.2: AppImage packaging, duplicate finder, Takeout organizer.
- [ ] Windows & macOS builds.
- [ ] Settings tab with custom naming templates.
- [ ] History tab.

## 📄 License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.
