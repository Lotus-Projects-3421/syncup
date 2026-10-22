"""Google Takeout / folder organizer.

ZIP archives are streamed member-by-member instead of being extracted to a
temporary directory.  That avoids doubling disk usage and lets the app show
progress while a large archive is processed.
"""

from __future__ import annotations

import json
import os
import shutil
import time
import zipfile
from datetime import datetime
from pathlib import Path

from core import fileops
from core.constants import IMAGE_EXT, MEDIA_EXT, VIDEO_EXT
from core.naming import (
    date_for_path,
    parse_datetime_from_name,
)
from core.naming import (
    get_target_path as build_target_path,
)

__all__ = [
    "IMAGE_EXT",
    "VIDEO_EXT",
    "MEDIA_EXT",
    "get_date_from_json",
    "get_date_from_name",
    "get_target_path",
    "run_takeout",
]


def _matches_filter(ext: str, media_filter: str) -> bool:
    if media_filter == "photos":
        return ext in IMAGE_EXT
    if media_filter == "videos":
        return ext in VIDEO_EXT
    return ext in MEDIA_EXT


def get_date_from_json_data(data) -> datetime | None:
    """Read a timestamp from a Google Takeout sidecar dictionary."""
    if not isinstance(data, dict):
        return None
    for key in ("photoTakenTime", "creationTime"):
        entry = data.get(key)
        if not isinstance(entry, dict):
            continue
        timestamp = entry.get("timestamp")
        if timestamp is None:
            continue
        try:
            return datetime.fromtimestamp(int(timestamp))
        except (ValueError, TypeError, OSError, OverflowError):
            continue
    return None


def get_date_from_json(json_path: Path) -> datetime | None:
    try:
        with open(json_path, "r", encoding="utf-8") as handle:
            return get_date_from_json_data(json.load(handle))
    except (OSError, json.JSONDecodeError, UnicodeDecodeError):
        return None


def get_date_from_name(stem: str) -> datetime | None:
    return parse_datetime_from_name(stem)


def get_target_path(
    file_path: Path,
    dest_root: Path,
    sort_mode: str,
    date_obj: datetime | None = None,
) -> Path:
    return build_target_path(Path(file_path), Path(dest_root), sort_mode, date_obj)


def _sidecar_candidates(path: Path):
    yield path.with_name(path.name + ".json")
    yield path.with_name(path.name + ".supplemental-metadata.json")


def _find_sidecar(path: Path) -> Path | None:
    for candidate in _sidecar_candidates(path):
        if candidate.exists() and candidate.is_file():
            return candidate
    return None


def _zip_sidecar_name(member_name: str, names: set[str]) -> str | None:
    candidates = [member_name + ".json"]
    candidates.append(member_name + ".supplemental-metadata.json")
    for candidate in candidates:
        if candidate in names:
            return candidate
    return None


def _zip_mtime_ns(info: zipfile.ZipInfo) -> int | None:
    try:
        return int(datetime(*info.date_time).timestamp() * 1_000_000_000)
    except (ValueError, OSError, OverflowError):
        return None


def _resolve_destination(
    destination: Path,
    conflict_mode: str,
    expected_size: int | None,
    expected_mtime_ns: int | None,
    protected: set[Path],
) -> tuple[Path, bool]:
    """Return ``(path_to_write, should_skip)``.

    An existing file with the same size and mtime is considered already
    organised.  This makes re-running the organizer idempotent instead of
    creating an endless stream of ``__1`` copies.

    ``protected`` contains destinations written earlier in the current run.
    They are never overwritten, even in overwrite mode, so two different
    source files that happen to share a target name cannot destroy each other.
    """
    if destination in protected:
        return fileops.unique_path(destination), False
    if not destination.exists():
        return destination, False
    if fileops.is_same_size_and_mtime(
        destination, expected_size, expected_mtime_ns
    ):
        return destination, True
    if conflict_mode == "overwrite":
        return destination, False
    return fileops.unique_path(destination), False


def _copy_loose_sidecar(
    sidecar: Path,
    target_media: Path,
    error_callback=None,
) -> str | None:
    destination = target_media.with_name(target_media.name + ".json")
    try:
        if destination.exists():
            if fileops.is_same_file(sidecar, destination):
                return None
            destination = fileops.unique_path(destination)
        fileops.copy_file_atomic(sidecar, destination, preserve_mtime=True)
        return None
    except Exception as exc:  # noqa: BLE001 - metadata is best-effort
        if error_callback:
            error_callback(sidecar.name, f"metadata copy failed: {exc}")
        return str(exc)


def _copy_zip_sidecar(
    archive: zipfile.ZipFile,
    sidecar_name: str,
    target_media: Path,
    error_callback=None,
) -> str | None:
    destination = target_media.with_name(target_media.name + ".json")
    try:
        info = archive.getinfo(sidecar_name)
        if destination.exists():
            if destination.stat().st_size == info.file_size:
                try:
                    if destination.read_bytes() == archive.read(info):
                        return None
                except OSError:
                    pass
            destination = fileops.unique_path(destination)
        with archive.open(info) as stream:
            fileops.copy_stream_atomic(
                stream,
                destination,
                expected_size=info.file_size,
                mtime_ns=_zip_mtime_ns(info),
            )
        return None
    except Exception as exc:  # noqa: BLE001 - metadata is best-effort
        if error_callback:
            error_callback(sidecar_name, f"metadata copy failed: {exc}")
        return str(exc)


def _format_duration(seconds: float) -> str:
    return f"{int(seconds // 60)}m {int(seconds % 60)}s"


def run_takeout(
    input_items,
    dest_path: str,
    sort_mode: str,
    media_filter: str,
    conflict_mode: str,
    progress_callback,
    cancel_event,
    preserve_metadata: bool = True,
    error_callback=None,
) -> dict:
    """Organise loose media files and/or ZIP archives into the destination."""
    start_time = time.time()
    if isinstance(input_items, (str, Path)):
        input_items = [input_items]
    input_items = [Path(item) for item in input_items or []]

    dest_root = Path(dest_path)
    try:
        dest_root.mkdir(parents=True, exist_ok=True)
        dest_resolved = dest_root.resolve()
    except OSError as exc:
        return {
            "error": f"Cannot create destination folder: {exc}",
            "total": 0,
            "processed": 0,
            "skipped": 0,
            "errors": 0,
            "error_details": [],
            "cancelled": False,
            "time": _format_duration(0),
        }

    loose_files: list[Path] = []
    zips: list[Path] = []
    seen_loose: set[str] = set()
    seen_zips: set[str] = set()
    errors = 0
    error_details: list[str] = []
    warnings: list[str] = []

    def add_loose(path: Path) -> None:
        key = str(path)
        if key not in seen_loose:
            seen_loose.add(key)
            loose_files.append(path)

    def add_zip(path: Path) -> None:
        key = str(path)
        if key not in seen_zips:
            seen_zips.add(key)
            zips.append(path)

    progress_callback(0, 0, "Scanning folders and ZIPs...")

    for item in input_items:
        if cancel_event.is_set():
            return {
                "total": 0,
                "processed": 0,
                "skipped": 0,
                "errors": 0,
                "error_details": [],
                "cancelled": True,
                "time": _format_duration(time.time() - start_time),
            }

        if item.is_dir():
            for root, dirs, files in os.walk(item):
                root_path = Path(root).resolve()
                if root_path == dest_resolved:
                    dirs[:] = []
                    continue
                dirs[:] = [name for name in dirs if not name.startswith(".")]

                for name in files:
                    if name.startswith(".") or fileops.is_partial_name(name):
                        continue
                    path = Path(root) / name
                    suffix = path.suffix.lower()
                    if suffix == ".zip":
                        add_zip(path)
                    elif _matches_filter(suffix, media_filter):
                        add_loose(path)
        elif item.is_file():
            suffix = item.suffix.lower()
            if suffix == ".zip":
                add_zip(item)
            elif _matches_filter(suffix, media_filter):
                add_loose(item)

    # Read the ZIP central directories once to build a reliable total.
    zip_members: list[tuple[Path, str, int]] = []
    for zip_path in zips:
        try:
            with zipfile.ZipFile(zip_path, "r") as archive:
                for info in archive.infolist():
                    if info.is_dir():
                        continue
                    extension = Path(info.filename.replace("\\", "/")).suffix.lower()
                    if _matches_filter(extension, media_filter):
                        zip_members.append((zip_path, info.filename, info.file_size))
        except (OSError, zipfile.BadZipFile) as exc:
            errors += 1
            detail = f"{zip_path.name}: cannot read archive ({exc})"
            error_details.append(detail)
            if error_callback:
                error_callback(zip_path.name, str(exc))

    total_files = len(loose_files) + len(zip_members)
    if total_files == 0:
        return {
            "total": 0,
            "processed": 0,
            "skipped": 0,
            "errors": errors,
            "error_details": error_details[:100],
            "warnings": warnings,
            "cancelled": False,
            "time": _format_duration(time.time() - start_time),
        }

    total_bytes = sum(info_size for _, _, info_size in zip_members)
    for path in loose_files:
        try:
            total_bytes += path.stat().st_size
        except OSError:
            pass
    try:
        free_bytes = shutil.disk_usage(dest_root).free
        if total_bytes > free_bytes:
            return {
                "error": (
                    "Not enough free space at the destination. "
                    f"Need about {total_bytes / (1024 ** 3):.1f} GB, "
                    f"only {free_bytes / (1024 ** 3):.1f} GB free."
                ),
                "total": total_files,
                "processed": 0,
                "skipped": 0,
                "errors": errors,
                "error_details": error_details[:100],
                "cancelled": False,
                "time": _format_duration(time.time() - start_time),
            }
    except OSError:
        pass

    processed = 0
    skipped = 0
    done = 0
    cancelled = False
    written_targets: set[Path] = set()

    # ------------------------------------------------------------------
    # Loose files
    # ------------------------------------------------------------------
    for source_file in loose_files:
        if cancel_event.is_set():
            cancelled = True
            break

        done += 1
        progress_callback(
            done - 1,
            total_files,
            f"Organizing {source_file.name}...",
        )
        try:
            source_stat = source_file.stat()
            sidecar = _find_sidecar(source_file)
            date_obj = get_date_from_json(sidecar) if sidecar else None
            if date_obj is None:
                date_obj = parse_datetime_from_name(source_file.stem)
            if date_obj is None:
                date_obj = date_for_path(source_file, source_stat)

            target_file = build_target_path(
                source_file, dest_root, sort_mode, date_obj
            )
            target_file, should_skip = _resolve_destination(
                target_file,
                conflict_mode,
                source_stat.st_size,
                source_stat.st_mtime_ns,
                written_targets,
            )

            if should_skip:
                skipped += 1
            else:
                fileops.copy_file_atomic(
                    source_file,
                    target_file,
                    preserve_mtime=True,
                )
                processed += 1
            written_targets.add(target_file)

            if preserve_metadata and sidecar is not None:
                sidecar_error = _copy_loose_sidecar(
                    sidecar, target_file, error_callback
                )
                if sidecar_error:
                    errors += 1
                    error_details.append(
                        f"{sidecar.name}: metadata copy failed: {sidecar_error}"
                    )
        except Exception as exc:  # noqa: BLE001 - report per-file failures
            errors += 1
            detail = f"{source_file.name}: {exc}"
            error_details.append(detail)
            if error_callback:
                error_callback(source_file.name, str(exc))

        progress_callback(done, total_files, f"Processed {done} / {total_files}")

    # ------------------------------------------------------------------
    # ZIP archives (streamed, never extracted)
    # ------------------------------------------------------------------
    current_zip: Path | None = None
    archive: zipfile.ZipFile | None = None
    names: set[str] = set()

    try:
        for zip_path, member_name, _member_size in zip_members:
            if cancelled or cancel_event.is_set():
                cancelled = True
                break

            if current_zip != zip_path:
                if archive is not None:
                    archive.close()
                try:
                    archive = zipfile.ZipFile(zip_path, "r")
                    current_zip = zip_path
                    names = set(archive.namelist())
                except (OSError, zipfile.BadZipFile) as exc:
                    errors += 1
                    detail = f"{zip_path.name}: cannot open archive ({exc})"
                    error_details.append(detail)
                    if error_callback:
                        error_callback(zip_path.name, str(exc))
                    archive = None
                    current_zip = None
                    continue

            if archive is None:
                continue

            done += 1
            progress_callback(
                done - 1,
                total_files,
                f"Organizing {Path(member_name).name}...",
            )

            try:
                info = archive.getinfo(member_name)
                sidecar_name = _zip_sidecar_name(member_name, names)
                date_obj = None
                if sidecar_name is not None:
                    try:
                        sidecar_data = json.loads(
                            archive.read(sidecar_name).decode("utf-8")
                        )
                        date_obj = get_date_from_json_data(sidecar_data)
                    except (KeyError, OSError, ValueError, UnicodeDecodeError):
                        date_obj = None

                member_path = Path(member_name.replace("\\", "/"))
                if date_obj is None:
                    date_obj = parse_datetime_from_name(member_path.stem)
                if date_obj is None:
                    date_obj = datetime(*info.date_time)

                mtime_ns = _zip_mtime_ns(info)
                target_file = build_target_path(
                    member_path, dest_root, sort_mode, date_obj
                )
                target_file, should_skip = _resolve_destination(
                    target_file,
                    conflict_mode,
                    info.file_size,
                    mtime_ns,
                    written_targets,
                )

                if should_skip:
                    skipped += 1
                else:
                    with archive.open(info) as stream:
                        fileops.copy_stream_atomic(
                            stream,
                            target_file,
                            expected_size=info.file_size,
                            mtime_ns=mtime_ns,
                        )
                    processed += 1
                written_targets.add(target_file)

                if preserve_metadata and sidecar_name is not None:
                    sidecar_error = _copy_zip_sidecar(
                        archive, sidecar_name, target_file, error_callback
                    )
                    if sidecar_error:
                        errors += 1
                        error_details.append(
                            f"{Path(sidecar_name).name}: "
                            f"metadata copy failed: {sidecar_error}"
                        )
            except Exception as exc:  # noqa: BLE001 - report per-file failures
                errors += 1
                detail = f"{Path(member_name).name}: {exc}"
                error_details.append(detail)
                if error_callback:
                    error_callback(Path(member_name).name, str(exc))

            progress_callback(done, total_files, f"Processed {done} / {total_files}")
    finally:
        if archive is not None:
            archive.close()

    return {
        "total": total_files,
        "processed": processed,
        "skipped": skipped,
        "errors": errors,
        "error_details": error_details[:100],
        "warnings": warnings,
        "cancelled": cancelled,
        "time": _format_duration(time.time() - start_time),
    }
