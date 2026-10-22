"""Duplicate finder tab with preview, manifest, and undo."""

from __future__ import annotations

import flet as ft

from core.dedup_engine import run_dedup, undo_dedup
from views.common import (
    begin_job,
    card,
    error_lines,
    finish_job,
    human_size,
    open_folder,
    register_job,
)


def build_dedup_view(
    page: ft.Page,
    state: dict,
    log_func,
    file_picker,
    progress_queue,
    play_sound,
    post_ui,
):
    d_state = {"scan_dir": "", "preview": None, "preview_scan_dir": None}

    d_input = ft.TextField(label="Folder to scan for duplicates", expand=True, read_only=True)
    d_preview_text = ft.Text(
        "No preview yet. Run a preview to see what would be moved.",
        size=12,
        color=ft.Colors.GREY_500,
    )

    async def d_pick_folder(e):
        path = await file_picker.get_directory_path(dialog_title="Select Folder to scan")
        if path:
            d_state["scan_dir"] = path
            d_state["preview"] = None
            d_state["preview_scan_dir"] = None
            d_input.value = path
            d_preview_text.value = (
                "No preview yet. Run a preview to see what would be moved."
            )
            page.update()

    preview_btn = ft.Button(
        content="Preview Duplicates",
        bgcolor=ft.Colors.BLUE_700,
        color=ft.Colors.WHITE,
    )
    start_btn = ft.Button(
        content="Move Duplicates",
        bgcolor=ft.Colors.ORANGE_600,
        color=ft.Colors.WHITE,
        height=50,
    )
    undo_btn = ft.Button(
        content="Undo Last Move",
        bgcolor=ft.Colors.GREY_700,
        color=ft.Colors.WHITE,
    )
    cancel_btn = ft.Button(
        content="Cancel",
        bgcolor=ft.Colors.RED_700,
        color=ft.Colors.WHITE,
        visible=False,
    )

    def d_cancel(event) -> None:
        state["cancel_event"].set()
        cancel_btn.disabled = True
        cancel_btn.content = "Cancelling..."
        log_func("Cancellation requested. Finishing the current file...")
        page.update()

    cancel_btn.on_click = d_cancel

    def _log_groups(result: dict) -> None:
        details = result.get("group_details") or []
        for index, group in enumerate(details[:5], start=1):
            log_func(
                f"Group {index}: keep {group['keep']} "
                f"({len(group['duplicates'])} duplicate(s))"
            )
        if len(details) > 5:
            log_func(f"... and {len(details) - 5} more groups.")

    def _finish(result: dict, mode: str, scan_dir: str | None = None) -> None:
        finish_job(state)

        if result.get("error"):
            log_func(result["error"], is_error=True)
            if mode == "preview":
                d_preview_text.value = "Preview failed. See the activity log."
        elif result.get("cancelled"):
            log_func(f"{mode.title()} cancelled.")
        elif mode == "preview":
            if scan_dir is not None and scan_dir != d_state["scan_dir"]:
                d_state["preview"] = None
                d_state["preview_scan_dir"] = None
                d_preview_text.value = (
                    "Folder changed during preview. Run Preview again."
                )
                log_func(
                    "Folder changed while the preview was running; "
                    "the result was discarded.",
                    is_error=True,
                )
            else:
                count = result.get("duplicates", 0)
                groups = result.get("groups", 0)
                size = human_size(result.get("bytes", 0))
                errors = result.get("errors", 0)
                error_suffix = (
                    f" {errors} file(s) could not be read." if errors else ""
                )
                if count:
                    d_state["preview"] = result
                    d_state["preview_scan_dir"] = scan_dir
                    d_preview_text.value = (
                        f"Preview: {count} duplicate file(s) in {groups} group(s), "
                        f"{size} movable. Nothing has been moved yet."
                        f"{error_suffix}"
                    )
                    log_func(
                        f"Preview complete: {count} duplicate(s) in {groups} group(s), "
                        f"{size} movable. Nothing was moved.{error_suffix}"
                    )
                    _log_groups(result)
                else:
                    d_state["preview"] = result
                    d_state["preview_scan_dir"] = scan_dir
                    d_preview_text.value = (
                        f"Preview: no exact duplicates found.{error_suffix}"
                    )
                    log_func(f"Preview complete: no exact duplicates found.{error_suffix}")
        elif mode == "move":
            moved = result.get("moved", 0)
            log_func(
                f"✅ Duplicate move finished in {result.get('time', '?')}: "
                f"{moved} file(s) moved, {result.get('errors', 0)} errors."
            )
            if result.get("manifest"):
                log_func(f"Undo manifest: {result['manifest']}")
            d_state["preview"] = None
            d_state["preview_scan_dir"] = None
            d_preview_text.value = (
                "Move complete. Run a new preview before moving again."
            )
        elif mode == "undo":
            log_func(
                f"↩️ Undo finished in {result.get('time', '?')}: "
                f"{result.get('restored', 0)} restored, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors."
            )
            d_state["preview"] = None
            d_state["preview_scan_dir"] = None

        # Undo has no per-file error callback, so show its details here.
        if mode == "undo":
            for line in error_lines(result):
                log_func(line, is_error=True)

        if not result.get("error") and not result.get("cancelled"):
            if mode == "move" and result.get("moved", 0) > 0:
                play_sound()
            if mode == "undo" and result.get("restored", 0) > 0:
                play_sound()

        progress_queue.put((0, 0, ""))
        page.update()

    def d_preview(event=None) -> None:
        if state["is_running"]:
            return
        if not d_state["scan_dir"]:
            log_func("Please select a folder first.", is_error=True)
            return

        scan_dir = d_state["scan_dir"]
        d_state["preview"] = None
        d_state["preview_scan_dir"] = None
        log_func("Scanning for duplicates (preview only)...")
        begin_job(state, cancel_btn)
        page.update()

        def progress_cb(current, total, message):
            progress_queue.put((current, total, message))

        def error_cb(name, message):
            log_func(f"Failed: {name}: {message}", is_error=True)

        def run():
            result: dict = {}
            try:
                result = run_dedup(
                    scan_dir,
                    progress_cb,
                    state["cancel_event"],
                    dry_run=True,
                    error_callback=error_cb,
                )
            except Exception as exc:  # noqa: BLE001 - surface to the user
                log_func(f"Error: {exc}", is_error=True)
                result = {"error": str(exc)}
            finally:
                post_ui(lambda: _finish(result, "preview", scan_dir))

        page.run_thread(run)

    def d_start(event) -> None:
        if state["is_running"]:
            return
        if not d_state["scan_dir"]:
            log_func("Please select a folder first.", is_error=True)
            return

        if (
            d_state["preview"] is None
            or d_state["preview_scan_dir"] != d_state["scan_dir"]
        ):
            log_func("Run Preview first so you can see what will move.")
            d_preview(event)
            return

        if d_state["preview"].get("duplicates", 0) == 0:
            log_func("Preview found no duplicates. Nothing to move.")
            return

        scan_dir = d_state["scan_dir"]
        log_func(
            f"Moving {d_state['preview'].get('duplicates', 0)} duplicate(s) to a "
            "_Duplicates folder..."
        )
        begin_job(state, cancel_btn)
        page.update()

        def progress_cb(current, total, message):
            progress_queue.put((current, total, message))

        def error_cb(name, message):
            log_func(f"Failed: {name}: {message}", is_error=True)

        def run():
            result: dict = {}
            try:
                result = run_dedup(
                    scan_dir,
                    progress_cb,
                    state["cancel_event"],
                    dry_run=False,
                    error_callback=error_cb,
                )
            except Exception as exc:  # noqa: BLE001 - surface to the user
                log_func(f"Error: {exc}", is_error=True)
                result = {"error": str(exc)}
            finally:
                post_ui(lambda: _finish(result, "move"))

        page.run_thread(run)

    def d_undo(event) -> None:
        if state["is_running"]:
            return
        if not d_state["scan_dir"]:
            log_func("Please select the folder that was scanned.", is_error=True)
            return

        scan_dir = d_state["scan_dir"]
        log_func("Looking for the latest duplicate manifest...")
        begin_job(state, cancel_btn)
        page.update()

        def progress_cb(current, total, message):
            progress_queue.put((current, total, message))

        def run():
            result: dict = {}
            try:
                result = undo_dedup(
                    scan_dir,
                    progress_cb,
                    state["cancel_event"],
                )
            except Exception as exc:  # noqa: BLE001 - surface to the user
                log_func(f"Error: {exc}", is_error=True)
                result = {"error": str(exc)}
            finally:
                post_ui(lambda: _finish(result, "undo"))

        page.run_thread(run)

    preview_btn.on_click = d_preview
    start_btn.on_click = d_start
    undo_btn.on_click = d_undo
    register_job(state, preview_btn, cancel_btn)
    register_job(state, start_btn, cancel_btn)
    register_job(state, undo_btn, cancel_btn)

    return ft.Column(
        [
            card(
                ft.Column(
                    [
                        ft.Text("Duplicate Finder", weight="bold", size=16),
                        ft.Text(
                            "Scans a folder deeply, compares exact contents with "
                            "BLAKE2b hashes, and moves duplicates to a "
                            "_Duplicates_<timestamp> subfolder. Files are never "
                            "deleted, and every move is recorded in a manifest.",
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
                        ft.Text("1. Folder to Scan", weight="bold"),
                        ft.Row(
                            [
                                d_input,
                                ft.IconButton(
                                    ft.Icons.FOLDER_OPEN,
                                    tooltip="Open Folder",
                                    on_click=lambda e: open_folder(
                                        d_state["scan_dir"]
                                    ),
                                ),
                            ]
                        ),
                        ft.Button("Browse Folder", on_click=d_pick_folder),
                    ]
                )
            ),
            card(
                ft.Column(
                    [
                        ft.Text("2. Preview & Move", weight="bold"),
                        d_preview_text,
                        ft.Row(
                            [preview_btn, start_btn],
                            spacing=10,
                        ),
                        ft.Text(
                            "Preview is required before moving. The manifest lets "
                            "you undo the last move.",
                            size=11,
                            color=ft.Colors.GREY_500,
                        ),
                    ]
                )
            ),
            ft.Row(
                [
                    ft.Container(expand=True),
                    undo_btn,
                    cancel_btn,
                    ft.Container(expand=True),
                ]
            ),
        ],
        scroll=ft.ScrollMode.AUTO,
    )
