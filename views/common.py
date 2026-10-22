"""Small shared UI helpers for the three tabs."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import flet as ft

from core.fileops import human_size

__all__ = [
    "open_folder",
    "card",
    "register_job",
    "begin_job",
    "finish_job",
    "error_lines",
    "human_size",
]


def open_folder(path: str | Path | None) -> None:
    """Open a folder in the platform file manager."""
    if not path:
        return
    folder = Path(path)
    if not folder.exists():
        return
    try:
        if sys.platform.startswith("darwin"):
            subprocess.Popen(
                ("open", str(folder)),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        elif os.name == "nt":
            os.startfile(str(folder))  # type: ignore[attr-defined]
        else:
            subprocess.Popen(
                ("xdg-open", str(folder)),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
    except OSError:
        pass


def card(content: ft.Control) -> ft.Container:
    return ft.Container(
        content=content,
        padding=16,
        bgcolor=ft.Colors.SURFACE,
        border_radius=12,
        border=ft.Border.all(1, ft.Colors.OUTLINE_VARIANT),
    )


def register_job(state: dict, start_button: ft.Control, cancel_button: ft.Control) -> None:
    """Register a tab's buttons so all starts are disabled during a job."""
    start_buttons = state.setdefault("start_buttons", [])
    cancel_buttons = state.setdefault("cancel_buttons", [])
    if start_button not in start_buttons:
        start_buttons.append(start_button)
    if cancel_button not in cancel_buttons:
        cancel_buttons.append(cancel_button)


def begin_job(state: dict, active_cancel_button: ft.Control) -> None:
    """Update button state. Must be called on the UI thread."""
    state["is_running"] = True
    state["cancel_event"].clear()

    for button in state.get("start_buttons", []):
        button.disabled = True

    for button in state.get("cancel_buttons", []):
        button.visible = button is active_cancel_button
        button.disabled = False
        button.content = "Cancel"


def finish_job(state: dict) -> None:
    """Restore button state after a job. Must run on the UI thread."""
    state["is_running"] = False
    for button in state.get("start_buttons", []):
        button.disabled = False
    for button in state.get("cancel_buttons", []):
        button.visible = False
        button.disabled = False
        button.content = "Cancel"


def error_lines(result: dict, limit: int = 20) -> list[str]:
    """Return human-readable per-file error lines from an engine result."""
    lines: list[str] = []
    details = result.get("error_details") or []
    for detail in details[:limit]:
        lines.append(f"Failed: {detail}")
    if len(details) > limit:
        lines.append(f"... and {len(details) - limit} more errors (see the log file).")
    return lines
