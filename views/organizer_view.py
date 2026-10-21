import flet as ft
import subprocess
import os
import sys
from core.takeout_engine import run_takeout

def open_folder(path):
    if not path or not os.path.exists(path): return
    if sys.platform.startswith('darwin'): subprocess.call(('open', path))
    elif os.name == 'nt': os.startfile(path)
    elif os.name == 'posix': subprocess.call(('xdg-open', path))

def build_organizer_view(page: ft.Page, state: dict, log_func, file_picker, progress_queue, play_sound):
    t_state = {"inputs": [], "destination": "", "sorting": "year", "filter": "both", "conflict": "keep"}
    
    def card(content):
        return ft.Container(content=content, padding=16, bgcolor=ft.Colors.SURFACE, border_radius=12, border=ft.Border.all(1, ft.Colors.OUTLINE_VARIANT))

    t_input = ft.TextField(label="Input (Folder or ZIPs)", expand=True, read_only=True)
    t_dest = ft.TextField(label="Destination folder", expand=True, read_only=True)
    
    async def t_pick_folder(e):
        path = await file_picker.get_directory_path(dialog_title="Select Input Folder")
        if path: t_state["inputs"] = [path]; t_input.value = f"Folder: {path}"; page.update()

    async def t_pick_zips(e):
        files = await file_picker.pick_files(dialog_title="Select ZIP files", allow_multiple=True, allowed_extensions=["zip"])
        if files:
            paths = [f.path for f in files]
            t_state["inputs"] = paths
            t_input.value = f"ZIP: {paths[0]}" if len(paths) == 1 else f"{len(paths)} ZIP files selected"
            page.update()

    async def t_pick_dest(e):
        path = await file_picker.get_directory_path(dialog_title="Select Destination folder")
        if path: t_state["destination"] = path; t_dest.value = path; page.update()

    t_sorting = ft.RadioGroup(
        content=ft.Row([ft.Radio(value="day", label="Day"), ft.Radio(value="month", label="Month"), ft.Radio(value="year", label="Year")]),
        value="year", on_change=lambda e: t_state.update({"sorting": e.control.value})
    )

    t_filter = ft.RadioGroup(
        content=ft.Row([ft.Radio(value="both", label="Both"), ft.Radio(value="photos", label="Photos"), ft.Radio(value="videos", label="Videos")]),
        value="both", on_change=lambda e: t_state.update({"filter": e.control.value})
    )
    
    t_conflict = ft.RadioGroup(
        content=ft.Row([ft.Radio(value="keep", label="Keep Both (Add __1)"), ft.Radio(value="overwrite", label="Overwrite Existing")]),
        value="keep", on_change=lambda e: t_state.update({"conflict": e.control.value})
    )

    cancel_btn = ft.Button("Cancel", bgcolor=ft.Colors.RED_700, color=ft.Colors.WHITE, visible=False)
    start_btn = ft.Button("Start Special Organizer", bgcolor=ft.Colors.BLUE_600, color=ft.Colors.WHITE, height=50)

    def t_cancel(e):
        state["cancel_event"].set()
        log_func("Cancellation requested. Stopping...", is_error=True)
    
    cancel_btn.on_click = t_cancel

    def t_start(e):
        if state["is_running"]: return
        if not t_state["inputs"] or not t_state["destination"]:
            log_func("Missing inputs for Organizer", is_error=True); return
        log_func("Starting Special Organizer...")
        
        state["is_running"] = True
        state["cancel_event"].clear()
        start_btn.visible = False
        cancel_btn.visible = True
        page.update()
        
        def progress_cb(current, total, msg): progress_queue.put((current, total, msg))

        def run():
            try:
                res = run_takeout(t_state["inputs"], t_state["destination"], t_state["sorting"], t_state["filter"], t_state["conflict"], progress_cb, state["cancel_event"])
                if "error" in res: log_func(res["error"], is_error=True)
                else: 
                    log_func(f"✅ Organizer complete: {res['processed']} files organized.")
                    play_sound()
            except Exception as ex: log_func(f"Error: {ex}", is_error=True)
            finally:
                state["is_running"] = False
                start_btn.visible = True
                cancel_btn.visible = False
                progress_queue.put((0, 0, ""))
                page.update()
        page.run_thread(run)
    
    start_btn.on_click = t_start

    return ft.Column([
        card(ft.Column([
            ft.Text("Special Organizer", weight="bold", size=16),
            ft.Text("Google Takeout + Personal Folder Organizer", weight="bold", size=12),
        ], spacing=4)),
        card(ft.Column([
            ft.Text("1. Input Source", weight="bold"), 
            t_input, 
            ft.Row([
                ft.Button("Browse Folder", on_click=t_pick_folder),
                ft.Button("Browse ZIPs (Multiple)", on_click=t_pick_zips, bgcolor=ft.Colors.BLUE_700, color=ft.Colors.WHITE),
            ], spacing=10)
        ])),
        card(ft.Column([
            ft.Text("2. Destination Folder", weight="bold"), 
            ft.Row([t_dest, ft.IconButton(ft.Icons.FOLDER_OPEN, tooltip="Open Destination", on_click=lambda e: open_folder(t_state["destination"]))]),
            ft.Button("Browse Dest", on_click=t_pick_dest)
        ])),
        card(ft.Column([ft.Text("3. Filter", weight="bold"), t_filter])),
        card(ft.Column([ft.Text("4. Conflict Resolution", weight="bold"), t_conflict])),
        card(ft.Column([ft.Text("5. Sorting", weight="bold"), t_sorting])),
        ft.Row([ft.Container(expand=True), start_btn, cancel_btn, ft.Container(expand=True)]),
    ], scroll=ft.ScrollMode.AUTO)
