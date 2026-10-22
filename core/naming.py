"""Date parsing and destination naming rules."""

from __future__ import annotations

import re
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
