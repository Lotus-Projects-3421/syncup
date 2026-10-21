import flet as ft
import time
import subprocess
import os
import sys
from core.device import get_connected_devices, get_folders
from core.engine import run_backup
from core.storage import load_profile, save_profile

def open_folder(path):
    if not path or not os.path.exists(path): return
    if sys.platform.startswith('darwin'): subprocess.call(('open', path))
    elif os.name == 'nt': os.startfile(path)
    elif os.name == 'posix': subprocess.call(('xdg-open', path))

def build_phone_view(page: ft.Page, state: dict, log_func, file_picker, progress_queue, play_sound):
    p_state = {"device": None, "current_phone_path": "", "folders": set(), "staging": "", "destination": "", "sorting": "year", "filter": "both", "conflict": "keep", "last_device_count": -1}
    
    def card(content):
        return ft.Container(content=content, padding=16, bgcolor=ft.Colors.SURFACE, border_radius=12, border=ft.Border.all(1, ft.Colors.OUTLINE_VARIANT))

    p_staging = ft.TextField(label="Staging folder", expand=True, read_only=True)
    p_dest = ft.TextField(label="Final destination", expand=True, read_only=True)

    async def p_pick_staging(e):
        path = await file_picker.get_directory_path()
        if path: p_state["staging"] = path; p_staging.value = path; page.update()

    async def p_pick_dest(e):
        path = await file_picker.get_directory_path()
        if path: p_state["destination"] = path; p_dest.value = path; page.update()

    p_sorting = ft.RadioGroup(
        content=ft.Row([ft.Radio(value="day", label="Day"), ft.Radio(value="month", label="Month"), ft.Radio(value="year", label="Year"), ft.Radio(value="flat", label="Flat")]),
        value="year", on_change=lambda e: p_state.update({"sorting": e.control.value})
    )

    p_filter = ft.RadioGroup(
        content=ft.Row([ft.Radio(value="both", label="Photos & Videos"), ft.Radio(value="photos", label="Photos Only"), ft.Radio(value="videos", label="Videos Only")]),
        value="both", on_change=lambda e: p_state.update({"filter": e.control.value})
    )

    p_conflict = ft.RadioGroup(
        content=ft.Row([ft.Radio(value="keep", label="Keep Both (Add __1)"), ft.Radio(value="overwrite", label="Overwrite Existing")]),
        value="keep", on_change=lambda e: p_state.update({"conflict": e.control.value})
    )

    p_dev_list = ft.Column(spacing=6)
    p_dev_status = ft.Text("Scanning...", size=13)
    
    def p_refresh_devices(manual=False):
        if state["is_running"]: return
        devices = get_connected_devices()
        if len(devices) == p_state["last_device_count"] and not manual: return
        p_state["last_device_count"] = len(devices)
        p_dev_list.controls.clear()
        for d in devices:
            p_dev_list.controls.append(ft.Container(content=ft.Row([
                ft.Icon(ft.Icons.SMARTPHONE, size=20), ft.Text(d["name"], size=13),
                ft.Container(expand=True), ft.Button("Select", on_click=lambda e, dev=d: p_select_device(dev)),
            ]), padding=8, bgcolor=ft.Colors.SURFACE_CONTAINER_HIGHEST))
        p_dev_status.value = f"{len(devices)} device(s) detected" if devices else "No devices."
        page.update()

    def p_select_device(dev):
        if state["is_running"]: return
        p_state["device"] = dev
        p_state["current_phone_path"] = ""
        p_dev_status.value = f"Selected: {dev['name']}"
        profile = load_profile(dev["id"])
        if profile:
            p_state.update(profile)
            p_staging.value = profile.get("staging", "")
            p_dest.value = profile.get("destination", "")
            p_sorting.value = profile.get("sorting", "year")
            p_filter.value = profile.get("filter", "both")
            p_conflict.value = profile.get("conflict", "keep")
            log_func("✨ Profile loaded!")
        p_rebuild_folders()

    p_folder_list = ft.Column()
    p_breadcrumb = ft.Text("📁 Root", size=12)
    
    def p_rebuild_folders():
        p_folder_list.controls.clear()
        if not p_state["device"]: return
        p_breadcrumb.value = f"📁 {p_state['current_phone_path'] or 'Root'}"
        folders = get_folders(p_state["device"]["path"], p_state["current_phone_path"])
        for f in folders:
            full_rel = f"{p_state['current_phone_path']}/{f}".lstrip("/")
            p_folder_list.controls.append(ft.Row([
                ft.Checkbox(label=f, value=(full_rel in p_state["folders"]), on_change=lambda e, fr=full_rel: p_toggle(fr, e.control.value), expand=True),
                ft.IconButton(ft.Icons.CHEVRON_RIGHT, on_click=lambda e, fn=f: p_nav(fn))
            ]))
        page.update()

    def p_nav(fn): p_state["current_phone_path"] = f"{p_state['current_phone_path']}/{fn}".lstrip("/"); p_rebuild_folders()
    def p_toggle(fr, c): p_state["folders"].add(fr) if c else p_state["folders"].discard(fr); p_rebuild_folders()

    cancel_btn = ft.Button("Cancel", bgcolor=ft.Colors.RED_700, color=ft.Colors.WHITE, visible=False)
    start_btn = ft.Button("Start Phone Backup", bgcolor=ft.Colors.GREEN_600, color=ft.Colors.WHITE, height=50)

    def p_cancel(e):
        state["cancel_event"].set()
        log_func("Cancellation requested. Stopping after current file...", is_error=True)
    
    cancel_btn.on_click = p_cancel

    def p_start(e):
        if state["is_running"]: return
        if not p_state["device"] or not p_state["folders"] or not p_state["staging"] or not p_state["destination"]:
            log_func("Missing settings for Phone Backup", is_error=True); return
        
        save_profile(p_state["device"]["id"], {"staging": p_state["staging"], "destination": p_state["destination"], "sorting": p_state["sorting"], "filter": p_state["filter"], "conflict": p_state["conflict"], "folders": list(p_state["folders"])})
        log_func("Starting Phone Backup...")
        
        state["is_running"] = True
        state["cancel_event"].clear()
        start_btn.visible = False
        cancel_btn.visible = True
        page.update()
        
        def progress_cb(current, total, msg): progress_queue.put((current, total, msg))

        def run():
            try:
                res = run_backup(p_state["device"]["id"], p_state["device"]["path"], list(p_state["folders"]), p_state["staging"], p_state["destination"], p_state["sorting"], p_state["filter"], p_state["conflict"], progress_cb, state["cancel_event"])
                if "error" in res: log_func(res["error"], is_error=True)
                else: 
                    log_func(f"✅ Phone backup complete! {res['processed']} files copied.")
                    play_sound()
            except Exception as ex: log_func(f"Error: {ex}", is_error=True)
            finally:
                state["is_running"] = False
                start_btn.visible = True
                cancel_btn.visible = False
                progress_queue.put((0, 0, ""))
                page.update()
        page.run_thread(run)

    start_btn.on_click = p_start

    def auto_scan():
        while not state["shutdown_event"].is_set():
            time.sleep(3)
            p_refresh_devices()
    page.run_thread(auto_scan)
    p_refresh_devices(manual=True)

    return ft.Column([
        card(ft.Column([ft.Text("1. Select Device", weight="bold"), p_dev_status, p_dev_list])),
        card(ft.Column([ft.Text("2. Folders", weight="bold"), p_breadcrumb, p_folder_list])),
        card(ft.Column([
            ft.Text("3. Paths", weight="bold"), 
            p_staging, ft.Button("Browse Staging", on_click=p_pick_staging), 
            ft.Row([p_dest, ft.IconButton(ft.Icons.FOLDER_OPEN, tooltip="Open Destination", on_click=lambda e: open_folder(p_state["destination"]))]),
            ft.Button("Browse Dest", on_click=p_pick_dest)
        ])),
        card(ft.Column([ft.Text("4. Filter", weight="bold"), p_filter])),
        card(ft.Column([ft.Text("5. Conflict Resolution", weight="bold"), p_conflict])),
        card(ft.Column([ft.Text("6. Sorting", weight="bold"), p_sorting])),
        ft.Row([ft.Container(expand=True), start_btn, cancel_btn, ft.Container(expand=True)]),
    ], scroll=ft.ScrollMode.AUTO)
