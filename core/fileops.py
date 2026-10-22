"""Safe file-system helpers.

The most important rule in this module: never replace or remove a destination
file until the complete new copy has been written and verified.  Copies are
written to a temporary ``.part`` file in the *destination* directory and then
atomically moved into place with :func:`os.replace`.
"""

from __future__ import annotations

import json
import os
import shutil
import uuid
from pathlib import Path
from typing import Any, BinaryIO

PART_MARKER = ".part-"
_SAME_FILE_TOLERANCE_NS = 2_000_000_000  # 2 seconds


def is_partial_name(name: str) -> bool:
    """Return True for SyncUp temporary files."""
    return name.startswith(".") and PART_MARKER in name


def is_hidden_file(path: Path) -> bool:
    """Skip dot-files and SyncUp temporary files."""
    return path.name.startswith(".") or is_partial_name(path.name)


def unique_path(destination: Path) -> Path:
    """Return a non-existing path using ``name__1.ext`` style suffixes."""
    if not destination.exists():
        return destination
    stem = destination.stem
    suffix = destination.suffix
    counter = 1
    while True:
        candidate = destination.with_name(f"{stem}__{counter}{suffix}")
        if not candidate.exists():
            return candidate
        counter += 1


def _temp_path_for(destination: Path) -> Path:
    """Create a unique temp path in the destination directory.

    Keeping the temp file next to the destination means ``os.replace`` is a
    same-filesystem rename, so the final step is atomic.
    """
    token = f"{os.getpid()}-{uuid.uuid4().hex[:10]}"
    return destination.with_name(f".{destination.name}{PART_MARKER}{token}")


def _cleanup_temp(temp: Path) -> None:
    try:
        if temp.exists():
            temp.unlink()
    except OSError:
        pass


def _verify_size(temp: Path, expected_size: int) -> None:
    actual = temp.stat().st_size
    if actual != expected_size:
        raise OSError(
            f"Size verification failed: expected {expected_size} bytes, wrote {actual}"
        )


def copy_file_atomic(
    source: Path,
    destination: Path,
    *,
    preserve_mtime: bool = True,
) -> Path:
    """Copy ``source`` to ``destination`` atomically and verify its size.

    Returns the final destination path.
    """
    source = Path(source)
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)

    source_stat = source.stat()
    temp = _temp_path_for(destination)
    try:
        shutil.copyfile(source, temp)

        if preserve_mtime:
            try:
                os.utime(temp, ns=(source_stat.st_atime_ns, source_stat.st_mtime_ns))
            except OSError:
                # Some MTP/FUSE mounts do not expose usable timestamps.  The
                # copy itself is still valid, so do not fail the whole file.
                pass

        _verify_size(temp, source_stat.st_size)
        try:
            # Make the completed copy durable before publishing it.
            with open(temp, "rb+") as handle:
                os.fsync(handle.fileno())
        except OSError:
            pass
        os.replace(temp, destination)
        return destination
    except Exception:
        _cleanup_temp(temp)
        raise


def copy_stream_atomic(
    source_stream: BinaryIO,
    destination: Path,
    *,
    expected_size: int | None = None,
    mtime_ns: int | None = None,
    chunk_size: int = 1024 * 1024,
) -> Path:
    """Copy a file-like object (for example a ZIP member) atomically."""
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)

    temp = _temp_path_for(destination)
    written = 0
    try:
        with open(temp, "wb") as out:
            while True:
                chunk = source_stream.read(chunk_size)
                if not chunk:
                    break
                out.write(chunk)
                written += len(chunk)
            out.flush()
            try:
                os.fsync(out.fileno())
            except OSError:
                pass

        if expected_size is not None and written != expected_size:
            raise OSError(
                f"Size verification failed: expected {expected_size} bytes, wrote {written}"
            )

        if mtime_ns is not None:
            try:
                os.utime(temp, ns=(mtime_ns, mtime_ns))
            except OSError:
                pass

        os.replace(temp, destination)
        return destination
    except Exception:
        _cleanup_temp(temp)
        raise


def is_same_file(source: Path, destination: Path) -> bool:
    """Cheap comparison used to avoid re-copying an identical file.

    Size must match and the modification times must be within two seconds.
    This is deliberately stricter than the old size-only check, which could
    silently skip a different file that happened to have the same size.
    """
    try:
        src = source.stat()
        dst = destination.stat()
    except OSError:
        return False

    if src.st_size != dst.st_size:
        return False
    return abs(src.st_mtime_ns - dst.st_mtime_ns) <= _SAME_FILE_TOLERANCE_NS


def is_same_size_and_mtime(
    destination: Path,
    expected_size: int | None,
    expected_mtime_ns: int | None,
) -> bool:
    """Same as :func:`is_same_file`, but for values known without a source."""
    if expected_size is None or expected_mtime_ns is None:
        return False
    try:
        dst = destination.stat()
    except OSError:
        return False
    if dst.st_size != expected_size:
        return False
    return abs(dst.st_mtime_ns - expected_mtime_ns) <= _SAME_FILE_TOLERANCE_NS


def atomic_write_json(path: Path, data: Any) -> None:
    """Write JSON atomically (temp file + fsync + os.replace)."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = _temp_path_for(path)
    try:
        with open(temp, "w", encoding="utf-8") as handle:
            json.dump(data, handle, indent=2, ensure_ascii=False)
            handle.flush()
            try:
                os.fsync(handle.fileno())
            except OSError:
                pass
        os.replace(temp, path)
    except Exception:
        _cleanup_temp(temp)
        raise


def human_size(num_bytes: int | float) -> str:
    """Format a byte count for humans."""
    value = float(num_bytes)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if value < 1024 or unit == "TB":
            if unit == "B":
                return f"{int(value)} {unit}"
            return f"{value:.1f} {unit}"
        value /= 1024
    return f"{value:.1f} TB"
