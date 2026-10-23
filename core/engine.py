"""Phone (MTP) backup engine.

Safety rules:
* A destination is never deleted before the replacement copy succeeds.
* Every copy is written to a temporary file in the destination folder, its
  size is verified, and only then atomically renamed into place.
* A file is recorded as backed up only after that verified copy.
"""

from __future__ import annotations

import os
import shutil
import time
from datetime import datetime
from pathlib import Path

from core import fileops
from core.constants import IMAGE_EXT, MEDIA_EXT, VIDEO_EXT
from core.naming import date_for_path, date_with_source
from core.naming import get_target_path as build_target_path
from core.storage import StateStore
from core.telemetry import (
    KIND_LOG,
    KIND_PHASE,
    KIND_SUMMARY,
    LEVEL_ERROR,
    LEVEL_ORGANIZE,
    LEVEL_SUCCESS,
    LEVEL_WARNING,
    PHASE_DONE,
    PHASE_ORGANIZE,
    PHASE_SCAN,
    PHASE_STATE,
    PHASE_TRANSFER,
    ProgressEmitter,
    date_sources_label,
    empty_date_sources,
    empty_detected,
    iso_now,
    with_timings,
)

# Re-exported for backwards compatibility.
__all__ = [
    "IMAGE_EXT",
    "VIDEO_EXT",
    "MEDIA_EXT",
    "get_media_files",
    "parse_date",
    "get_target_path",
    "run_backup",
]

MAX_CONSECUTIVE_ERRORS = 25
STATE_COMMIT_EVERY = 10


def _matches_filter(ext: str, media_filter: str) -> bool:
    if media_filter == "photos":
        return ext in IMAGE_EXT
    if media_filter == "videos":
        return ext in VIDEO_EXT
    return ext in MEDIA_EXT


def get_media_files(folder_path: Path, media_filter: str) -> list[Path]:
    """Recursively collect media files, ignoring hidden/temporary entries."""
    media: list[Path] = []
    for root, dirs, files in os.walk(folder_path):
        dirs[:] = [name for name in dirs if not name.startswith(".")]
        for name in files:
            if name.startswith(".") or fileops.is_partial_name(name):
                continue
            path = Path(root) / name
            if _matches_filter(path.suffix.lower(), media_filter):
                media.append(path)
    return media


def parse_date(file_path: Path) -> datetime:
    """Compatibility wrapper for the shared naming rules."""
    return date_for_path(Path(file_path))


def get_target_path(
    file_path: Path,
    dest_root: Path,
    sort_mode: str,
    date_obj: datetime | None = None,
) -> Path:
    """Compatibility wrapper for the shared naming rules."""
    return build_target_path(Path(file_path), Path(dest_root), sort_mode, date_obj)


def _safe_relative(folder: str) -> Path | None:
    """Reject absolute paths and ``..`` segments coming from profiles."""
    raw = str(folder).replace("\\", "/").strip()
    if not raw:
        return None
    candidate = Path(raw)
    if candidate.is_absolute():
        return None
    candidate = Path(raw.strip("/"))
    if not candidate.parts or any(part == ".." for part in candidate.parts):
        return None
    return candidate


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


def _format_duration(seconds: float) -> str:
    return f"{int(seconds // 60)}m {int(seconds % 60)}s"


def run_backup(
    phone_id: str,
    phone_path: str,
    selected_folders,
    dest_path: str,
    sort_mode: str,
    media_filter: str,
    conflict_mode: str,
    progress_callback,
    cancel_event,
    error_callback=None,
) -> dict:
    """Back up selected phone folders.

    Returns a result dict with ``total``, ``processed``, ``skipped``,
    ``errors``, ``error_details``, ``time`` and ``cancelled`` keys.  ``error``
    is present only for a fatal problem (device missing, no space, and so on).
    """
    start_time = time.time()
    started_at = iso_now()
    emitter = ProgressEmitter(progress_callback)
    date_sources = empty_date_sources()
    bytes_copied = 0
    detected = empty_detected()

    def _finalize(data: dict) -> dict:
        data.setdefault("detected", detected)
        data.setdefault("date_sources", date_sources)
        return with_timings(data, started_at, start_time)

    phone_root = Path(phone_path) / "Internal shared storage"
    if not phone_root.exists():
        phone_root = Path(phone_path)
    dest_root = Path(dest_path)

    if not phone_root.exists():
        return _finalize({
            "error": f"Phone storage was not found at {phone_path}.",
            "total": 0,
            "processed": 0,
            "skipped": 0,
            "errors": 0,
            "error_details": [],
            "cancelled": False,
            "time": _format_duration(0),
        })

    try:
        # Check before creating anything: writing a folder onto the phone just
        # to reject it is both slow and risky on MTP.
        if dest_root.resolve().is_relative_to(phone_root.resolve()):
            return _finalize({
                "error": "The destination folder cannot be inside the phone storage.",
                "total": 0,
                "processed": 0,
                "skipped": 0,
                "errors": 0,
                "error_details": [],
                "cancelled": False,
                "time": _format_duration(0),
            })
    except (OSError, ValueError):
        pass

    try:
        dest_root.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        return _finalize({
            "error": f"Cannot create destination folder: {exc}",
            "total": 0,
            "processed": 0,
            "skipped": 0,
            "errors": 0,
            "error_details": [],
            "cancelled": False,
            "time": _format_duration(0),
        })

    root_folders = []
    for folder in selected_folders or []:
        relative = _safe_relative(folder)
        if relative is None:
            continue
        folder_path = phone_root / relative
        if folder_path.exists() and folder_path.is_dir():
            root_folders.append((relative, folder_path))

    if not root_folders:
        return _finalize({
            "error": "No readable folders were selected.",
            "total": 0,
            "processed": 0,
            "skipped": 0,
            "errors": 0,
            "error_details": [],
            "cancelled": False,
            "time": _format_duration(0),
        })

    store = StateStore(phone_id)
    processed_files = store.load()
    total_files = 0
    processed = 0
    skipped = 0
    errors = 0
    error_details: list[str] = []
    warnings: list[str] = []
    candidates: list[tuple[Path, str, os.stat_result]] = []
    total_bytes = 0
    found_files = 0

    try:
        emitter.emit(
            PHASE_SCAN,
            f"Scanning {len(root_folders)} folders...",
            kind=KIND_PHASE,
            indeterminate=True,
            level=LEVEL_WARNING,
            metrics={"folders": len(root_folders), "zips": 0, "loose": 0},
        )
        seen_paths: set[str] = set()

        for relative, folder_path in root_folders:
            for source_file in get_media_files(folder_path, media_filter):
                try:
                    source_stat = source_file.stat()
                except OSError as exc:
                    errors += 1
                    detail = f"{source_file.name}: cannot read file information ({exc})"
                    error_details.append(detail)
                    if error_callback:
                        error_callback(source_file.name, str(exc))
                    continue

                rel_path = str(source_file.relative_to(phone_root))
                # Selected folders can overlap (e.g. DCIM and DCIM/Camera).
                # Copy each source path only once per run.
                if rel_path in seen_paths:
                    continue
                seen_paths.add(rel_path)
                found_files += 1
                if _is_processed(
                    processed_files.get(rel_path),
                    source_stat.st_size,
                    source_stat.st_mtime_ns,
                ):
                    skipped += 1
                    continue

                candidates.append((source_file, rel_path, source_stat))
                total_bytes += source_stat.st_size

        total_files = len(candidates)
        detected.update(
            {
                "total": found_files,
                "zips": 0,
                "loose": found_files,
                "folders": len(root_folders),
            }
        )

        emitter.emit(
            PHASE_SCAN,
            f"Scanning {len(root_folders)} folders... "
            f"Found 0 ZIP files and {found_files} loose files.",
            kind=KIND_LOG,
            indeterminate=True,
            level=LEVEL_WARNING,
            metrics={
                "folders": len(root_folders),
                "zips": 0,
                "loose": found_files,
                "candidates": total_files,
            },
        )

        oldest_backup = store.oldest_copied_at()
        oldest_label = oldest_backup[:10] if oldest_backup else "none yet"
        emitter.emit(
            PHASE_STATE,
            "Checking database... "
            f"Found {len(processed_files)} previously backed-up files. "
            f"(Oldest backup: {oldest_label}). "
            f"{total_files} new files to process.",
            kind=KIND_LOG,
            indeterminate=True,
            level=LEVEL_WARNING,
            metrics={
                "previously_backed_up": len(processed_files),
                "oldest_backup": oldest_backup,
                "new_files": total_files,
                "skipped": skipped,
            },
        )

        if total_files == 0:
            result = _finalize({
                "total": 0,
                "processed": 0,
                "skipped": skipped,
                "errors": errors,
                "error_details": error_details[:100],
                "warnings": warnings,
                "cancelled": False,
                "total_bytes": total_bytes,
                "bytes_copied": 0,
                "oldest_backup": oldest_backup,
                "time": _format_duration(time.time() - start_time),
            })
            emitter.emit(
                PHASE_DONE,
                "Phone backup finished: nothing new to copy.",
                kind=KIND_SUMMARY,
                level=LEVEL_SUCCESS,
                current=0,
                total=0,
                metrics={"processed": 0, "skipped": skipped, "errors": errors},
            )
            return result

        try:
            free_bytes = shutil.disk_usage(dest_root).free
            if total_bytes > free_bytes:
                return _finalize({
                    "error": (
                        "Not enough free space at the destination. "
                        f"Need {total_bytes / (1024 ** 3):.1f} GB, "
                        f"only {free_bytes / (1024 ** 3):.1f} GB free."
                    ),
                    "total": total_files,
                    "processed": 0,
                    "skipped": skipped,
                    "errors": errors,
                    "error_details": error_details[:100],
                    "cancelled": False,
                    "total_bytes": total_bytes,
                    "bytes_copied": 0,
                    "oldest_backup": oldest_backup,
                    "time": _format_duration(time.time() - start_time),
                })
        except OSError:
            pass

        cancelled = False
        abort_message = None
        consecutive_errors = 0
        created_targets: set[Path] = set()

        emitter.emit(
            PHASE_TRANSFER,
            f"Transferring {total_files} files...",
            kind=KIND_PHASE,
            current=0,
            total=total_files,
            indeterminate=False,
            level=LEVEL_ORGANIZE,
        )

        for index, (source_file, rel_path, source_stat) in enumerate(candidates, start=1):
            if cancel_event.is_set():
                cancelled = True
                break

            speed = bytes_copied / emitter.elapsed if emitter.elapsed > 0 else 0.0
            emitter.emit(
                PHASE_TRANSFER,
                f"Copying {source_file.name}... ({fileops.human_size(speed)}/s)",
                kind="progress",
                current=index - 1,
                total=total_files,
                indeterminate=False,
                level=LEVEL_ORGANIZE,
                metrics={
                    "filename": source_file.name,
                    "bytes_done": bytes_copied,
                    "speed_bytes_per_sec": round(speed, 1),
                },
            )

            try:
                date_obj, date_source = date_with_source(source_file, source_stat)
                date_sources[date_source] = date_sources.get(date_source, 0) + 1
                target_file = build_target_path(
                    source_file, dest_root, sort_mode, date_obj
                )

                created_this_run = target_file in created_targets
                if target_file.exists():
                    if not created_this_run and fileops.is_same_file(
                        source_file, target_file
                    ):
                        processed_files[rel_path] = {
                            "size": source_stat.st_size,
                            "mtime": source_stat.st_mtime_ns,
                        }
                        store.mark(
                            rel_path,
                            source_stat.st_size,
                            source_stat.st_mtime_ns,
                        )
                        created_targets.add(target_file)
                        skipped += 1
                        consecutive_errors = 0
                        emitter.emit(
                            PHASE_TRANSFER,
                            f"Already present: {source_file.name}",
                            kind="progress",
                            current=index,
                            total=total_files,
                            indeterminate=False,
                            level=LEVEL_ORGANIZE,
                            metrics={"filename": source_file.name, "skipped": skipped},
                        )
                        continue

                    # Never let two different source files overwrite each
                    # other during the same run.  Overwrite mode is only for
                    # replacing files left by an earlier run.
                    if conflict_mode != "overwrite" or created_this_run:
                        target_file = fileops.unique_path(target_file)
                    # In overwrite mode we let os.replace() inside
                    # copy_file_atomic replace the old file only after the new
                    # copy has been verified.

                # Crash-safe verified copy; see core/fileops.py.
                fileops.copy_file_atomic(
                    source_file,
                    target_file,
                    preserve_mtime=True,
                )
                created_targets.add(target_file)
                processed += 1
                bytes_copied += source_stat.st_size
                consecutive_errors = 0
                processed_files[rel_path] = {
                    "size": source_stat.st_size,
                    "mtime": source_stat.st_mtime_ns,
                }
                store.mark(
                    rel_path,
                    source_stat.st_size,
                    source_stat.st_mtime_ns,
                    commit=(processed % STATE_COMMIT_EVERY == 0),
                )
            except Exception as exc:  # noqa: BLE001 - report per-file failures
                errors += 1
                consecutive_errors += 1
                detail = f"{rel_path}: {exc}"
                error_details.append(detail)
                if error_callback:
                    error_callback(rel_path, str(exc))
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS:
                    abort_message = (
                        f"Stopped after {consecutive_errors} consecutive errors. "
                        "The phone may have disconnected."
                    )
                    break

            emitter.emit(
                PHASE_TRANSFER,
                f"Processed {index} / {total_files}",
                kind="progress",
                current=index,
                total=total_files,
                indeterminate=False,
                level=LEVEL_ORGANIZE,
                metrics={
                    "processed": processed,
                    "skipped": skipped,
                    "errors": errors,
                    "bytes_done": bytes_copied,
                },
            )

        store.commit()

        emitter.emit(
            PHASE_ORGANIZE,
            f"Extracted dates: {date_sources_label(date_sources)}.",
            kind=KIND_LOG,
            level=LEVEL_ORGANIZE,
            metrics={"date_sources": date_sources},
        )
        emitter.emit(
            PHASE_ORGANIZE,
            f"Skipped {skipped} files (already exist).",
            kind=KIND_LOG,
            level=LEVEL_ORGANIZE,
            metrics={"skipped": skipped},
        )
        emitter.emit(
            PHASE_ORGANIZE,
            f"Renamed/Organized {processed} new files.",
            kind=KIND_LOG,
            level=LEVEL_ORGANIZE,
            metrics={"processed": processed},
        )

        elapsed = time.time() - start_time
        result = {
            "total": total_files,
            "processed": processed,
            "skipped": skipped,
            "errors": errors,
            "error_details": error_details[:100],
            "warnings": warnings,
            "cancelled": cancelled,
            "total_bytes": total_bytes,
            "bytes_copied": bytes_copied,
            "oldest_backup": oldest_backup,
            "time": _format_duration(elapsed),
        }
        result = _finalize(result)
        if abort_message:
            result["error"] = abort_message
        if cancelled:
            summary_level = LEVEL_WARNING
            summary_message = "Phone backup cancelled."
        elif result.get("error") or errors:
            summary_level = LEVEL_ERROR
            summary_message = (
                f"Phone backup finished with {errors} error(s) in {result['time']}."
            )
        else:
            summary_level = LEVEL_SUCCESS
            summary_message = (
                f"Phone backup finished in {result['time']}: "
                f"{processed} copied, {skipped} skipped, {errors} errors."
            )
        emitter.emit(
            PHASE_DONE,
            summary_message,
            kind=KIND_SUMMARY,
            level=summary_level,
            current=processed,
            total=total_files,
            indeterminate=False,
            metrics={
                "processed": processed,
                "skipped": skipped,
                "errors": errors,
                "bytes_copied": bytes_copied,
                "date_sources": date_sources,
            },
        )
        return result
    finally:
        store.close()
