"""Small shared UI helpers for the three tabs."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import flet as ft

from core.fileops import human_size

__all__ = [
    "open_folder",
    "show_snack",
    "level_color",
    "show_scoreboard",
    "progress_callback_for",
    "card",
    "register_job",
    "begin_job",
    "finish_job",
    "error_lines",
    "human_size",
]

# One source of truth for Activity Log / scoreboard colours.  The second value
# is used in light mode where the pastel dark-mode variants are too faint.
_LEVEL_COLORS = {
    "success": (ft.Colors.GREEN_400, ft.Colors.GREEN_700),
    "scan": (ft.Colors.AMBER_400, ft.Colors.AMBER_800),
    "warning": (ft.Colors.AMBER_400, ft.Colors.AMBER_800),
    "state": (ft.Colors.AMBER_400, ft.Colors.AMBER_800),
    "organize": (ft.Colors.BLUE_300, ft.Colors.BLUE_700),
    "extract": (ft.Colors.BLUE_300, ft.Colors.BLUE_700),
    "error": (ft.Colors.RED_400, ft.Colors.RED_700),
    "info": (ft.Colors.ON_SURFACE, ft.Colors.ON_SURFACE),
}


def level_color(level: str | None, is_dark: bool = True) -> str:
    """Return the theme-appropriate colour for a telemetry/log level."""
    dark, light = _LEVEL_COLORS.get(str(level or "info").lower(), _LEVEL_COLORS["info"])
    return dark if is_dark else light


def is_dark_mode(page: ft.Page) -> bool:
    """Best-effort dark-mode check that also handles the SYSTEM setting."""
    try:
        return page.theme_mode != ft.ThemeMode.LIGHT
    except Exception:  # noqa: BLE001 - theme probing must never break logging
        return True


def open_folder(path: str | Path | None, on_error=None) -> bool:
    """Open a folder without blocking the Flet UI thread.

    Returns ``True`` when the opener was launched.  Missing paths and opener
    failures are reported through *on_error* instead of raising.
    """

    def _fail(message: str) -> bool:
        if callable(on_error):
            try:
                on_error(message)
            except Exception:  # noqa: BLE001 - never let reporting break the UI
                pass
        return False

    if path is None or not str(path).strip():
        return _fail("No folder selected.")

    try:
        folder = Path(path).expanduser()
    except (OSError, ValueError) as exc:
        return _fail(f"Invalid folder path: {exc}")

    if not folder.exists():
        return _fail(f"Folder does not exist: {folder}")

    try:
        if sys.platform.startswith("darwin"):
            subprocess.Popen(
                ("open", str(folder)),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                close_fds=True,
            )
        elif os.name == "nt":
            os.startfile(str(folder))  # type: ignore[attr-defined]
        else:
            opener = shutil.which("xdg-open")
            if opener is None:
                return _fail("xdg-open is not installed, so the folder could not be opened.")
            subprocess.Popen(
                (opener, str(folder)),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                close_fds=True,
                start_new_session=True,
            )
        return True
    except Exception as exc:  # noqa: BLE001 - a failed opener is non-fatal
        return _fail(f"Could not open: {exc}")


def show_snack(
    page: ft.Page,
    message: str,
    *,
    is_error: bool = False,
    duration: int = 4000,
) -> None:
    """Show a non-blocking SnackBar.  Must be called on the UI thread."""
    try:
        snack = ft.SnackBar(
            content=ft.Text(
                message,
                color=ft.Colors.WHITE if is_error else ft.Colors.ON_SURFACE,
            ),
            bgcolor=ft.Colors.RED_700 if is_error else ft.Colors.SURFACE_CONTAINER_HIGHEST,
            duration=duration,
            show_close_icon=True,
        )
        page.show_dialog(snack)
    except Exception:  # noqa: BLE001 - feedback must never break a job
        pass


def progress_callback_for(progress_queue, log_func=None):
    """Build a callback that forwards telemetry and logs human-readable events."""

    def progress_cb(current, total, message, event=None):
        if isinstance(event, dict) and log_func is not None:
            kind = event.get("kind", "progress")
            if kind in {"phase", "log", "summary"} and message:
                log_func(message, level=event.get("level", "info"))
        progress_queue.put((current, total, message, event))

    return progress_cb


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


def _metric_row(label: str, value: str, color: str | None = None) -> ft.Row:
    return ft.Row(
        [
            ft.Text(
                label,
                size=12,
                color=ft.Colors.ON_SURFACE,
                opacity=0.7,
                width=190,
            ),
            ft.Text(
                value,
                size=12,
                color=color or ft.Colors.ON_SURFACE,
                selectable=True,
                expand=True,
            ),
        ],
        spacing=8,
        vertical_alignment=ft.CrossAxisAlignment.START,
    )


def _section(title: str) -> ft.Text:
    return ft.Text(
        title,
        size=13,
        weight=ft.FontWeight.BOLD,
        color=ft.Colors.ON_SURFACE,
    )


def _status_for(result: dict) -> tuple[str, str, str]:
    """Return ``(status_text, level, icon)`` for the scoreboard banner."""
    if result.get("cancelled"):
        return "CANCELLED", "scan", ft.Icons.PAUSE_CIRCLE_OUTLINE
    if result.get("error") or result.get("errors", 0) > 0:
        return "COMPLETED WITH ERRORS", "error", ft.Icons.ERROR_OUTLINE
    return "SUCCESS", "success", ft.Icons.CHECK_CIRCLE_OUTLINE


def show_scoreboard(
    page: ft.Page,
    result: dict | None,
    *,
    title: str,
    destination: str | Path | None = None,
    operation: str = "backup",
    log_func=None,
) -> None:
    """Show the end-of-run summary dialog (``result`` is never mutated)."""
    result = result or {}
    is_dark = is_dark_mode(page)
    status, status_level, status_icon = _status_for(result)
    status_color = level_color(status_level, is_dark)

    controls: list[ft.Control] = [
        ft.Container(
            content=ft.Row(
                [
                    ft.Icon(status_icon, color=status_color, size=22),
                    ft.Text(
                        status,
                        size=14,
                        weight=ft.FontWeight.BOLD,
                        color=status_color,
                    ),
                    ft.Container(expand=True),
                ]
            ),
            padding=10,
            bgcolor=ft.Colors.SURFACE_CONTAINER_HIGHEST,
            border_radius=8,
        ),
        ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
        _section("Timings"),
        _metric_row("Start", str(result.get("started_at") or "-")),
        _metric_row("End", str(result.get("finished_at") or "-")),
        _metric_row(
            "Total duration",
            str(
                result.get("time")
                or (
                    f"{float(result.get('duration_seconds')):.1f}s"
                    if result.get("duration_seconds") is not None
                    else "-"
                )
            ),
        ),
        ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
    ]

    detected = result.get("detected") or {}
    detected_total = detected.get("total", result.get("total", 0))
    digits = (
        f"{detected_total} total  |  {detected.get('zips', 0)} ZIPs  |  "
        f"{detected.get('loose', 0)} loose  |  {detected.get('folders', 0)} folders"
    )
    controls.extend(
        [
            _section("Files detected"),
            _metric_row("Detected", digits),
            _section("Files skipped"),
            _metric_row(
                "Skipped",
                f"{result.get('skipped', 0)} file(s)",
                level_color("scan", is_dark) if result.get("skipped", 0) else None,
            ),
            _metric_row(
                "Oldest backup",
                str(result.get("oldest_backup") or "not recorded")[:10],
            ),
        ]
    )

    processed_value = f"{result.get('processed', 0)} file(s)"
    if operation == "preview":
        processed_value = (
            f"{result.get('duplicates', 0)} duplicate(s) found; nothing moved"
        )
    elif operation == "move":
        processed_value = (
            f"{result.get('moved', 0)} moved of "
            f"{result.get('duplicates', 0)} duplicate(s)"
        )
    elif operation == "undo":
        processed_value = f"{result.get('restored', 0)} restored"
    controls.extend(
        [
            _section("Files processed"),
            _metric_row(
                "Processed",
                processed_value,
                level_color("success", is_dark)
                if result.get("processed", 0) or result.get("moved", 0)
                else None,
            ),
        ]
    )

    date_sources = result.get("date_sources") or {}
    if date_sources:
        controls.append(
            _metric_row(
                "Date extraction",
                "Filename: {filename}  |  EXIF: {exif}  |  JSON: {json}  |  "
                "mtime: {mtime}".format(
                    filename=date_sources.get("filename", 0),
                    exif=date_sources.get("exif", 0),
                    json=date_sources.get("json", 0),
                    mtime=date_sources.get("mtime", 0),
                ),
                level_color("organize", is_dark),
            )
        )

    if operation in {"preview", "move"} or "duplicates" in result:
        controls.extend(
            [
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                _section("Duplicate scan"),
                _metric_row("Duplicate groups", str(result.get("groups", 0))),
                _metric_row(
                    "Duplicate files",
                    str(result.get("duplicates", 0)),
                    level_color("warning", is_dark),
                ),
                _metric_row(
                    "Reclaimable size",
                    human_size(result.get("bytes", 0)),
                    level_color("warning", is_dark),
                ),
            ]
        )
    if operation == "move":
        controls.append(_metric_row("Moved", str(result.get("moved", 0))))
        if result.get("manifest"):
            controls.append(_metric_row("Undo manifest", str(result["manifest"])))
    if operation == "undo":
        controls.extend(
            [
                _metric_row("Restored", str(result.get("restored", 0))),
                _metric_row("Left in place", str(result.get("skipped", 0))),
            ]
        )

    error_items: list[str] = []
    if result.get("error"):
        error_items.append(str(result["error"]))
    error_items.extend(str(line) for line in (result.get("error_details") or []))
    if error_items:
        controls.extend(
            [
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                _section(f"Errors ({len(error_items)})"),
            ]
        )
        for line in error_items[:10]:
            controls.append(
                ft.Text(
                    f"- {line}",
                    size=11,
                    color=level_color("error", is_dark),
                    selectable=True,
                )
            )
        if len(error_items) > 10:
            controls.append(
                ft.Text(
                    f"...and {len(error_items) - 10} more (see log)",
                    size=11,
                    color=level_color("error", is_dark),
                )
            )

    if operation == "preview" and result.get("duplicates", 0) > 0:
        controls.extend(
            [
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                ft.Text(
                    f"{result.get('duplicates', 0)} duplicate(s) are ready. Close this "
                    "summary, then click Proceed to Move.",
                    size=12,
                    color=level_color("organize", is_dark),
                ),
            ]
        )

    def _close(event=None) -> None:
        try:
            page.pop_dialog()
        except Exception:  # noqa: BLE001 - the page may already be closing
            pass
        try:
            page.update()
        except Exception:  # noqa: BLE001
            pass

    def _on_dismiss(event=None) -> None:
        # The dialog may already be closed by Flet (barrier/Escape).  Do not
        # pop again, which could dismiss a SnackBar underneath it.
        try:
            page.update()
        except Exception:  # noqa: BLE001
            pass

    actions: list[ft.Control] = []
    if destination and operation != "preview":
        def _open_destination(event=None) -> None:
            def _on_error(message: str) -> None:
                reason = str(message).strip()
                if not reason.lower().startswith("could not open"):
                    reason = f"Could not open: {reason}"
                if log_func is not None:
                    log_func(reason, is_error=True, level="error")
                show_snack(page, reason, is_error=True)

            if open_folder(destination, on_error=_on_error):
                show_snack(page, "Opened in file manager")

        actions.append(
            ft.Button(
                "Open Destination Folder",
                icon=ft.Icons.FOLDER_OPEN,
                on_click=_open_destination,
            )
        )

    actions.append(ft.Button("Close", on_click=_close))

    dialog = ft.AlertDialog(
        modal=True,
        title=ft.Text(
            title,
            weight=ft.FontWeight.BOLD,
            color=ft.Colors.ON_SURFACE,
        ),
        content=ft.Container(
            content=ft.Column(
                controls,
                spacing=6,
                tight=True,
                scroll=ft.ScrollMode.AUTO,
            ),
            width=640,
            height=460,
            padding=ft.Padding.symmetric(horizontal=4, vertical=2),
        ),
        bgcolor=ft.Colors.SURFACE,
        actions=actions,
        actions_alignment=ft.MainAxisAlignment.END,
        on_dismiss=_on_dismiss,
    )

    # Rapid back-to-back finishes must not stack dialogs.
    try:
        page.pop_dialog()
    except Exception:  # noqa: BLE001 - no dialog was open
        pass
    try:
        page.show_dialog(dialog)
    except Exception:  # noqa: BLE001 - a dialog must not hide the log
        if log_func is not None:
            log_func("Could not display the run summary.", is_error=True, level="error")
