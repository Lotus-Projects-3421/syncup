"""Date parsing and destination naming rules."""

from __future__ import annotations

import re
import struct
from datetime import datetime
from pathlib import Path

# Matches dates such as 2024-01-01, 20240101, 2024_01_01 and optionally a
# time such as 123456, 12-34-56 or 12.34.56.
_DATE_TIME_RE = re.compile(
    r"(?P<year>19\d{2}|20\d{2})[-._]?"
    r"(?P<month>0[1-9]|1[0-2])[-._]?"
    r"(?P<day>0[1-9]|[12]\d|3[01])"
    r"(?:[T _.-]?"
    r"(?P<hour>[01]\d|2[0-3])[-._:]?"
    r"(?P<minute>[0-5]\d)[-._:]?"
    r"(?P<second>[0-5]\d))?"
)

# The prefix SyncUp adds: 2024-01-01_12-34-56_
_GENERATED_PREFIX_RE = re.compile(r"^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}_")

_MAX_STEM_LENGTH = 140


def _match_datetime(stem: str) -> datetime | None:
    match = _DATE_TIME_RE.search(stem)
    if not match:
        return None
    try:
        parts = match.groupdict()
        return datetime(
            int(parts["year"]),
            int(parts["month"]),
            int(parts["day"]),
            int(parts["hour"] or 0),
            int(parts["minute"] or 0),
            int(parts["second"] or 0),
        )
    except ValueError:
        return None


def strip_generated_prefix(stem: str) -> str:
    """Remove one or more SyncUp date prefixes from a filename stem."""
    result = stem
    while _GENERATED_PREFIX_RE.match(result):
        result = _GENERATED_PREFIX_RE.sub("", result, count=1)
    return result or stem


def parse_datetime_from_name(stem: str) -> datetime | None:
    """Extract the best date/time available in a filename.

    If the name starts with a SyncUp prefix whose time is midnight, a real time
    later in the original name is preferred.  This repairs files produced by
    older versions, which parsed only the date and wrote ``00-00-00``.
    """
    stripped = strip_generated_prefix(stem)
    if stripped != stem:
        prefix_dt = _match_datetime(stem)
        if prefix_dt is not None:
            if (
                prefix_dt.hour == 0
                and prefix_dt.minute == 0
                and prefix_dt.second == 0
            ):
                better = _match_datetime(stripped)
                if better is not None and better.date() == prefix_dt.date():
                    return better
            return prefix_dt
    return _match_datetime(stem)


def date_for_path(path: Path, stat_result=None) -> datetime:
    """Date from the filename, then the file mtime, then now."""
    parsed = parse_datetime_from_name(path.stem)
    if parsed is not None:
        return parsed

    try:
        mtime = stat_result.st_mtime if stat_result is not None else path.stat().st_mtime
        return datetime.fromtimestamp(mtime)
    except (OSError, OverflowError, ValueError):
        return datetime.now()


# ---------------------------------------------------------------------------
# Minimal, dependency-free EXIF date reader (JPEG APP1 and TIFF files)
# ---------------------------------------------------------------------------

_EXIF_SUFFIXES = frozenset({".jpg", ".jpeg", ".tif", ".tiff"})
_EXIF_MAX_READ = 2 * 1024 * 1024  # EXIF lives near the start of normal photos

# TIFF tags
_TAG_EXIF_IFD = 0x8769
_TAG_DATETIME_ORIGINAL = 0x9003
_TAG_DATETIME_DIGITIZED = 0x9004
_TAG_DATETIME = 0x0132

_TYPE_ASCII = 2
_TYPE_LONG = 4


def _unpack(endian: str, fmt: str, data: bytes):
    return struct.unpack_from(endian + fmt, data)


def _ifd_pointer(data: bytes, endian: str, ifd_offset: int, wanted_tag: int) -> int | None:
    """Return a LONG IFD pointer stored in *ifd_offset*."""
    if ifd_offset <= 0 or ifd_offset + 2 > len(data):
        return None
    count = _unpack(endian, "H", data[ifd_offset : ifd_offset + 2])[0]
    start = ifd_offset + 2
    if start + count * 12 > len(data):
        return None
    for index in range(count):
        entry = start + index * 12
        tag, value_type, _value_count = _unpack(
            endian, "HHI", data[entry : entry + 8]
        )
        if tag == wanted_tag and value_type == _TYPE_LONG:
            return _unpack(endian, "I", data[entry + 8 : entry + 12])[0]
    return None


def _ascii_tag(data: bytes, endian: str, ifd_offset: int, wanted_tag: int) -> str | None:
    """Read an ASCII tag from an IFD."""
    if ifd_offset <= 0 or ifd_offset + 2 > len(data):
        return None
    count = _unpack(endian, "H", data[ifd_offset : ifd_offset + 2])[0]
    start = ifd_offset + 2
    if start + count * 12 > len(data):
        return None
    for index in range(count):
        entry = start + index * 12
        tag, value_type, value_count = _unpack(
            endian, "HHI", data[entry : entry + 8]
        )
        if tag != wanted_tag or value_type != _TYPE_ASCII or value_count <= 0:
            continue
        if value_count <= 4:
            raw = data[entry + 8 : entry + 8 + value_count]
        else:
            value_offset = _unpack(endian, "I", data[entry + 8 : entry + 12])[0]
            if value_offset < 0 or value_offset + value_count > len(data):
                continue
            raw = data[value_offset : value_offset + value_count]
        return raw.split(b"\x00", 1)[0].decode("ascii", "ignore").strip()
    return None


def _parse_exif_text(value: str | None) -> datetime | None:
    if not value:
        return None
    text = value.strip()
    for fmt in (
        "%Y:%m:%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y:%m:%d %H:%M",
        "%Y:%m:%d",
    ):
        try:
            parsed = datetime.strptime(text, fmt)
        except ValueError:
            continue
        if 1900 <= parsed.year <= 3000:
            return parsed
    return None


def _parse_tiff_datetime(data: bytes) -> datetime | None:
    """Parse a TIFF-structured EXIF block (also used inside JPEG APP1)."""
    if len(data) < 8:
        return None
    if data[:2] == b"II":
        endian = "<"
    elif data[:2] == b"MM":
        endian = ">"
    else:
        return None
    if _unpack(endian, "H", data[2:4])[0] != 42:
        return None
    ifd0 = _unpack(endian, "I", data[4:8])[0]

    exif_ifd = _ifd_pointer(data, endian, ifd0, _TAG_EXIF_IFD)
    candidates: list[str | None] = []
    if exif_ifd is not None:
        candidates.append(
            _ascii_tag(data, endian, exif_ifd, _TAG_DATETIME_ORIGINAL)
        )
        candidates.append(
            _ascii_tag(data, endian, exif_ifd, _TAG_DATETIME_DIGITIZED)
        )
    candidates.append(_ascii_tag(data, endian, ifd0, _TAG_DATETIME))

    for candidate in candidates:
        parsed = _parse_exif_text(candidate)
        if parsed is not None:
            return parsed
    return None


def _parse_jpeg_datetime(data: bytes) -> datetime | None:
    """Find the first EXIF APP1 segment in a JPEG stream."""
    if not data.startswith(b"\xff\xd8"):
        return None
    offset = 2
    size = len(data)
    while offset + 1 < size:
        if data[offset] != 0xFF:
            offset += 1
            continue
        while offset < size and data[offset] == 0xFF:
            offset += 1
        if offset >= size:
            break
        marker = data[offset]
        offset += 1

        if marker in (0xD8, 0xD9, 0x01) or 0xD0 <= marker <= 0xD7:
            continue
        if marker == 0xDA:  # start of scan data; no more metadata
            break
        if offset + 2 > size:
            break

        segment_length = int.from_bytes(data[offset : offset + 2], "big")
        if segment_length < 2:
            break
        segment_start = offset + 2
        segment_end = min(offset + segment_length, size)
        if (
            marker == 0xE1
            and data[segment_start : segment_start + 6] == b"Exif\x00\x00"
        ):
            return _parse_tiff_datetime(data[segment_start + 6 : segment_end])
        offset = segment_end
    return None


def read_exif_datetime(path: Path) -> datetime | None:
    """Best-effort EXIF ``DateTimeOriginal`` reader; never raises."""
    path = Path(path)
    if path.suffix.lower() not in _EXIF_SUFFIXES:
        return None
    try:
        # Read-only metadata probe; all write paths live in core/fileops.py.
        with open(path, "rb") as handle:
            data = handle.read(_EXIF_MAX_READ)
        if data.startswith((b"II", b"MM")):
            return _parse_tiff_datetime(data)
        return _parse_jpeg_datetime(data)
    except Exception:  # noqa: BLE001 - metadata is optional
        return None


def date_with_source(
    path: Path, stat_result=None
) -> tuple[datetime, str]:
    """Return ``(date, source)`` where source is filename/exif/mtime.

    Filename dates still win over EXIF for phone backups so existing file
    naming stays stable.  EXIF is only reported when it is the source used.
    """
    path = Path(path)
    parsed = parse_datetime_from_name(path.stem)
    if parsed is not None:
        return parsed, "filename"

    exif_date = read_exif_datetime(path)
    if exif_date is not None:
        return exif_date, "exif"

    try:
        mtime = stat_result.st_mtime if stat_result is not None else path.stat().st_mtime
        return datetime.fromtimestamp(mtime), "mtime"
    except (OSError, OverflowError, ValueError):
        return datetime.now(), "mtime"


def build_target_name(file_path: Path, date_obj: datetime) -> str:
    """Build a deterministic, idempotent destination filename.

    Any existing SyncUp prefix is removed before the new prefix is added, so
    processing an already-organised folder does not produce
    ``2024-01-01_..._2024-01-01_...`` names.
    """
    base_stem = strip_generated_prefix(file_path.stem)
    if len(base_stem) > _MAX_STEM_LENGTH:
        base_stem = base_stem[:_MAX_STEM_LENGTH]

    prefix = date_obj.strftime("%Y-%m-%d_%H-%M-%S")
    return f"{prefix}_{base_stem}{file_path.suffix.lower()}"


def get_target_path(
    file_path: Path,
    dest_root: Path,
    sort_mode: str,
    date_obj: datetime | None = None,
) -> Path:
    """Return the full destination path for a media file."""
    file_path = Path(file_path)
    dest_root = Path(dest_root)
    date_obj = date_obj or date_for_path(file_path)

    year = date_obj.strftime("%Y")
    year_month = date_obj.strftime("%Y-%m")
    year_month_day = date_obj.strftime("%Y-%m-%d")
    new_name = build_target_name(file_path, date_obj)

    if sort_mode == "year":
        return dest_root / year / new_name
    if sort_mode == "month":
        return dest_root / year / year_month / new_name
    if sort_mode == "day":
        return dest_root / year / year_month_day / new_name
    if sort_mode == "flat":
        return dest_root / new_name
    # Unknown modes fall back to the safest default instead of dropping the
    # date prefix and reusing the original name verbatim.
    return dest_root / year / new_name
