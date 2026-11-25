# SyncUp v2 Architecture

This document describes the SyncUp v2 implementation (Rust/Tauri/Svelte). For the legacy Python architecture, see the git history or `archive/`.

## Overview

SyncUp v2 is a desktop application built with:
*   **Backend:** Rust (Tauri 2)
*   **Frontend:** Svelte 5 (Runes), TypeScript, Tailwind CSS
*   **IPC:** `tauri-specta` (Type-safe bindings generated from Rust)
*   **Storage:** SQLite (`rusqlite`) with WAL mode

## Backend Structure (`src-tauri/src/`)

### Commands (`commands/`)
Thin IPC handlers exposed to the frontend. They validate inputs and delegate to services.
*   `backup.rs`: MTP device discovery, folder browsing, backup start.
*   `wifi.rs`: Wi-Fi session start/stop.
*   `organizer.rs`: Takeout processing start.
*   `dedup.rs`: Scan, move, undo, manifest listing.
*   `migration.rs`: Legacy import, settings management.
*   `system.rs`: Health check, version.
*   `jobs.rs`: Job status, cancellation.

### Services (`services/`)
Core business logic and workflow orchestration.
*   `fileops.rs`: **The Safety Engine.** Implements atomic copy, verification (size/BLAKE2b), conflict resolution, and durability (fsync). All file movements go through here.
*   `backup.rs`: Orchestrates MTP file streaming into `fileops`.
*   `wifi.rs`: Axum HTTP server, token auth, staging management, upload handling.
*   `organizer.rs`: ZIP streaming, date parsing (JSON/EXIF/filename), naming logic.
*   `dedup.rs`: Hashing pipeline (size -> quick -> full), manifest generation, safe move/undo logic.
*   `jobs.rs`: `JobRegistry` for tracking long-running tasks, progress reporting, and cancellation tokens.

### Domain (`domain/`)
Pure logic with no side effects.
*   `naming.rs`: Date parsing, filename sanitization, idempotent prefix stripping.
*   `conflict.rs`: Conflict mode resolution (KeepBoth, Overwrite, Skip).
*   `constants.rs`: Media extensions, defaults.

### Platform (`platform/`)
OS-specific implementations.
*   `linux/mtp.rs`: GVFS/GIO integration for Android device discovery and file access.
*   *Windows/macOS:* Stubs planned for future versions; currently return `Unsupported`.

### Storage (`storage/`)
*   `schema.rs`: SQLite DDL, migrations, `PRAGMA user_version`.
*   `profiles.rs`: Device profile persistence (JSON).
*   `settings.rs`: App settings persistence (JSON).

### State (`state.rs`)
Tauri managed state holding:
*   `DbPool`: `r2d2` SQLite connection pool.
*   `JobRegistry`: Active jobs.
*   `ScanResultStore`: In-memory cache of duplicate scan results.
*   `WifiSessionManager`: Active Wi-Fi session state.

## Frontend Structure (`src/`)

*   `routes/`: Svelte components for each view (Dashboard, Phone Backup, etc.).
*   `lib/services/`: TypeScript wrappers for IPC commands. Handle errors and loading states.
*   `lib/state/`: Global reactive state (navigation, theme) using Svelte 5 runes.
*   `lib/components/`: Reusable UI components (shadcn-svelte based).
*   `lib/ipc.ts`: **Auto-generated** TypeScript bindings. **Do not edit manually.**

## IPC & Type Safety

We use `tauri-specta` to generate TypeScript types from Rust structs.
1.  Rust commands define input/output types.
2.  `npm run gen:bindings` generates `src/lib/ipc.ts`.
3.  `npm run specta:check` ensures bindings are up-to-date in CI.

## Safety Invariants

1.  **Atomic Publish:** `NamedTempFile` in destination dir -> write -> fsync -> verify -> `persist()`.
2.  **Durability:** Directory fsync after publish (POSIX).
3.  **Verification:** Size check always; BLAKE2b hash check for streams/untrusted sources.
4.  **No Partial State:** DB updated only after file durability confirmed.
5.  **Reversible Dedup:** Manifest written *before* first move. Undo uses manifest.
6.  **Wi-Fi Security:** Token in memory only, never in URL/logs. Single device per session. HTTP only (LAN trust model).

## Job System

Long-running operations (backup, scan, organize) return a `JobId` immediately.
*   Frontend polls `get_job(id)` for progress.
*   Cancellation via `cancel_job(id)` sets a `CancellationToken` checked at file boundaries.
*   Jobs run on Tokio tasks, not blocking the UI thread.

## Legacy Migration

*   **Snapshot:** Uses `rusqlite::Connection::backup` (WAL-safe) to snapshot the v1 DB.
*   **Verify:** `PRAGMA integrity_check` on snapshot.
*   **Import:** Reads v1 JSON/DB, inserts into v2 schema in a transaction.
*   **Preserve:** Original v1 files are never modified/deleted.
