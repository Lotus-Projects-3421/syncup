"""Linux MTP device discovery."""

from __future__ import annotations

import getpass
import os
import re
import subprocess
import threading
import time
from pathlib import Path
from urllib.parse import unquote

_MOUNT_COOLDOWN_SECONDS = 10.0
_last_mount_attempt = 0.0
_mount_lock = threading.Lock()


def _trigger_mount_if_needed(force: bool = False) -> None:
    """Wake up lazy gvfs MTP mounts.

    Polling every few seconds used to spawn ``gio mount`` continuously.  A
    cooldown keeps idle CPU/wakeups low while still fixing the common Linux
    behaviour where a phone only appears after its mount is requested.
    """
    global _last_mount_attempt
    now = time.monotonic()
    with _mount_lock:
        if not force and now - _last_mount_attempt < _MOUNT_COOLDOWN_SECONDS:
            return
        _last_mount_attempt = now

    try:
        result = subprocess.run(
            ["gio", "mount", "-li"],
            capture_output=True,
            text=True,
            timeout=8,
        )
        uris = re.findall(r"(mtp://[^\s]+)", result.stdout)
        for uri in uris:
            subprocess.run(
                ["gio", "mount", uri],
                capture_output=True,
                timeout=8,
            )
    except (OSError, subprocess.SubprocessError):
        pass


def _display_name(raw_name: str) -> str:
    clean = raw_name.replace("mtp:host=", "")
    clean = unquote(clean)
    parts = [part for part in clean.split("_") if part]
    if parts:
        return " ".join(parts[:3]).title()
    return "Android device"


def get_connected_devices(force_mount: bool = False) -> list[dict]:
    devices: list[dict] = []
    uid = os.getuid()
    try:
        username = getpass.getuser()
    except Exception:  # noqa: BLE001 - only used to build candidate paths
        username = "user"

    _trigger_mount_if_needed(force=force_mount)

    search_paths = [
        Path(f"/run/user/{uid}/gvfs"),
        Path(f"/media/{username}"),
        Path(f"/run/media/{username}"),
    ]

    for base_path in search_paths:
        if not base_path.exists():
            continue
        try:
            entries = list(base_path.iterdir())
        except OSError:
            continue
        for item in entries:
            try:
                if not item.is_dir() or "mtp" not in item.name.lower():
                    continue
            except OSError:
                continue

            if any(device["path"] == str(item) for device in devices):
                continue

            devices.append(
                {
                    "name": _display_name(item.name),
                    "id": f"mtp://{item.name.replace('mtp:host=', '')}/",
                    "path": str(item),
                }
            )

    return devices


def get_folders(device_path: str, relative_path: str = "") -> list[str]:
    """List immediate subfolders inside the phone storage."""
    folders: list[str] = []
    try:
        root = Path(device_path) / "Internal shared storage"
        if not root.exists():
            root = Path(device_path)

        target_dir = root / relative_path
        if not target_dir.exists() or not target_dir.is_dir():
            return []

        for item in target_dir.iterdir():
            try:
                if (
                    item.is_dir()
                    and not item.name.startswith(".")
                    and not item.name.startswith("_")
                ):
                    folders.append(item.name)
            except OSError:
                continue
    except OSError:
        pass
    return sorted(folders, key=str.lower)
