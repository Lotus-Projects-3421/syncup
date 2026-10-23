"""SyncUp — photo/video backup and organizer UI entry point."""

from __future__ import annotations

import asyncio
import os
import queue
import subprocess
import sys
import threading
from datetime import datetime
from pathlib import Path

import flet as ft

from core.cli import sanitize_argv
from core.constants import APP_NAME, VERSION
from core.logging_setup import setup_logging
from core.storage import load_global_settings, save_global_settings
from views.common import level_color, open_folder, show_snack
from views.dedup_view import build_dedup_view
from views.organizer_view import build_organizer_view
from views.phone_view import build_phone_view

# AppImages forward literal %U/%F field codes as arguments; strip them before
# Flet or any folder-opening code can see them.
sys.argv[:] = sanitize_argv(sys.argv)


def _play_ding() -> None:
    try:
        if os.name == "nt":
            import winsound

            winsound.Beep(1000, 500)
        elif sys.platform == "darwin":
            subprocess.run(
                ["afplay", "/System/Library/Sounds/Glass.aiff"],
                timeout=5,
                check=False,
            )
        else:
            subprocess.run(
                ["canberra-gtk-play", "-i", "complete"],
                capture_output=True,
                timeout=5,
                check=False,
            )
    except Exception:  # noqa: BLE001 - sound is optional
        pass


def play_ding() -> None:
    """Play the completion sound without blocking the Flet UI thread."""
    threading.Thread(target=_play_ding, daemon=True).start()


def main(page: ft.Page) -> None:
    logger, log_dir = setup_logging()
    settings = load_global_settings()

    page.title = f"{APP_NAME} — Backup & Organizer"
    page.padding = 0
    page.theme_mode = (
        ft.ThemeMode.LIGHT
        if settings.get("theme") == "light"
        else ft.ThemeMode.DARK
    )
    page.theme = ft.Theme(color_scheme_seed=ft.Colors.BLUE)
    page.dark_theme = ft.Theme(color_scheme_seed=ft.Colors.BLUE)

    try:
        page.window.width = settings.get("window_width", 1200)
        page.window.height = settings.get("window_height", 950)
        page.window.min_width = 1000
        page.window.min_height = 700
        app_icon = Path(__file__).resolve().parent / "assets" / "syncup.png"
        if app_icon.exists():
            page.window.icon = str(app_icon)
    except Exception:  # noqa: BLE001 - window API varies by Flet version
        pass

    state = {
        "is_running": False,
        "shutdown_event": threading.Event(),
        "cancel_event": threading.Event(),
        "start_buttons": [],
        "cancel_buttons": [],
    }
    progress_queue: queue.Queue = queue.Queue()
    ui_queue: queue.Queue = queue.Queue()

    def post_ui(callback) -> None:
        """Schedule a function on the Flet UI thread."""
        if not state["shutdown_event"].is_set():
            ui_queue.put(callback)

    def log(message: str, is_error: bool = False, level: str | None = None) -> None:
        if state["shutdown_event"].is_set():
            return
        log_level = "error" if is_error else (level or "info")
        if is_error or log_level == "error":
            logger.error(message)
        else:
            logger.info(message)

        timestamp = datetime.now().strftime("%H:%M:%S")

        def append_log() -> None:
            log_col.controls.append(
                ft.Text(
                    f"[{timestamp}] {message}",
                    size=13,
                    font_family="monospace",
                    color=level_color(
                        log_level,
                        page.theme_mode != ft.ThemeMode.LIGHT,
                    ),
                )
            )
            while len(log_col.controls) > 800:
                log_col.controls.pop(0)
            log_col.auto_scroll = True

        post_ui(append_log)

    progress_text = ft.Text(
        "",
        size=13,
        weight=ft.FontWeight.BOLD,
        color=ft.Colors.ON_SURFACE,
        font_family="monospace",
    )
    phase_text = ft.Text(
        "",
        size=14,
        weight=ft.FontWeight.BOLD,
        color=ft.Colors.ON_SURFACE,
    )
    progress_ring = ft.ProgressRing(
        width=24,
        height=24,
        stroke_width=3,
        color=ft.Colors.BLUE_300,
        visible=False,
    )
    progress_bar = ft.ProgressBar(
        value=0,
        color=ft.Colors.BLUE_300,
        bgcolor=ft.Colors.OUTLINE_VARIANT,
        visible=False,
        expand=True,
    )
    progress_host = ft.Container(
        content=ft.Row(
            [progress_ring, progress_bar],
            spacing=10,
            vertical_alignment=ft.CrossAxisAlignment.CENTER,
        ),
        height=30,
    )
    log_col = ft.Column(spacing=4, scroll=ft.ScrollMode.AUTO, expand=True, auto_scroll=True)

    def clear_log(event) -> None:
        log_col.controls.clear()
        page.update()

    theme_btn = ft.IconButton(
        icon=ft.Icons.LIGHT_MODE
        if page.theme_mode == ft.ThemeMode.DARK
        else ft.Icons.DARK_MODE,
        icon_color=ft.Colors.WHITE,
    )

    def toggle_theme(event) -> None:
        if page.theme_mode == ft.ThemeMode.DARK:
            page.theme_mode = ft.ThemeMode.LIGHT
            theme_btn.icon = ft.Icons.DARK_MODE
            settings["theme"] = "light"
        else:
            page.theme_mode = ft.ThemeMode.DARK
            theme_btn.icon = ft.Icons.LIGHT_MODE
            settings["theme"] = "dark"
        try:
            save_global_settings(settings)
        except Exception:  # noqa: BLE001 - a failed theme save must not break UI
            logger.exception("Could not save theme setting")
        page.update()

    theme_btn.on_click = toggle_theme

    header = ft.Container(
        content=ft.Row(
            [
                ft.Text(
                    APP_NAME,
                    size=26,
                    weight=ft.FontWeight.BOLD,
                    color=ft.Colors.WHITE,
                ),
                ft.Text(
                    f"v{VERSION}",
                    size=12,
                    color=ft.Colors.WHITE_70,
                ),
                ft.Container(expand=True),
                theme_btn,
            ]
        ),
        padding=ft.Padding.symmetric(horizontal=20, vertical=10),
        bgcolor=ft.Colors.BLUE_700,
    )

    def open_log_folder(event=None) -> None:
        def _on_error(message: str) -> None:
            log(message, is_error=True, level="error")
            show_snack(page, message, is_error=True)

        if open_folder(log_dir, on_error=_on_error):
            show_snack(page, "Opened log folder")

    log_header = ft.Row(
        [
            ft.Text("Activity log", size=13, color=ft.Colors.ON_SURFACE),
            ft.Container(expand=True),
            ft.IconButton(
                ft.Icons.DELETE_SWEEP,
                tooltip="Clear log",
                icon_size=18,
                on_click=clear_log,
            ),
            ft.IconButton(
                ft.Icons.FOLDER_OPEN,
                tooltip="Open log folder",
                icon_size=18,
                on_click=open_log_folder,
            ),
        ]
    )

    report_card = ft.Container(
        content=ft.Column(
            [
                ft.Text(
                    "Progress & Report",
                    weight=ft.FontWeight.BOLD,
                    size=16,
                    color=ft.Colors.ON_SURFACE,
                ),
                ft.Container(content=phase_text, height=24),
                progress_host,
                ft.Container(content=progress_text, height=24),
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                log_header,
                ft.Container(
                    content=log_col,
                    padding=12,
                    bgcolor=ft.Colors.SURFACE_CONTAINER_HIGHEST,
                    border_radius=8,
                    expand=True,
                ),
            ],
            spacing=8,
            expand=True,
        ),
        padding=20,
        bgcolor=ft.Colors.SURFACE,
        border_radius=12,
        border=ft.Border.all(1, ft.Colors.OUTLINE_VARIANT),
        expand=2,
    )

    async def ui_consumer() -> None:
        """Process UI callbacks from worker threads on the Flet thread."""
        while not state["shutdown_event"].is_set():
            callbacks = []
            try:
                while len(callbacks) < 100:
                    callbacks.append(ui_queue.get_nowait())
            except queue.Empty:
                pass

            if not callbacks:
                await asyncio.sleep(0.05)
                continue

            for callback in callbacks:
                try:
                    callback()
                except Exception:  # noqa: BLE001 - keep the UI loop alive
                    logger.exception("UI callback failed")

            try:
                page.update()
            except Exception:  # noqa: BLE001 - page may be closing
                pass

    def _unpack_progress(item):
        """Accept legacy 3-tuples and the new 4-tuple telemetry shape."""
        if isinstance(item, (tuple, list)):
            if len(item) >= 4:
                event = item[3] if isinstance(item[3], dict) else None
                return item[0], item[1], item[2], event
            if len(item) == 3:
                return item[0], item[1], item[2], None
        return 0, 0, "", None

    def _reset_progress() -> None:
        phase_text.value = "Idle"
        progress_text.value = ""
        progress_ring.visible = False
        progress_bar.visible = False
        progress_bar.value = 0

    def _apply_progress(current, total, message, event) -> None:
        if not message and not event:
            _reset_progress()
            return

        event = event or {}
        kind = event.get("kind", "progress")
        phase = event.get("phase")

        if kind == "summary" or phase == "done":
            phase_text.value = message or "Finished"
            progress_ring.visible = False
            progress_bar.visible = False
            progress_bar.value = 1 if total and current >= total else progress_bar.value
            progress_text.value = ""
            return

        if kind == "log":
            if phase in {"scan", "state"}:
                phase_text.value = message
                progress_ring.visible = True
                progress_bar.visible = False
                progress_text.value = ""
            return

        if kind == "phase":
            phase_text.value = message
            if event.get("indeterminate", total <= 0):
                progress_ring.visible = True
                progress_bar.visible = False
                progress_text.value = ""
            else:
                progress_ring.visible = False
                progress_bar.visible = True
                progress_bar.value = (
                    min(max(current / total, 0.0), 1.0) if total > 0 else None
                )
                progress_text.value = (
                    f"{current} / {total} ({progress_bar.value * 100:.1f}%)"
                    if total > 0 and progress_bar.value is not None
                    else ""
                )
            return

        # Ordinary progress update.
        if total > 0:
            value = min(max(current / total, 0.0), 1.0)
            progress_ring.visible = False
            progress_bar.visible = True
            progress_bar.value = value
            phase_text.value = message
            progress_text.value = f"{current} / {total} ({value * 100:.1f}%)"
        else:
            phase_text.value = message
            progress_ring.visible = True
            progress_bar.visible = False
            progress_text.value = ""

    async def progress_consumer() -> None:
        """Apply telemetry on the Flet thread, latest progress wins."""
        while not state["shutdown_event"].is_set():
            items = []
            try:
                while len(items) < 200:
                    items.append(progress_queue.get_nowait())
            except queue.Empty:
                pass

            if not items:
                await asyncio.sleep(0.15)
                continue

            latest_progress = None
            for item in items:
                current, total, message, event = _unpack_progress(item)
                if not message and not event:
                    latest_progress = None
                    _reset_progress()
                    continue
                kind = (event or {}).get("kind", "progress")
                if kind == "progress":
                    latest_progress = (current, total, message, event)
                    continue

                # Flush the latest progress before a later control event so a
                # "done" summary cannot be overwritten by stale progress.
                if latest_progress is not None:
                    _apply_progress(*latest_progress)
                    latest_progress = None
                _apply_progress(current, total, message, event)

            if latest_progress is not None:
                _apply_progress(*latest_progress)

            try:
                page.update()
            except Exception:  # noqa: BLE001 - page may be closing
                pass

            await asyncio.sleep(0.15)

    file_picker = ft.FilePicker()
    page.services.append(file_picker)

    phone_view = build_phone_view(
        page,
        state,
        log,
        file_picker,
        progress_queue,
        play_ding,
        post_ui,
    )
    takeout_view = build_organizer_view(
        page,
        state,
        log,
        file_picker,
        progress_queue,
        play_ding,
        post_ui,
    )
    dedup_view = build_dedup_view(
        page,
        state,
        log,
        file_picker,
        progress_queue,
        play_ding,
        post_ui,
    )

    tabs = ft.Tabs(
        length=3,
        selected_index=0,
        animation_duration=300,
        expand=True,
        content=ft.Column(
            expand=True,
            controls=[
                ft.TabBar(
                    tabs=[
                        ft.Tab(label="Phone Backup", icon=ft.Icons.PHONE_ANDROID),
                        ft.Tab(label="Special Organizer", icon=ft.Icons.AUTO_AWESOME),
                        ft.Tab(label="Duplicate Finder", icon=ft.Icons.CONTENT_COPY),
                    ]
                ),
                ft.TabBarView(
                    expand=True,
                    controls=[
                        ft.Container(content=phone_view, padding=10, expand=True),
                        ft.Container(content=takeout_view, padding=10, expand=True),
                        ft.Container(content=dedup_view, padding=10, expand=True),
                    ],
                ),
            ],
        ),
    )

    body = ft.Row(
        [ft.Container(content=tabs, expand=3), report_card],
        expand=True,
    )
    page.add(header, body)

    page.run_task(ui_consumer)
    page.run_task(progress_consumer)

    def on_close(event) -> None:
        state["shutdown_event"].set()
        state["cancel_event"].set()
        try:
            width = page.window.width
            height = page.window.height
            if width:
                settings["window_width"] = width
            if height:
                settings["window_height"] = height
            save_global_settings(settings)
        except Exception:  # noqa: BLE001 - closing must not fail
            logger.exception("Could not save window settings")

    page.on_close = on_close


if hasattr(ft, "run"):
    ft.run(main)
else:
    ft.app(target=main)
