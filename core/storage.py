"""Persistent settings, device profiles, and backup state.

Settings and profiles are small JSON documents written atomically.  Backup
state is stored in SQLite so a crash cannot corrupt the whole "already backed
up" list and so progress can be committed in small transactions.
"""

from __future__ import annotations

import json
import os
import sqlite3
import sys
import threading
import time
from datetime import datetime
from pathlib import Path

from core.constants import DEFAULT_SETTINGS
from core.fileops import atomic_write_json

_LOCK = threading.RLock()


def get_app_data_dir() -> Path:
    """Return the per-user configuration directory and create it."""
    if os.name == "nt":
        base = Path(os.getenv("APPDATA", Path.home() / "AppData" / "Roaming"))
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.getenv("XDG_CONFIG_HOME", Path.home() / ".config"))
    app_dir = base / "SyncUp"
    app_dir.mkdir(parents=True, exist_ok=True)
    return app_dir


def get_log_dir() -> Path:
    """Return the directory used for rotated log files."""
    log_dir = get_app_data_dir() / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    return log_dir


# ---------------------------------------------------------------------------
# Settings and profiles
# ---------------------------------------------------------------------------

def load_global_settings() -> dict:
    """Load settings, filling in defaults added in newer versions."""
    settings_file = get_app_data_dir() / "settings.json"
    data: dict = {}
    if settings_file.exists():
        try:
            with open(settings_file, "r", encoding="utf-8") as handle:
                loaded = json.load(handle)
            if isinstance(loaded, dict):
                data = loaded
        except (OSError, json.JSONDecodeError):
            data = {}
    return {**DEFAULT_SETTINGS, **data}


def save_global_settings(data: dict) -> None:
    with _LOCK:
        atomic_write_json(get_app_data_dir() / "settings.json", data)


def _safe_id(device_id: str) -> str:
    safe = "".join(char for char in str(device_id) if char.isalnum() or char in ("-", "_"))
    return safe or "default"


def _profile_path(device_id: str) -> Path:
    return get_app_data_dir() / f"profile_{_safe_id(device_id)}.json"


def load_profile(device_id: str) -> dict | None:
    profile_file = _profile_path(device_id)
    if not profile_file.exists():
        return None
    try:
        with open(profile_file, "r", encoding="utf-8") as handle:
            data = json.load(handle)
        return data if isinstance(data, dict) else None
    except (OSError, json.JSONDecodeError):
        return None


def save_profile(device_id: str, profile_data: dict) -> None:
    with _LOCK:
        atomic_write_json(_profile_path(device_id), profile_data)


# ---------------------------------------------------------------------------
# Backup state (SQLite with legacy JSON migration)
# ---------------------------------------------------------------------------

def get_state_file(device_id: str) -> Path:
    """Legacy JSON state path (kept for migration and compatibility)."""
    return get_app_data_dir() / f"state_{_safe_id(device_id)}.json"


class StateStore:
    """Thread-confined SQLite store of files already backed up.

    The values kept are the source size and mtime.  That lets a later run
    notice that a file was replaced or edited even if its path is unchanged.
    Rows written by older versions have ``size``/``mtime`` set to NULL and are
    treated as "processed" for backward compatibility.
    """

    def __init__(self, device_id: str):
        self.device_id = _safe_id(device_id)
        self.db_path = get_app_data_dir() / f"state_{self.device_id}.db"
        self._lock = threading.RLock()
        self._conn = self._connect()
        self._migrate_legacy_json()

    def _connect(self) -> sqlite3.Connection:
        conn = None
        try:
            conn = sqlite3.connect(str(self.db_path), timeout=30)
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=NORMAL")
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS processed (
                    rel_path TEXT PRIMARY KEY,
                    size INTEGER,
                    mtime INTEGER,
                    copied_at TEXT
                )
                """
            )
            conn.commit()
            return conn
        except sqlite3.DatabaseError as exc:
            # A lock/permission problem is not corruption.  Never rename a
            # database that another running instance may still be using.
            if isinstance(exc, sqlite3.OperationalError):
                if conn is not None:
                    try:
                        conn.close()
                    except sqlite3.Error:
                        pass
                raise
            if conn is not None:
                try:
                    conn.close()
                except sqlite3.Error:
                    pass
            # Preserve the damaged database instead of deleting it, then start
            # a fresh one.  The old JSON state (if present) is still imported.
            try:
                backup = self.db_path.with_name(
                    f"{self.db_path.name}.corrupt-{time.time_ns()}"
                )
                os.replace(self.db_path, backup)
                for suffix in ("-wal", "-shm"):
                    sidecar = Path(str(self.db_path) + suffix)
                    if sidecar.exists():
                        os.replace(sidecar, Path(str(backup) + suffix))
            except OSError:
                pass
            conn = sqlite3.connect(str(self.db_path), timeout=30)
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS processed (
                    rel_path TEXT PRIMARY KEY,
                    size INTEGER,
                    mtime INTEGER,
                    copied_at TEXT
                )
                """
            )
            conn.commit()
            return conn

    def _migrate_legacy_json(self) -> None:
        legacy = get_state_file(self.device_id)
        if not legacy.exists():
            return
        try:
            with open(legacy, "r", encoding="utf-8") as handle:
                entries = json.load(handle)
        except (OSError, json.JSONDecodeError):
            return

        if not isinstance(entries, list):
            return

        rows = [(str(entry), None, None, None) for entry in entries]
        if not rows:
            return
        with self._lock:
            self._conn.executemany(
                "INSERT OR IGNORE INTO processed(rel_path, size, mtime, copied_at) "
                "VALUES(?, ?, ?, ?)",
                rows,
            )
            self._conn.commit()

    def load(self) -> dict[str, dict]:
        with self._lock:
            rows = self._conn.execute(
                "SELECT rel_path, size, mtime FROM processed"
            ).fetchall()
        return {
            rel_path: {"size": size, "mtime": mtime}
            for rel_path, size, mtime in rows
        }

    def mark(
        self,
        rel_path: str,
        size: int | None = None,
        mtime: int | None = None,
        *,
        commit: bool = False,
    ) -> None:
        with self._lock:
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

    def commit(self) -> None:
        with self._lock:
            self._conn.commit()

    def close(self) -> None:
        with self._lock:
            try:
                self._conn.commit()
            finally:
                self._conn.close()

    def __enter__(self) -> "StateStore":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()


def load_state(device_id: str) -> dict[str, dict]:
    store = StateStore(device_id)
    try:
        return store.load()
    finally:
        store.close()


def save_state(device_id: str, processed) -> None:
    """Compatibility helper for callers that want to replace/save a set/dict."""
    store = StateStore(device_id)
    try:
        if isinstance(processed, dict):
            for rel_path, value in processed.items():
                if isinstance(value, dict):
                    store.mark(rel_path, value.get("size"), value.get("mtime"))
                else:
                    store.mark(rel_path)
        else:
            for rel_path in processed:
                store.mark(rel_path)
        store.commit()
    finally:
        store.close()
