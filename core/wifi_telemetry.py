"""Telemetry bridge for Wi-Fi Backup: FastAPI routes → ``progress_queue``.

The engines talk to a ``(current, total, message, event)`` callback.  The
mobile upload endpoints do not produce engine-style progress, so they call the
convenience helpers on this class instead.  Both paths enqueue the exact same
tuple shape that ``main.progress_consumer`` already understands.

This module is thread-safe: ``queue.Queue.put`` is safe to call from the Uvicorn
worker threads, and ``ProgressEmitter`` is stateless apart from its start time.
"""

from __future__ import annotations

import queue
from typing import Callable

from core.telemetry import (
    KIND_LOG,
    KIND_PHASE,
    KIND_PROGRESS,
    KIND_SUMMARY,
    LEVEL_ERROR,
    LEVEL_INFO,
    LEVEL_ORGANIZE,
    LEVEL_SUCCESS,
    LEVEL_WARNING,
    PHASE_DONE,
    PHASE_ORGANIZE,
    PHASE_SCAN,
    PHASE_STATE,
    PHASE_TRANSFER,
    ProgressEmitter,
)

ProgressTuple = tuple[int, int, str, dict | None]


class WiFiTelemetry:
    """Thread-safe telemetry emitter for the Wi-Fi backup server.

    ``progress_callback`` is the public callable to hand to ``run_wifi_backup``.
    """

    def __init__(
        self,
        progress_queue: "queue.Queue[ProgressTuple]",
        post_ui: Callable[[Callable[[], None]], None] | None = None,
    ) -> None:
        self._queue = progress_queue
        self._post_ui = post_ui
        self._emitter = ProgressEmitter(self._callback)

    # ------------------------------------------------------------------
    # Callback plumbing
    # ------------------------------------------------------------------

    def _callback(
        self,
        current: int,
        total: int,
        message: str,
        event: dict | None = None,
    ) -> None:
        """Internal callback invoked by :class:`ProgressEmitter`."""
        self._queue.put((current, total, message, event))

    @property
    def progress_callback(self) -> Callable[..., None]:
        """Public 4-argument callback for :func:`core.engine.run_wifi_backup`."""
        return self._callback

    def send(
        self,
        current: int,
        total: int,
        message: str,
        event: dict | None = None,
    ) -> None:
        """Forward an already-built event tuple."""
        self._callback(current, total, message, event)

    # ------------------------------------------------------------------
    # Phases (kept in lock-step with core.engine.run_backup)
    # ------------------------------------------------------------------

    def scan_started(self, folder_count: int) -> None:
        self._emitter.emit(
            PHASE_SCAN,
            f"Scanning {folder_count} uploaded folder(s)...",
            kind=KIND_PHASE,
            indeterminate=True,
            level=LEVEL_WARNING,
            metrics={"folders": folder_count},
        )

    def scan_complete(self, total_files: int, total_bytes: int) -> None:
        self._emitter.emit(
            PHASE_SCAN,
            f"Found {total_files} uploaded file(s) ({total_bytes} bytes) to process.",
            kind=KIND_LOG,
            indeterminate=True,
            level=LEVEL_WARNING,
            metrics={"candidates": total_files, "total_bytes": total_bytes},
        )

    def state_checked(self, previously_backed_up: int, new_files: int) -> None:
        self._emitter.emit(
            PHASE_STATE,
            f"Database check: {previously_backed_up} backed up, {new_files} new.",
            kind=KIND_LOG,
            indeterminate=True,
            level=LEVEL_WARNING,
            metrics={
                "previously_backed_up": previously_backed_up,
                "new_files": new_files,
            },
        )

    def upload_progress(
        self,
        index: int,
        total: int,
        filename: str,
        bytes_received: int,
    ) -> None:
        """Drive the shared Flet progress bar while the phone is uploading."""
        self._emitter.emit(
            PHASE_TRANSFER,
            f"Receiving {filename}...",
            kind=KIND_PROGRESS,
            current=index,
            total=total,
            indeterminate=False,
            level=LEVEL_ORGANIZE,
            metrics={
                "filename": filename,
                "bytes_received": bytes_received,
                "upload": True,
            },
        )

    def transfer_started(self, total_files: int) -> None:
        self._emitter.emit(
            PHASE_TRANSFER,
            f"Transferring {total_files} file(s)...",
            kind=KIND_PHASE,
            current=0,
            total=total_files,
            indeterminate=False,
            level=LEVEL_ORGANIZE,
        )

    def transfer_progress(
        self,
        index: int,
        total: int,
        filename: str,
        bytes_done: int,
        speed_bps: float,
    ) -> None:
        self._emitter.emit(
            PHASE_TRANSFER,
            f"Copying {filename}...",
            kind=KIND_PROGRESS,
            current=index,
            total=total,
            indeterminate=False,
            level=LEVEL_ORGANIZE,
            metrics={
                "filename": filename,
                "bytes_done": bytes_done,
                "speed_bytes_per_sec": round(speed_bps, 1),
            },
        )

    def organize_summary(
        self,
        date_sources: dict,
        skipped: int,
        processed: int,
    ) -> None:
        self._emitter.emit(
            PHASE_ORGANIZE,
            f"Date sources: {date_sources}",
            kind=KIND_LOG,
            level=LEVEL_ORGANIZE,
            metrics={"date_sources": date_sources},
        )
        self._emitter.emit(
            PHASE_ORGANIZE,
            f"Skipped {skipped} file(s).",
            kind=KIND_LOG,
            level=LEVEL_ORGANIZE,
            metrics={"skipped": skipped},
        )
        self._emitter.emit(
            PHASE_ORGANIZE,
            f"Organized {processed} new file(s).",
            kind=KIND_LOG,
            level=LEVEL_ORGANIZE,
            metrics={"processed": processed},
        )

    def done(
        self,
        processed: int,
        skipped: int,
        errors: int,
        bytes_copied: int,
        cancelled: bool = False,
        error: str | None = None,
    ) -> None:
        if cancelled:
            level = LEVEL_WARNING
            message = "Wi-Fi backup cancelled."
        elif error or errors:
            level = LEVEL_ERROR
            message = f"Wi-Fi backup finished with {errors} error(s)."
        else:
            level = LEVEL_SUCCESS
            message = f"Wi-Fi backup finished: {processed} copied, {skipped} skipped."

        self._emitter.emit(
            PHASE_DONE,
            message,
            kind=KIND_SUMMARY,
            level=level,
            current=processed,
            total=processed + skipped,
            indeterminate=False,
            metrics={
                "processed": processed,
                "skipped": skipped,
                "errors": errors,
                "bytes_copied": bytes_copied,
            },
        )

    # ------------------------------------------------------------------
    # Ad-hoc log lines (server lifecycle, manifests, uploads)
    # ------------------------------------------------------------------

    def log(self, message: str, level: str = LEVEL_INFO) -> None:
        self._emitter.emit(
            PHASE_TRANSFER,
            message,
            kind=KIND_LOG,
            level=level,
        )

    def error(self, message: str) -> None:
        self.log(message, level=LEVEL_ERROR)


__all__ = ["WiFiTelemetry", "ProgressTuple"]
