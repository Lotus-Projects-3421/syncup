"""Structured progress telemetry shared by all engines.

Engines emit JSON-serialisable events through the existing progress callback.
The event is passed as an optional fourth positional argument so older
three-argument callbacks keep working.  Everything here is pure Python and has
no UI dependencies.
"""

from __future__ import annotations

import inspect
import time
from datetime import datetime

# Phases
PHASE_SCAN = "scan"
PHASE_STATE = "state"
PHASE_ORGANIZE = "organize"
PHASE_TRANSFER = "transfer"
PHASE_HASH = "hash"
PHASE_MOVE = "move"
PHASE_UNDO = "undo"
PHASE_DONE = "done"

# Event kinds
KIND_PHASE = "phase"
KIND_LOG = "log"
KIND_PROGRESS = "progress"
KIND_SUMMARY = "summary"

# Log levels understood by the UI
LEVEL_INFO = "info"
LEVEL_SUCCESS = "success"
LEVEL_WARNING = "warning"
LEVEL_ORGANIZE = "organize"
LEVEL_ERROR = "error"

SCHEMA_VERSION = 1


def iso_now() -> str:
    """Return the current local time in a stable, second-precision format."""
    return datetime.now().isoformat(timespec="seconds")


def empty_detected() -> dict:
    """Return zeroed file-detection counters."""
    return {
        "total": 0,
        "zips": 0,
        "loose": 0,
        "folders": 0,
        "zip_members": 0,
    }


def empty_date_sources() -> dict:
    """Return zeroed date-extraction counters."""
    return {"filename": 0, "exif": 0, "json": 0, "mtime": 0}


def with_timings(result: dict, started_at: str, started_monotonic: float) -> dict:
    """Attach start/end/duration fields without overwriting engine data."""
    result.setdefault("started_at", started_at)
    result["finished_at"] = iso_now()
    result.setdefault(
        "duration_seconds", round(time.monotonic() - started_monotonic, 2)
    )
    return result


def _accepts_event(callback) -> bool:
    """Return True when *callback* can accept the structured event argument."""
    try:
        signature = inspect.signature(callback)
    except (TypeError, ValueError):
        # Builtins and some callables do not expose a signature.  Prefer the
        # new protocol; engines are internal callers.
        return True

    parameters = list(signature.parameters.values())
    if any(p.kind is inspect.Parameter.VAR_POSITIONAL for p in parameters):
        return True
    positional = [
        p
        for p in parameters
        if p.kind
        in (inspect.Parameter.POSITIONAL_ONLY, inspect.Parameter.POSITIONAL_OR_KEYWORD)
    ]
    return len(positional) >= 4


class ProgressEmitter:
    """Build and send phase-structured progress events.

    ``progress_callback`` is called as ``(current, total, message, event)``.
    Callbacks that only accept three arguments receive the legacy shape.
    """

    def __init__(self, progress_callback):
        if progress_callback is None:
            progress_callback = lambda *args: None  # noqa: E731
        self._callback = progress_callback
        self._send_event = _accepts_event(progress_callback)
        self.started_monotonic = time.monotonic()

    @property
    def elapsed(self) -> float:
        return time.monotonic() - self.started_monotonic

    def emit(
        self,
        phase: str,
        message: str = "",
        *,
        kind: str = KIND_PROGRESS,
        current: int = 0,
        total: int = 0,
        level: str = LEVEL_INFO,
        indeterminate: bool | None = None,
        metrics: dict | None = None,
        **extra_metrics,
    ) -> dict:
        """Emit one event and return it (handy for tests)."""
        current = int(current or 0)
        total = int(total or 0)
        percent = round((current / total) * 100, 1) if total > 0 else 0.0
        if indeterminate is None:
            indeterminate = total <= 0

        merged = dict(metrics or {})
        merged.update(extra_metrics)

        event = {
            "version": SCHEMA_VERSION,
            "phase": phase,
            "kind": kind,
            "level": level,
            "message": str(message),
            "current": current,
            "total": total,
            "percent": percent,
            "indeterminate": bool(indeterminate),
            "timestamp": iso_now(),
            "metrics": merged,
        }

        if self._send_event:
            self._callback(current, total, message, event)
        else:
            self._callback(current, total, message)
        return event


def date_sources_label(sources: dict | None) -> str:
    """Format date-source counters for a log line."""
    sources = sources or {}
    return (
        f"{int(sources.get('filename', 0))} from filename, "
        f"{int(sources.get('exif', 0))} from EXIF, "
        f"{int(sources.get('json', 0))} from JSON sidecars, "
        f"{int(sources.get('mtime', 0))} from mtime"
    )


__all__ = [
    "PHASE_SCAN",
    "PHASE_STATE",
    "PHASE_ORGANIZE",
    "PHASE_TRANSFER",
    "PHASE_HASH",
    "PHASE_MOVE",
    "PHASE_UNDO",
    "PHASE_DONE",
    "KIND_PHASE",
    "KIND_LOG",
    "KIND_PROGRESS",
    "KIND_SUMMARY",
    "LEVEL_INFO",
    "LEVEL_SUCCESS",
    "LEVEL_WARNING",
    "LEVEL_ORGANIZE",
    "LEVEL_ERROR",
    "SCHEMA_VERSION",
    "ProgressEmitter",
    "iso_now",
    "empty_detected",
    "empty_date_sources",
    "with_timings",
    "date_sources_label",
]
