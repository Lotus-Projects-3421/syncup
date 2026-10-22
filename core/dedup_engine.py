"""Duplicate finder with preview, safety manifest, and undo."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import time
from collections import defaultdict
from datetime import datetime
from pathlib import Path

from core import fileops
from core.constants import DUP_PREFIX

__all__ = [
    "get_file_hash",
    "run_dedup",
    "undo_dedup",
    "find_latest_manifest",
]

_HASH_ALGORITHM = "blake2b"
_QUICK_CHUNK = 64 * 1024


def _format_duration(seconds: float) -> str:
    return f"{int(seconds // 60)}m {int(seconds % 60)}s"


def get_file_hash(filepath: Path, algorithm: str = _HASH_ALGORITHM) -> str:
    """Return the full content hash of a file."""
    hasher = hashlib.new(algorithm)
    with open(filepath, "rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            hasher.update(chunk)
    return hasher.hexdigest()


def _quick_hash(filepath: Path) -> str:
    """Size + first/last 64 KiB, used to reject most non-duplicates cheaply."""
    size = filepath.stat().st_size
    hasher = hashlib.blake2b(digest_size=16)
    hasher.update(str(size).encode("ascii"))
    with open(filepath, "rb") as handle:
        hasher.update(handle.read(_QUICK_CHUNK))
        if size > _QUICK_CHUNK:
            handle.seek(-_QUICK_CHUNK, os.SEEK_END)
            hasher.update(handle.read(_QUICK_CHUNK))
    return hasher.hexdigest()


def _walk_files(
    scan_path: Path,
    error_details: list[str],
    error_callback=None,
) -> list[tuple[Path, int, int]]:
    records: list[tuple[Path, int, int]] = []
    for root, dirs, files in os.walk(scan_path):
        dirs[:] = [
            name
            for name in dirs
            if not name.startswith(".") and not name.startswith(DUP_PREFIX)
        ]
        for name in files:
            if name.startswith(".") or fileops.is_partial_name(name):
                continue
            path = Path(root) / name
            try:
                stat = path.stat()
            except OSError as exc:
                error_details.append(f"{path}: cannot read file information ({exc})")
                if error_callback:
                    error_callback(path.name, str(exc))
                continue
            if stat.st_size > 0:
                records.append((path, stat.st_size, stat.st_mtime_ns))
    return records


def _serialize_groups(groups: list[dict], limit: int = 200) -> list[dict]:
    result = []
    for group in groups[:limit]:
        result.append(
            {
                "size": group["size"],
                "hash": group["hash"],
                "keep": str(group["keep"]),
                "duplicates": [str(path) for path in group["duplicates"]],
            }
        )
    return result


def run_dedup(
    scan_dir: str,
    progress_callback,
    cancel_event,
    dry_run: bool = False,
    error_callback=None,
) -> dict:
    """Find exact duplicates and (unless ``dry_run``) move them aside.

    The moved files are recorded in ``_Duplicates_<timestamp>/_manifest.json``
    so :func:`undo_dedup` can put them back.
    """
    start_time = time.time()
    scan_path = Path(scan_dir)
    if not scan_path.exists() or not scan_path.is_dir():
        return {"error": "Invalid folder."}

    errors = 0
    error_details: list[str] = []

    progress_callback(0, 0, "Scanning files...")
    records = _walk_files(scan_path, error_details, error_callback)
    total_files = len(records)
    if total_files == 0:
        return {
            "processed": 0,
            "groups": 0,
            "duplicates": 0,
            "bytes": 0,
            "moved": 0,
            "errors": len(error_details),
            "error_details": error_details[:100],
            "cancelled": False,
            "dry_run": dry_run,
            "manifest": None,
            "time": _format_duration(time.time() - start_time),
        }

    by_size: dict[int, list[tuple[Path, int, int]]] = defaultdict(list)
    for record in records:
        by_size[record[1]].append(record)

    size_groups = [group for group in by_size.values() if len(group) > 1]
    total_to_hash = sum(len(group) for group in size_groups)
    if total_to_hash == 0:
        return {
            "processed": total_files,
            "groups": 0,
            "duplicates": 0,
            "bytes": 0,
            "moved": 0,
            "errors": len(error_details),
            "error_details": error_details[:100],
            "cancelled": False,
            "dry_run": dry_run,
            "manifest": None,
            "time": _format_duration(time.time() - start_time),
        }

    # Stage 1: cheap head/tail hash inside same-size groups.
    quick_groups: dict[tuple[int, str], list[tuple[Path, int, int]]] = defaultdict(list)
    hashed = 0
    for group in size_groups:
        for record in group:
            if cancel_event.is_set():
                return {
                    "processed": total_files,
                    "groups": 0,
                    "duplicates": 0,
                    "bytes": 0,
                    "moved": 0,
                    "errors": len(error_details),
                    "error_details": error_details[:100],
                    "cancelled": True,
                    "dry_run": dry_run,
                    "manifest": None,
                    "time": _format_duration(time.time() - start_time),
                }
            hashed += 1
            progress_callback(
                hashed,
                total_to_hash,
                f"Quick check {record[0].name}...",
            )
            try:
                quick = _quick_hash(record[0])
            except OSError as exc:
                errors += 1
                detail = f"{record[0]}: {exc}"
                error_details.append(detail)
                if error_callback:
                    error_callback(record[0].name, str(exc))
                continue
            quick_groups[(record[1], quick)].append(record)

    candidates = [group for group in quick_groups.values() if len(group) > 1]
    if not candidates:
        return {
            "processed": total_files,
            "groups": 0,
            "duplicates": 0,
            "bytes": 0,
            "moved": 0,
            "errors": len(error_details),
            "error_details": error_details[:100],
            "cancelled": False,
            "dry_run": dry_run,
            "manifest": None,
            "time": _format_duration(time.time() - start_time),
        }

    # Stage 2: full content hash for the remaining candidates.
    full_groups: dict[tuple[int, str], list[tuple[Path, int, int]]] = defaultdict(list)
    total_to_hash = sum(len(group) for group in candidates)
    hashed = 0
    for group in candidates:
        for record in group:
            if cancel_event.is_set():
                return {
                    "processed": total_files,
                    "groups": 0,
                    "duplicates": 0,
                    "bytes": 0,
                    "moved": 0,
                    "errors": len(error_details),
                    "error_details": error_details[:100],
                    "cancelled": True,
                    "dry_run": dry_run,
                    "manifest": None,
                    "time": _format_duration(time.time() - start_time),
                }
            hashed += 1
            progress_callback(
                hashed,
                total_to_hash,
                f"Hashing {record[0].name}...",
            )
            try:
                digest = get_file_hash(record[0])
            except OSError as exc:
                errors += 1
                detail = f"{record[0]}: {exc}"
                error_details.append(detail)
                if error_callback:
                    error_callback(record[0].name, str(exc))
                continue
            full_groups[(record[1], digest)].append(record)

    duplicate_groups: list[dict] = []
    duplicate_count = 0
    duplicate_bytes = 0
    for (size, digest), group in full_groups.items():
        if len(group) < 2:
            continue
        # Prefer the shallowest path, then the oldest file.  This tends to keep
        # the original library copy over a nested duplicate.
        group.sort(key=lambda item: (len(item[0].parts), item[2], str(item[0])))
        keep = group[0][0]
        duplicates = [item[0] for item in group[1:]]
        duplicate_groups.append(
            {
                "size": size,
                "hash": digest,
                "keep": keep,
                "duplicates": duplicates,
            }
        )
        duplicate_count += len(duplicates)
        duplicate_bytes += size * len(duplicates)

    serialized = _serialize_groups(duplicate_groups)

    if dry_run or duplicate_count == 0:
        return {
            "processed": total_files,
            "groups": len(duplicate_groups),
            "duplicates": duplicate_count,
            "bytes": duplicate_bytes,
            "moved": 0,
            "errors": len(error_details),
            "error_details": error_details[:100],
            "cancelled": False,
            "dry_run": dry_run,
            "manifest": None,
            "group_details": serialized,
            "time": _format_duration(time.time() - start_time),
        }

    duplicate_dir = fileops.unique_path(scan_path / f"{DUP_PREFIX}{int(time.time())}")
    planned_entries: list[dict] = []
    for group in duplicate_groups:
        for duplicate in group["duplicates"]:
            try:
                relative = duplicate.relative_to(scan_path)
            except ValueError as exc:
                errors += 1
                detail = f"{duplicate}: cannot build relative path ({exc})"
                error_details.append(detail)
                if error_callback:
                    error_callback(duplicate.name, str(exc))
                continue
            destination = fileops.unique_path(duplicate_dir / relative)
            planned_entries.append(
                {
                    "original": str(duplicate),
                    "moved_to": str(destination),
                    "size": group["size"],
                    "hash": group["hash"],
                }
            )

    manifest_path: Path | None = None
    if planned_entries:
        manifest_path = duplicate_dir / "_manifest.json"
        try:
            # Write the manifest *before* touching any file.  If the process
            # dies halfway through the moves, undo can still discover exactly
            # which files were intended to move and restore the ones that
            # actually made it.
            fileops.atomic_write_json(
                manifest_path,
                {
                    "version": 1,
                    "created_at": datetime.now().isoformat(timespec="seconds"),
                    "scan_dir": str(scan_path),
                    "entries": planned_entries,
                },
            )
        except Exception as exc:  # noqa: BLE001 - never move without a manifest
            detail = f"manifest: {exc}"
            return {
                "error": f"Cannot write duplicate manifest: {exc}",
                "processed": total_files,
                "groups": len(duplicate_groups),
                "duplicates": duplicate_count,
                "bytes": duplicate_bytes,
                "moved": 0,
                "errors": len(error_details) + 1,
                "error_details": (error_details + [detail])[:100],
                "cancelled": False,
                "dry_run": False,
                "manifest": None,
                "group_details": serialized,
                "time": _format_duration(time.time() - start_time),
            }

    moved = 0
    cancelled = False
    for entry in planned_entries:
        if cancel_event.is_set():
            cancelled = True
            break
        duplicate = Path(entry["original"])
        destination = Path(entry["moved_to"])
        try:
            if destination.exists():
                raise OSError("move target appeared unexpectedly")
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(duplicate), str(destination))
            moved += 1
            progress_callback(
                moved,
                duplicate_count,
                f"Moved {duplicate.name}...",
            )
        except Exception as exc:  # noqa: BLE001 - report per-file failures
            errors += 1
            detail = f"{duplicate}: {exc}"
            error_details.append(detail)
            if error_callback:
                error_callback(duplicate.name, str(exc))

    return {
        "processed": total_files,
        "groups": len(duplicate_groups),
        "duplicates": duplicate_count,
        "bytes": duplicate_bytes,
        "moved": moved,
        "errors": len(error_details),
        "error_details": error_details[:100],
        "cancelled": cancelled,
        "dry_run": False,
        "manifest": str(manifest_path) if manifest_path else None,
        "group_details": serialized,
        "time": _format_duration(time.time() - start_time),
    }


def find_latest_manifest(scan_dir: str) -> Path | None:
    """Return the newest duplicate-finder manifest under ``scan_dir``."""
    scan_path = Path(scan_dir)
    if not scan_path.exists():
        return None
    manifests: list[tuple[int, Path]] = []
    for path in scan_path.glob(f"{DUP_PREFIX}*/_manifest.json"):
        try:
            manifests.append((path.stat().st_mtime_ns, path))
        except OSError:
            continue
    if not manifests:
        return None
    return max(manifests, key=lambda item: item[0])[1]


def undo_dedup(
    scan_dir: str,
    progress_callback,
    cancel_event,
    manifest_path: str | Path | None = None,
) -> dict:
    """Move duplicates recorded in a manifest back to their original paths."""
    start_time = time.time()
    manifest = Path(manifest_path) if manifest_path else find_latest_manifest(scan_dir)
    if manifest is None or not manifest.exists():
        return {"error": "No duplicate-finder manifest was found to undo."}

    try:
        with open(manifest, "r", encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        return {"error": f"Cannot read manifest: {exc}"}

    entries = data.get("entries", []) if isinstance(data, dict) else []
    if not isinstance(entries, list):
        entries = []
    total = len(entries)
    restored = 0
    skipped = 0
    errors = 0
    error_details: list[str] = []
    cancelled = False

    for index, entry in enumerate(entries, start=1):
        if cancel_event.is_set():
            cancelled = True
            break
        if not isinstance(entry, dict):
            errors += 1
            error_details.append(f"entry {index}: malformed manifest entry")
            continue
        progress_callback(
            index - 1,
            total,
            f"Restoring {Path(entry.get('original', '')).name}...",
        )
        try:
            original = Path(entry["original"])
            moved = Path(entry["moved_to"])
            if not moved.exists():
                skipped += 1
                continue
            if original.exists():
                skipped += 1
                error_details.append(f"{original}: target already exists, left in place")
                continue
            original.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(moved), str(original))
            restored += 1
        except Exception as exc:  # noqa: BLE001 - report per-file failures
            errors += 1
            error_details.append(f"{entry.get('original', '?')}: {exc}")

    return {
        "manifest": str(manifest),
        "total": total,
        "restored": restored,
        "skipped": skipped,
        "errors": errors,
        "error_details": error_details[:100],
        "cancelled": cancelled,
        "time": _format_duration(time.time() - start_time),
    }
