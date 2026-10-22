"""Google Takeout / folder organizer tab."""

from __future__ import annotations

import flet as ft

from core.takeout_engine import run_takeout
from views.common import (
    begin_job,
    card,
    finish_job,
    open_folder,
    register_job,
)


def build_organizer_view(
    page: ft.Page,
    state: dict,
    log_func,
    file_picker,
    progress_queue,
    play_sound,
    post_ui,
):
    t_state = {
        "inputs": [],
        "destination": "",
        "sorting": "year",
        "filter": "both",
        "conflict": "keep",
        "preserve_metadata": True,
    }

    t_input = ft.TextField(label="Input (folder or ZIP files)", expand=True, read_only=True)
    t_dest = ft.TextField(label="Destination folder", expand=True, read_only=True)

    async def t_pick_folder(e):
        path = await file_picker.get_directory_path(dialog_title="Select Input Folder")
        if path:
            t_state["inputs"] = [path]
            t_input.value = f"Folder: {path}"
            page.update()

    async def t_pick_zips(e):
        files = await file_picker.pick_files(
            dialog_title="Select ZIP files",
            allow_multiple=True,
            allowed_extensions=["zip"],
        )
        if files:
            paths = [file.path for file in files if file.path]
            if not paths:
                return
            t_state["inputs"] = paths
            t_input.value = (
                f"ZIP: {paths[0]}"
                if len(paths) == 1
                else f"{len(paths)} ZIP files selected"
            )
            page.update()

    async def t_pick_dest(e):
        path = await file_picker.get_directory_path(dialog_title="Select Destination folder")
        if path:
            t_state["destination"] = path
            t_dest.value = path
            page.update()

    def _set_sorting(event):
        t_state["sorting"] = event.control.value

    def _set_filter(event):
        t_state["filter"] = event.control.value

    def _set_conflict(event):
        t_state["conflict"] = event.control.value

    t_sorting = ft.RadioGroup(
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

    t_filter = ft.RadioGroup(
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

    t_conflict = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="keep", label="Keep Both (Add __1)"),
                ft.Radio(value="overwrite", label="Overwrite Existing"),
            ]
        ),
        value="keep",
        on_change=_set_conflict,
    )

    t_preserve = ft.Checkbox(
        label="Preserve JSON metadata sidecars (.json)",
        value=True,
        on_change=lambda e: t_state.update({"preserve_metadata": e.control.value}),
    )

    start_btn = ft.Button(
        content="Start Special Organizer",
        bgcolor=ft.Colors.BLUE_600,
        color=ft.Colors.WHITE,
        height=50,
    )
    cancel_btn = ft.Button(
        content="Cancel",
        bgcolor=ft.Colors.RED_700,
        color=ft.Colors.WHITE,
        visible=False,
    )

    def t_cancel(event) -> None:
        state["cancel_event"].set()
        cancel_btn.disabled = True
        cancel_btn.content = "Cancelling..."
        log_func("Cancellation requested. Finishing the current file...")
        page.update()

    cancel_btn.on_click = t_cancel

    def _finish(result: dict) -> None:
        finish_job(state)
        if result.get("error"):
            log_func(result["error"], is_error=True)
        elif result.get("cancelled"):
            log_func(
                "Organizer cancelled: "
                f"{result.get('processed', 0)} organized, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors."
            )
        else:
            log_func(
                f"✅ Organizer finished in {result.get('time', '?')}: "
                f"{result.get('processed', 0)} organized, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors."
            )
            if result.get("total", 0) == 0:
                if result.get("errors", 0):
                    log_func(
                        "No media was organized; one or more inputs could not "
                        "be read. See the errors above."
                    )
                else:
                    log_func("No matching media or ZIP files were found.")

        if (
            not result.get("error")
            and not result.get("cancelled")
            and result.get("processed", 0) > 0
        ):
            play_sound()

        progress_queue.put((0, 0, ""))
        page.update()

    def t_start(event) -> None:
        if state["is_running"]:
            return
        if not t_state["inputs"] or not t_state["destination"]:
            log_func("Missing inputs or destination for Organizer.", is_error=True)
            return

        # Snapshot inputs so browsing during a run cannot change the job.
        inputs = list(t_state["inputs"])
        destination = t_state["destination"]
        sorting = t_state["sorting"]
        media_filter = t_state["filter"]
        conflict = t_state["conflict"]
        preserve_metadata = t_state["preserve_metadata"]

        log_func("Starting Special Organizer...")
        begin_job(state, cancel_btn)
        page.update()

        def progress_cb(current, total, message):
            progress_queue.put((current, total, message))

        def error_cb(name, message):
            log_func(f"Failed: {name}: {message}", is_error=True)

        def run():
            result: dict = {}
            try:
                result = run_takeout(
                    inputs,
                    destination,
                    sorting,
                    media_filter,
                    conflict,
                    progress_cb,
                    state["cancel_event"],
                    preserve_metadata=preserve_metadata,
                    error_callback=error_cb,
                )
            except Exception as exc:  # noqa: BLE001 - surface to the user
                log_func(f"Error: {exc}", is_error=True)
                result = {"error": str(exc)}
            finally:
                post_ui(lambda: _finish(result))

        page.run_thread(run)

    start_btn.on_click = t_start
    register_job(state, start_btn, cancel_btn)

    return ft.Column(
        [
            card(
                ft.Column(
                    [
                        ft.Text("Special Organizer", weight="bold", size=16),
                        ft.Text(
                            "Google Takeout + personal folder organizer. ZIPs are "
                            "streamed, not extracted to a temporary folder.",
                            size=12,
                            color=ft.Colors.GREY_500,
                        ),
                    ],
                    spacing=4,
                )
            ),
            card(
                ft.Column(
                    [
                        ft.Text("1. Input Source", weight="bold"),
                        t_input,
                        ft.Row(
                            [
                                ft.Button("Browse Folder", on_click=t_pick_folder),
                                ft.Button(
                                    "Browse ZIPs (Multiple)",
                                    on_click=t_pick_zips,
                                    bgcolor=ft.Colors.BLUE_700,
                                    color=ft.Colors.WHITE,
                                ),
                            ],
                            spacing=10,
                        ),
                        ft.Text(
                            "Existing files with the same size and timestamp are "
                            "skipped, so re-running is safe.",
                            size=11,
                            color=ft.Colors.GREY_500,
                        ),
                    ]
                )
            ),
            card(
                ft.Column(
                    [
                        ft.Text("2. Destination Folder", weight="bold"),
                        ft.Row(
                            [
                                t_dest,
                                ft.IconButton(
                                    ft.Icons.FOLDER_OPEN,
                                    tooltip="Open Destination",
                                    on_click=lambda e: open_folder(
                                        t_state["destination"]
                                    ),
                                ),
                            ]
                        ),
                        ft.Button("Browse Destination", on_click=t_pick_dest),
                    ]
                )
            ),
            card(
                ft.Column(
                    [ft.Text("3. Filter", weight="bold"), t_filter]
                )
            ),
            card(
                ft.Column(
                    [ft.Text("4. Conflict Resolution", weight="bold"), t_conflict]
                )
            ),
            card(
                ft.Column(
                    [ft.Text("5. Sorting", weight="bold"), t_sorting]
                )
            ),
            card(
                ft.Column(
                    [
                        ft.Text("6. Metadata", weight="bold"),
                        t_preserve,
                        ft.Text(
                            "Sidecars are copied next to the organized media so "
                            "date/album information is not lost.",
                            size=11,
                            color=ft.Colors.GREY_500,
                        ),
                    ]
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
