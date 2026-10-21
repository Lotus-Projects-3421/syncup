import flet as ft
import subprocess
import os
import sys
from core.dedup_engine import run_dedup

def open_folder(path):
    if not path or not os.path.exists(path): return
    if sys.platform.startswith('darwin'): subprocess.call(('open', path))
    elif os.name == 'nt': os.startfile(path)
    elif os.name == 'posix': subprocess.call(('xdg-open', path))

def build_dedup_view(page: ft.Page, state: dict, log_func, file_picker, progress_queue, play_sound):
    d_state = {"scan_dir": ""}
    
    def card(content):
        return ft.Container(content=content, padding=16, bgcolor=ft.Colors.SURFACE, border_radius=12, border=ft.Border.all(1, ft.Colors.OUTLINE_VARIANT))

    d_input = ft.TextField(label="Folder to scan for duplicates", expand=True, read_only=True)
    
    async def d_pick_folder(e):
        path = await file_picker.get_directory_path(dialog_title="Select Folder to scan")
        if path: d_state["scan_dir"] = path; d_input.value = path; page.update()

    cancel_btn = ft.Button("Cancel", bgcolor=ft.Colors.RED_700, color=ft.Colors.WHITE, visible=False)
    start_btn = ft.Button("Find & Move Duplicates", bgcolor=ft.Colors.ORANGE_600, color=ft.Colors.WHITE, height=50)

    def d_cancel(e):
        state["cancel_event"].set()
        log_func("Cancellation requested. Stopping...", is_error=True)
    
    cancel_btn.on_click = d_cancel

    def d_start(e):
        if state["is_running"]: return
        if not d_state["scan_dir"]:
            log_func("Please select a folder first.", is_error=True); return
        
        log_func("Starting Duplicate Finder...")
        state["is_running"] = True
        state["cancel_event"].clear()
        start_btn.visible = False
        cancel_btn.visible = True
        page.update()
        
        def progress_cb(current, total, msg): progress_queue.put((current, total, msg))

        def run():
            try:
                res = run_dedup(d_state["scan_dir"], progress_cb, state["cancel_event"])
                if "error" in res: log_func(res["error"], is_error=True)
                else: 
                    log_func(f"✅ Deduplication complete! {res['moved']} duplicates moved to _Duplicates folder.")
                    play_sound()
            except Exception as ex: log_func(f"Error: {ex}", is_error=True)
            finally:
                state["is_running"] = False
                start_btn.visible = True
                cancel_btn.visible = False
                progress_queue.put((0, 0, ""))
                page.update()
        page.run_thread(run)
    
    start_btn.on_click = d_start

    return ft.Column([
        card(ft.Column([
            ft.Text("Duplicate Finder", weight="bold", size=16),
            ft.Text("Scans a folder deeply and uses MD5 hashing to find exact copies. To ensure safety, duplicates are NOT deleted; they are moved to a '_Duplicates_<timestamp>' subfolder.", size=12),
        ], spacing=4)),
        card(ft.Column([
            ft.Text("1. Folder to Scan", weight="bold"), 
            ft.Row([d_input, ft.IconButton(ft.Icons.FOLDER_OPEN, tooltip="Open Folder", on_click=lambda e: open_folder(d_state["scan_dir"]))]),
            ft.Button("Browse Folder", on_click=d_pick_folder)
        ])),
        ft.Row([ft.Container(expand=True), start_btn, cancel_btn, ft.Container(expand=True)]),
    ], scroll=ft.ScrollMode.AUTO)
