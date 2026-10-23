"""Phone backup tab."""

from __future__ import annotations

import time

import flet as ft

from core.device import get_connected_devices, get_folders
from core.engine import run_backup
from core.storage import load_profile, save_profile
from views.common import (
    begin_job,
    card,
    finish_job,
    open_folder,
    progress_callback_for,
    register_job,
    show_scoreboard,
    show_snack,
)


def build_phone_view(
    page: ft.Page,
    state: dict,
    log_func,
    file_picker,
    progress_queue,
    play_sound,
    post_ui,
):
    p_state = {
        "device": None,
        "current_phone_path": "",
        "folders": set(),
        "folder_cache": {},
        "destination": "",
        "sorting": "year",
        "filter": "both",
        "conflict": "keep",
        "device_signature": None,
    }

    p_dest = ft.TextField(label="Destination folder", expand=True, read_only=True)

    def p_open_destination(event=None) -> None:
        def _on_error(message: str) -> None:
            log_func(message, is_error=True, level="error")
            show_snack(page, message, is_error=True)

        if open_folder(p_state["destination"], on_error=_on_error):
            show_snack(page, "Opened in file manager")

    async def p_pick_dest(e):
        path = await file_picker.get_directory_path(dialog_title="Select Destination")
        if path:
            p_state["destination"] = path
            p_dest.value = path
            page.update()

    def _set_sorting(event):
        p_state["sorting"] = event.control.value

    def _set_filter(event):
        p_state["filter"] = event.control.value

    def _set_conflict(event):
        p_state["conflict"] = event.control.value

    p_sorting = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="day", label="Day"),
                ft.Radio(value="month", label="Month"),
                ft.Radio(value="year", label="Year"),
                ft.Radio(value="flat", label="Flat"),
            ]
        ),
        value="year",
        on_change=_set_sorting,
    )

    p_filter = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="both", label="Photos & Videos"),
                ft.Radio(value="photos", label="Photos Only"),
                ft.Radio(value="videos", label="Videos Only"),
            ]
        ),
        value="both",
        on_change=_set_filter,
    )

    p_conflict = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="keep", label="Keep Both (Add __1)"),
                ft.Radio(value="overwrite", label="Overwrite Existing"),
            ]
        ),
        value="keep",
        on_change=_set_conflict,
    )

    p_dev_list = ft.Column(spacing=6)
    p_dev_status = ft.Text("Scanning...", size=13)

    p_breadcrumb = ft.Text("📁 Root", size=12)
    p_folder_list = ft.Column(spacing=2)
    p_up_btn = ft.IconButton(
        ft.Icons.ARROW_UPWARD,
        tooltip="Up one folder",
        disabled=True,
    )
    p_folder_refresh_btn = ft.IconButton(
        ft.Icons.REFRESH,
        tooltip="Refresh this folder",
    )

    def _apply_devices(devices: list[dict]) -> None:
        signature = tuple(sorted(device["id"] for device in devices))
        p_state["device_signature"] = signature
        p_dev_list.controls.clear()
        for device in devices:
            p_dev_list.controls.append(
                ft.Container(
                    content=ft.Row(
                        [
                            ft.Icon(ft.Icons.SMARTPHONE, size=20),
                            ft.Text(device["name"], size=13),
                            ft.Container(expand=True),
                            ft.Button(
                                content="Select",
                                on_click=lambda e, dev=device: p_select_device(dev),
                            ),
                        ]
                    ),
                    padding=8,
                    bgcolor=ft.Colors.SURFACE_CONTAINER_HIGHEST,
                    border_radius=8,
                )
            )

        connected_ids = {device["id"] for device in devices}
        if p_state["device"] and p_state["device"]["id"] not in connected_ids:
            p_state["device"] = None
            p_state["folders"] = set()
            p_state["folder_cache"].clear()
            p_state["current_phone_path"] = ""
            p_folder_list.controls.clear()
            p_breadcrumb.value = "📁 Root"
            p_up_btn.disabled = True
            p_dev_status.value = "Selected device disconnected. Reconnect it and refresh."
        elif p_state["device"]:
            p_dev_status.value = f"Selected: {p_state['device']['name']}"
        elif devices:
            p_dev_status.value = f"{len(devices)} device(s) detected"
        else:
            p_dev_status.value = "No Android device detected."

        page.update()

    def p_refresh_devices(manual: bool = False) -> None:
        if state["is_running"]:
            return
        try:
            devices = get_connected_devices(force_mount=manual)
        except Exception as exc:  # noqa: BLE001 - discovery must not kill the UI
            log_func(f"Could not scan for Android devices: {exc}", is_error=True)
            return
        signature = tuple(sorted(device["id"] for device in devices))
        if not manual and signature == p_state["device_signature"]:
            return
        post_ui(lambda: _apply_devices(devices))

    def _render_folders(relative_path: str, folders: list[str]) -> None:
        if relative_path != p_state["current_phone_path"]:
            return
        p_breadcrumb.value = f"📁 {relative_path or 'Root'}"
        p_up_btn.disabled = not relative_path
        p_folder_list.controls.clear()

        if not folders:
            p_folder_list.controls.append(
                ft.Text("No subfolders.", size=12, color=ft.Colors.GREY_500)
            )

        for name in folders:
            full_rel = f"{relative_path}/{name}".strip("/")
            p_folder_list.controls.append(
                ft.Row(
                    [
                        ft.Checkbox(
                            label=name,
                            value=full_rel in p_state["folders"],
                            on_change=lambda e, fr=full_rel: p_toggle(fr, e.control.value),
                            expand=True,
                        ),
                        ft.IconButton(
                            ft.Icons.CHEVRON_RIGHT,
                            tooltip="Open folder",
                            on_click=lambda e, folder_name=name: p_nav(folder_name),
                        ),
                    ]
                )
            )
        page.update()

    def _load_folders(device_path: str, relative_path: str) -> None:
        try:
            folders = get_folders(device_path, relative_path)
        except Exception as exc:  # noqa: BLE001 - a bad folder must not kill the UI
            folders = []
            log_func(
                f"Could not read {relative_path or 'the phone root'}: {exc}",
                is_error=True,
            )
        p_state["folder_cache"][relative_path] = folders
        post_ui(lambda: _render_folders(relative_path, folders))

    def p_rebuild_folders() -> None:
        if not p_state["device"]:
            return
        relative_path = p_state["current_phone_path"]
        p_up_btn.disabled = not relative_path

        cached = p_state["folder_cache"].get(relative_path)
        if cached is not None:
            _render_folders(relative_path, cached)
            return

        p_breadcrumb.value = f"📁 {relative_path or 'Root'} — loading..."
        p_folder_list.controls.clear()
        page.update()
        device_path = p_state["device"]["path"]
        page.run_thread(lambda: _load_folders(device_path, relative_path))

    def p_nav(folder_name: str) -> None:
        p_state["current_phone_path"] = (
            f"{p_state['current_phone_path']}/{folder_name}".strip("/")
        )
        p_rebuild_folders()

    def p_up(event) -> None:
        current = p_state["current_phone_path"]
        if not current:
            return
        p_state["current_phone_path"] = "/".join(current.split("/")[:-1])
        p_rebuild_folders()

    def p_reload_folder(event=None) -> None:
        if not p_state["device"]:
            return
        p_state["folder_cache"].pop(p_state["current_phone_path"], None)
        p_rebuild_folders()

    p_up_btn.on_click = p_up
    p_folder_refresh_btn.on_click = p_reload_folder

    def p_toggle(full_rel: str, checked: bool) -> None:
        if checked:
            p_state["folders"].add(full_rel)
        else:
            p_state["folders"].discard(full_rel)

    def p_select_device(device: dict) -> None:
        if state["is_running"]:
            return
        p_state["device"] = device
        p_state["current_phone_path"] = ""
        p_state["folder_cache"].clear()
        p_state["folders"] = set()
        p_dev_status.value = f"Selected: {device['name']}"

        profile = load_profile(device["id"])
        if profile:
            p_state["destination"] = str(profile.get("destination") or "")
            sorting = profile.get("sorting")
            p_state["sorting"] = (
                sorting if sorting in {"day", "month", "year", "flat"} else "year"
            )
            media_filter = profile.get("filter")
            p_state["filter"] = (
                media_filter if media_filter in {"both", "photos", "videos"} else "both"
            )
            conflict = profile.get("conflict")
            p_state["conflict"] = (
                conflict if conflict in {"keep", "overwrite"} else "keep"
            )
            raw_folders = profile.get("folders")
            if isinstance(raw_folders, (list, tuple, set)):
                p_state["folders"] = {
                    str(folder).strip("/")
                    for folder in raw_folders
                    if str(folder).strip("/")
                }
            else:
                p_state["folders"] = set()
            p_dest.value = p_state["destination"]
            p_sorting.value = p_state["sorting"]
            p_filter.value = p_state["filter"]
            p_conflict.value = p_state["conflict"]
            log_func("Profile loaded.")

        page.update()
        p_rebuild_folders()

    start_btn = ft.Button(
        content="Start Phone Backup",
        bgcolor=ft.Colors.GREEN_600,
        color=ft.Colors.WHITE,
        height=50,
    )
    cancel_btn = ft.Button(
        content="Cancel",
        bgcolor=ft.Colors.RED_700,
        color=ft.Colors.WHITE,
        visible=False,
    )

    def p_cancel(event) -> None:
        state["cancel_event"].set()
        cancel_btn.disabled = True
        cancel_btn.content = "Cancelling..."
        log_func("Cancellation requested. Finishing the current file...")
        page.update()

    cancel_btn.on_click = p_cancel

    def _finish(result: dict, destination: str) -> None:
        finish_job(state)
        if result.get("error"):
            log_func(result["error"], is_error=True)
        elif result.get("cancelled"):
            log_func(
                "Backup cancelled: "
                f"{result.get('processed', 0)} copied, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors.",
                level="warning",
            )
        else:
            log_func(
                f"✅ Phone backup finished in {result.get('time', '?')}: "
                f"{result.get('processed', 0)} copied, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors.",
                level="success",
            )
            if result.get("total", 0) == 0:
                if result.get("errors", 0):
                    log_func(
                        "No new files were copied because some files could not "
                        "be read. See the errors above.",
                        level="warning",
                    )
                else:
                    log_func("Nothing new to copy.", level="warning")

        if (
            not result.get("error")
            and not result.get("cancelled")
            and result.get("processed", 0) > 0
        ):
            play_sound()

        progress_queue.put((0, 0, ""))
        show_scoreboard(
            page,
            result,
            title="Phone Backup Summary",
            destination=destination,
            operation="backup",
            log_func=log_func,
        )
        page.update()

    def p_start(event) -> None:
        if state["is_running"]:
            return
        if (
            not p_state["device"]
            or not p_state["folders"]
            or not p_state["destination"]
        ):
            log_func("Missing device, folders, or destination for Phone Backup.", is_error=True)
            return

        # Snapshot the job inputs before starting the worker. The user can
        # still navigate the folder tree, but the running job must not change.
        device_id = p_state["device"]["id"]
        device_path = p_state["device"]["path"]
        folders = sorted(p_state["folders"])
        destination = p_state["destination"]
        sorting = p_state["sorting"]
        media_filter = p_state["filter"]
        conflict = p_state["conflict"]

        try:
            save_profile(
                device_id,
                {
                    "destination": destination,
                    "sorting": sorting,
                    "filter": media_filter,
                    "conflict": conflict,
                    "folders": folders,
                },
            )
        except Exception as exc:  # noqa: BLE001 - profile is a convenience
            log_func(f"Could not save profile: {exc}", is_error=True)

        log_func("Starting Phone Backup...", level="organize")
        begin_job(state, cancel_btn)
        page.update()

        progress_cb = progress_callback_for(progress_queue, log_func)

        def error_cb(name, message):
            log_func(f"Failed: {name}: {message}", is_error=True)

        def run():
            result: dict = {}
            try:
                result = run_backup(
                    device_id,
                    device_path,
                    folders,
                    destination,
                    sorting,
                    media_filter,
                    conflict,
                    progress_cb,
                    state["cancel_event"],
                    error_callback=error_cb,
                )
            except Exception as exc:  # noqa: BLE001 - surface to the user
                log_func(f"Error: {exc}", is_error=True)
                result = {"error": str(exc)}
            finally:
                post_ui(lambda: _finish(result, destination))

        page.run_thread(run)

    start_btn.on_click = p_start
    register_job(state, start_btn, cancel_btn)

    def auto_scan() -> None:
        while not state["shutdown_event"].is_set():
            time.sleep(20)
            try:
                p_refresh_devices()
            except Exception:  # noqa: BLE001 - polling must not kill the thread
                pass

    page.run_thread(auto_scan)
    page.run_thread(lambda: p_refresh_devices(manual=True))

    return ft.Column(
        [
            card(
                ft.Column(
                    [
                        ft.Row(
                            [
                                ft.Text("1. Select Device", weight="bold"),
                                ft.Container(expand=True),
                                ft.IconButton(
                                    ft.Icons.REFRESH,
                                    tooltip="Rescan devices",
                                    on_click=lambda e: page.run_thread(
                                        lambda: p_refresh_devices(manual=True)
                                    ),
                                ),
                            ]
                        ),
                        p_dev_status,
                        p_dev_list,
                    ]
                )
            ),
            card(
                ft.Column(
                    [
                        ft.Row(
                            [
                                ft.Text("2. Folders", weight="bold"),
                                ft.Container(expand=True),
                                p_folder_refresh_btn,
                                p_up_btn,
                            ]
                        ),
                        p_breadcrumb,
                        p_folder_list,
                    ]
                )
            ),
            card(
                ft.Column(
                    [
                        ft.Text("3. Destination", weight="bold"),
                        ft.Row(
                            [
                                p_dest,
                                ft.IconButton(
                                    ft.Icons.FOLDER_OPEN,
                                    tooltip="Open Destination",
                                    on_click=p_open_destination,
                                ),
                            ]
                        ),
                        ft.Button("Browse Destination", on_click=p_pick_dest),
                        ft.Text(
                            "Temporary files are written next to the destination "
                            "and renamed only after verification.",
                            size=11,
                            color=ft.Colors.GREY_500,
                        ),
                    ]
                )
            ),
            card(
                ft.Column(
                    [ft.Text("4. Filter", weight="bold"), p_filter]
                )
            ),
            card(
                ft.Column(
                    [ft.Text("5. Conflict Resolution", weight="bold"), p_conflict]
                )
            ),
            card(
                ft.Column(
                    [ft.Text("6. Sorting", weight="bold"), p_sorting]
                )
            ),
            ft.Row(
                [
                    ft.Container(expand=True),
                    start_btn,
                    cancel_btn,
                    ft.Container(expand=True),
                ]
            ),
        ],
        scroll=ft.ScrollMode.AUTO,
    )
