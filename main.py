import flet as ft
import time
import threading
import queue
import asyncio
import subprocess
import os
from datetime import datetime
from core.storage import load_global_settings, save_global_settings
from views.phone_view import build_phone_view
from views.organizer_view import build_organizer_view
from views.dedup_view import build_dedup_view

def play_ding():
    try:
        if os.name == 'nt':
            import winsound; winsound.Beep(1000, 500)
        elif os.uname().sysname == 'Darwin':
            subprocess.run(["afplay", "/System/Library/Sounds/Glass.aiff"])
        else:
            subprocess.run(["canberra-gtk-play", "-i", "complete"], capture_output=True)
    except: pass

def main(page: ft.Page):
    page.title = "SyncUp — Backup & Organizer"
    page.padding = 0
    page.theme_mode = ft.ThemeMode.DARK
    page.theme = ft.Theme(color_scheme_seed=ft.Colors.BLUE)
    page.dark_theme = ft.Theme(color_scheme_seed=ft.Colors.BLUE)

    # Load saved window size
    settings = load_global_settings()
    try:
        page.window.width = settings.get("window_width", 1200)
        page.window.height = settings.get("window_height", 950)
    except: pass

    state = {"is_running": False, "shutdown_event": threading.Event(), "cancel_event": threading.Event()}
    progress_queue = queue.Queue()

    def log(msg, is_error=False):
        if state["shutdown_event"].is_set(): return
        ts = datetime.now().strftime("%H:%M:%S")
        color = ft.Colors.RED_400 if is_error else ft.Colors.ON_SURFACE
        log_col.controls.append(ft.Text(f"[{ts}] {msg}", size=13, font_family="monospace", color=color))
        page.update()

    def toggle_theme(e):
        if page.theme_mode == ft.ThemeMode.DARK:
            page.theme_mode = ft.ThemeMode.LIGHT; theme_btn.icon = ft.Icons.DARK_MODE
        else:
            page.theme_mode = ft.ThemeMode.DARK; theme_btn.icon = ft.Icons.LIGHT_MODE
        page.update()

    theme_btn = ft.IconButton(icon=ft.Icons.LIGHT_MODE, icon_color=ft.Colors.WHITE, on_click=toggle_theme)
    header = ft.Container(
        content=ft.Row([
            ft.Text("SyncUp", size=26, weight=ft.FontWeight.BOLD, color=ft.Colors.WHITE),
            ft.Container(expand=True), theme_btn,
        ]),
        padding=ft.Padding.symmetric(horizontal=20, vertical=10), bgcolor=ft.Colors.BLUE_700,
    )

    progress_text = ft.Text("", size=16, weight=ft.FontWeight.BOLD, color=ft.Colors.GREEN_400, font_family="monospace")
    log_col = ft.Column(spacing=4, scroll=ft.ScrollMode.AUTO, expand=True)
    
    report_card = ft.Container(
        content=ft.Column([
            ft.Text("Progress & Report", weight=ft.FontWeight.BOLD, size=16, color=ft.Colors.ON_SURFACE),
            ft.Container(content=progress_text, height=30),
            ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
            ft.Text("Activity log", size=13, color=ft.Colors.ON_SURFACE),
            ft.Container(content=log_col, padding=12, bgcolor=ft.Colors.SURFACE_CONTAINER_HIGHEST, border_radius=8, expand=True),
        ], spacing=8, expand=True),
        padding=20, bgcolor=ft.Colors.SURFACE, border_radius=12, border=ft.Border.all(1, ft.Colors.OUTLINE_VARIANT),
        expand=2 
    )

    async def queue_consumer():
        while not state["shutdown_event"].is_set():
            try:
                current, total, msg = progress_queue.get_nowait()
                if total > 0:
                    pct = (current / total) * 100
                    progress_text.value = f"Progress: {current} / {total} ({pct:.1f}%)"
                else:
                    progress_text.value = msg
                progress_text.update()
            except queue.Empty:
                await asyncio.sleep(0.2)
            except Exception:
                pass
    page.run_task(queue_consumer)

    p_file_picker = ft.FilePicker()
    page.services.append(p_file_picker)

    # Build all 3 views
    phone_view = build_phone_view(page, state, log, p_file_picker, progress_queue, play_ding)
    takeout_view = build_organizer_view(page, state, log, p_file_picker, progress_queue, play_ding)
    dedup_view = build_dedup_view(page, state, log, p_file_picker, progress_queue, play_ding)

    tabs = ft.Tabs(
        length=3, selected_index=0, animation_duration=300, expand=True,
        content=ft.Column(expand=True, controls=[
            ft.TabBar(tabs=[
                ft.Tab(label="Phone Backup", icon=ft.Icons.PHONE_ANDROID),
                ft.Tab(label="Special Organizer", icon=ft.Icons.AUTO_AWESOME),
                ft.Tab(label="Duplicate Finder", icon=ft.Icons.CONTENT_COPY),
            ]),
            ft.TabBarView(expand=True, controls=[
                ft.Container(content=phone_view, padding=10, expand=True),
                ft.Container(content=takeout_view, padding=10, expand=True),
                ft.Container(content=dedup_view, padding=10, expand=True),
            ]),
        ]),
    )

    body = ft.Row([ft.Container(content=tabs, expand=3), report_card], expand=True)
    page.add(header, body)

    def on_close(e):
        state["shutdown_event"].set()
        # Save window size
        try:
            settings["window_width"] = page.window.width
            settings["window_height"] = page.window.height
            save_global_settings(settings)
        except: pass
        
    page.on_close = on_close

if hasattr(ft, "run"):
    ft.run(main)
else:
    ft.app(target=main)
