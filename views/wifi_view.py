"""Wi-Fi Backup tab for the Flet UI.

The tab owns the lifecycle of the local HTTP server, renders a QR code that
points at it, and reacts to server snapshots on the Flet UI thread.  All server
work (uploads and the backup engine) happens on daemon threads; nothing in this
module blocks the UI loop.
"""

from __future__ import annotations

import base64
import os
import queue
import shutil
import socket
import tempfile
import threading
import time
import uuid
from pathlib import Path

import flet as ft
import qrcode
import qrcode.image.svg

from core.fileops import human_size
from core.storage import load_profile, save_profile
from core.wifi_server import (
    DEFAULT_PORT,
    WiFiServer,
    create_app,
    find_available_port,
)
from views.common import (
    begin_job,
    card,
    finish_job,
    level_color,
    open_folder,
    register_job,
    show_scoreboard,
    show_snack,
)

WIFI_DEVICE_ID = "wifi-backup"
SORT_MODES = {"year", "month", "day", "flat"}
FILTER_MODES = {"both", "photos", "videos"}
CONFLICT_MODES = {"keep", "overwrite"}

# 1x1 transparent PNG used until a real QR code is available.  Flet's Image
# control requires a non-empty src even while it is invisible.
BLANK_IMAGE_SRC = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
)


# ---------------------------------------------------------------------------
# QR code / network helpers (module level so they are easy to unit-test)
# ---------------------------------------------------------------------------


def generate_qr_svg(data: str, *, box_size: int = 10, border: int = 2) -> str:
    """Return a standalone SVG document encoding *data*.

    ``qrcode`` returns ``bytes`` when an encoding is passed to
    ``to_string()``, so the result is normalised to ``str`` here.
    """
    qr = qrcode.QRCode(
        version=None,
        error_correction=qrcode.constants.ERROR_CORRECT_M,
        box_size=box_size,
        border=border,
    )
    qr.add_data(data)
    qr.make(fit=True)

    image = qr.make_image(image_factory=qrcode.image.svg.SvgPathImage)
    try:
        svg = image.to_string(encoding="unicode")
    except TypeError:  # pragma: no cover - very old qrcode releases
        svg = image.to_string()

    if isinstance(svg, bytes):
        svg = svg.decode("utf-8")
    svg = str(svg)

    # Flet recognises inline SVG by its xmlns attribute.  qrcode's
    # SvgPathImage already adds it, but keep this guard for other factories.
    if "xmlns" not in svg:
        svg = svg.replace(
            "<svg ",
            '<svg xmlns="http://www.w3.org/2000/svg" ',
            1,
        )
    return svg


def qr_image_source(svg: str) -> str:
    """Return a ``data:`` URI that Flet 1.x renders through ``Image.src``.

    Flet decodes the base64 payload and detects the SVG ``xmlns`` marker, then
    renders it with ``SvgPicture``.  (``Image.src_base64`` no longer exists in
    Flet 1.0.)
    """
    payload = base64.b64encode(svg.encode("utf-8")).decode("ascii")
    return f"data:image/svg+xml;base64,{payload}"


def get_local_ip() -> str:
    """Best-effort primary LAN address (no packets are actually sent)."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.settimeout(0.5)
            probe.connect(("8.8.8.8", 80))
            address = probe.getsockname()[0]
            if address:
                return str(address)
    except OSError:
        pass

    try:
        return socket.gethostbyname(socket.gethostname())
    except OSError:
        return "127.0.0.1"


def is_lan_address(address: str) -> bool:
    """True when *address* is usable from another device on the LAN."""
    return bool(address) and not address.startswith("127.") and address != "0.0.0.0"


def _prune_stale_directories(base: Path, max_age_seconds: float = 24 * 60 * 60) -> None:
    """Remove abandoned staging folders left by an earlier crash."""
    cutoff = time.time() - max_age_seconds
    try:
        entries = list(base.iterdir())
    except OSError:
        return

    for entry in entries:
        if not entry.name.startswith(("wifi-", "syncup-wifi-")):
            continue
        try:
            if entry.is_dir() and entry.stat().st_mtime < cutoff:
                shutil.rmtree(entry, ignore_errors=True)
        except OSError:
            continue


def create_staging_dir() -> Path:
    """Create a fresh session-scoped staging directory.

    ``SYNCUP_WIFI_STAGING_DIR`` can point at a large disk when ``/tmp`` is a
    RAM-backed tmpfs.
    """
    override = os.environ.get("SYNCUP_WIFI_STAGING_DIR", "").strip()
    if override:
        base = Path(override).expanduser()
    else:
        base = Path(tempfile.gettempdir()) / "syncup-wifi"

    base.mkdir(parents=True, exist_ok=True)
    _prune_stale_directories(base)

    session_dir = base / f"wifi-{uuid.uuid4().hex[:8]}"
    session_dir.mkdir(parents=True, exist_ok=False, mode=0o700)
    return session_dir


# ---------------------------------------------------------------------------
# Flet view
# ---------------------------------------------------------------------------


def build_wifi_view(
    page: ft.Page,
    state: dict,
    log_func,
    file_picker,
    progress_queue: queue.Queue,
    play_sound,
    post_ui,
):
    """Build the Wi-Fi Backup tab."""
    profile = load_profile(WIFI_DEVICE_ID) or {}

    destination = str(profile.get("destination") or "")
    sorting = str(profile.get("sorting") or "year")
    if sorting not in SORT_MODES:
        sorting = "year"
    media_filter = str(profile.get("filter") or "both")
    if media_filter not in FILTER_MODES:
        media_filter = "both"
    conflict = str(profile.get("conflict") or "keep")
    if conflict not in CONFLICT_MODES:
        conflict = "keep"

    cancel_event = state.setdefault("cancel_event", threading.Event())

    w_state: dict = {
        "server": None,
        "app": None,
        "staging_root": None,
        "destination": destination,
        "sorting": sorting,
        "filter": media_filter,
        "conflict": conflict,
        "device_id": WIFI_DEVICE_ID,
        "cancel_event": cancel_event,
        "job_active": False,
        "starting": False,
        "display_url": "",
        "port": DEFAULT_PORT,
    }

    # -- controls ----------------------------------------------------------

    w_dest = ft.TextField(
        label="Destination folder",
        expand=True,
        read_only=True,
        value=destination,
    )

    w_qr_hint = ft.Text(
        "Server stopped",
        size=12,
        color=ft.Colors.ON_SURFACE,
        opacity=0.7,
        text_align=ft.TextAlign.CENTER,
    )
    w_qr_placeholder = ft.Container(
        width=300,
        height=300,
        bgcolor=ft.Colors.SURFACE_CONTAINER_HIGHEST,
        border_radius=12,
        content=ft.Column(
            [
                ft.Icon(
                    ft.Icons.WIFI_OFF,
                    size=48,
                    color=ft.Colors.ON_SURFACE,
                    opacity=0.45,
                ),
                w_qr_hint,
            ],
            spacing=8,
            alignment=ft.MainAxisAlignment.CENTER,
            horizontal_alignment=ft.CrossAxisAlignment.CENTER,
        ),
    )
    w_qr_image = ft.Image(
        src=BLANK_IMAGE_SRC,
        width=300,
        height=300,
        fit=ft.BoxFit.CONTAIN,
        visible=False,
    )

    w_status_dot = ft.Icon(
        ft.Icons.CIRCLE,
        size=10,
        color=ft.Colors.ON_SURFACE,
        opacity=0.4,
    )
    w_status = ft.Text(
        "Server stopped",
        size=13,
        weight=ft.FontWeight.BOLD,
        color=ft.Colors.ON_SURFACE,
        expand=True,
    )
    w_device_count = ft.Text(
        "0 phones connected",
        size=13,
        color=ft.Colors.ON_SURFACE,
        opacity=0.85,
    )
    w_upload_stats = ft.Text(
        "0 files uploaded",
        size=13,
        color=ft.Colors.ON_SURFACE,
        opacity=0.85,
    )
    w_staging_info = ft.Text(
        "",
        size=11,
        color=ft.Colors.ON_SURFACE,
        opacity=0.65,
        selectable=True,
    )

    def _set_status(text: str, level: str = "info") -> None:
        is_dark = page.theme_mode != ft.ThemeMode.LIGHT
        colour = level_color(level, is_dark)
        w_status.value = text
        w_status.color = colour
        w_status_dot.color = colour
        w_status_dot.opacity = 0.45 if level == "info" else 0.95

    def _render_stats(snapshot: dict | None = None) -> None:
        if snapshot is None:
            if w_state["server"] is not None and w_state["server"].is_running:
                w_device_count.value = "Waiting for phone..."
            else:
                w_device_count.value = "0 phones connected"
            w_upload_stats.value = "0 files uploaded"
            return

        stage = snapshot.get("stage")
        client_ip = snapshot.get("client_ip")
        if client_ip:
            if stage in ("uploading", "processing"):
                w_device_count.value = f"Phone connected: {client_ip}"
            else:
                w_device_count.value = f"Last phone: {client_ip}"
        else:
            w_device_count.value = "Waiting for phone..."

        received = int(snapshot.get("files_received") or 0)
        expected = int(snapshot.get("expected_files") or 0)
        received_bytes = int(snapshot.get("bytes_received") or 0)
        expected_bytes = int(snapshot.get("expected_bytes") or 0)
        if received or expected:
            w_upload_stats.value = (
                f"{received}/{expected} file(s) uploaded "
                f"({human_size(received_bytes)} of {human_size(expected_bytes)})"
            )
        else:
            w_upload_stats.value = "0 files uploaded"

    # -- destination / settings -------------------------------------------

    def w_open_destination(event=None) -> None:
        def _on_error(message: str) -> None:
            log_func(message, is_error=True, level="error")
            show_snack(page, message, is_error=True)

        if open_folder(w_state["destination"], on_error=_on_error):
            show_snack(page, "Opened in file manager")

    async def w_pick_dest(event=None) -> None:
        if w_state["starting"] or (w_state["server"] is not None and w_state["server"].is_running):
            show_snack(
                page,
                "Stop the Wi-Fi server before changing the destination.",
                is_error=True,
            )
            return
        try:
            path = await file_picker.get_directory_path(dialog_title="Select Destination Folder")
        except Exception as exc:  # noqa: BLE001 - picker failures are non-fatal
            log_func(f"Could not open the folder picker: {exc}", is_error=True)
            return
        if path:
            w_state["destination"] = path
            w_dest.value = path
            page.update()

    def _set_sorting(event) -> None:
        w_state["sorting"] = event.control.value

    def _set_filter(event) -> None:
        w_state["filter"] = event.control.value

    def _set_conflict(event) -> None:
        w_state["conflict"] = event.control.value

    w_sorting = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="day", label="Day"),
                ft.Radio(value="month", label="Month"),
                ft.Radio(value="year", label="Year"),
                ft.Radio(value="flat", label="Flat"),
            ]
        ),
        value=sorting,
        on_change=_set_sorting,
    )
    w_filter = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="both", label="Photos & Videos"),
                ft.Radio(value="photos", label="Photos Only"),
                ft.Radio(value="videos", label="Videos Only"),
            ]
        ),
        value=media_filter,
        on_change=_set_filter,
    )
    w_conflict = ft.RadioGroup(
        content=ft.Row(
            [
                ft.Radio(value="keep", label="Keep Both (Add __1)"),
                ft.Radio(value="overwrite", label="Overwrite Existing"),
            ]
        ),
        value=conflict,
        on_change=_set_conflict,
    )

    def _persist_profile() -> None:
        try:
            save_profile(
                WIFI_DEVICE_ID,
                {
                    "destination": w_state["destination"],
                    "sorting": w_state["sorting"],
                    "filter": w_state["filter"],
                    "conflict": w_state["conflict"],
                },
            )
        except Exception as exc:  # noqa: BLE001 - settings are a convenience
            log_func(f"Could not save Wi-Fi settings: {exc}", is_error=True)

    # -- buttons -----------------------------------------------------------

    start_btn = ft.Button(
        content="Start Wi-Fi Server",
        icon=ft.Icons.PLAY_ARROW,
        bgcolor=ft.Colors.BLUE_600,
        color=ft.Colors.WHITE,
        height=50,
    )
    stop_btn = ft.Button(
        content="Stop Server",
        icon=ft.Icons.STOP,
        bgcolor=ft.Colors.RED_700,
        color=ft.Colors.WHITE,
        height=50,
        visible=False,
    )
    cancel_btn = ft.Button(
        content="Cancel Processing",
        icon=ft.Icons.CANCEL,
        bgcolor=ft.Colors.ORANGE_700,
        color=ft.Colors.WHITE,
        height=50,
        visible=False,
    )
    w_browse_btn = ft.Button(
        "Browse Destination",
        icon=ft.Icons.FOLDER,
        on_click=w_pick_dest,
    )

    # -- QR / status -------------------------------------------------------

    def update_qr_code() -> None:
        """Render the QR code for the running server (or the idle state)."""
        server = w_state["server"]
        running = bool(server is not None and server.is_running)

        if not running:
            w_state["display_url"] = ""
            w_qr_image.visible = False
            w_qr_placeholder.visible = True
            w_qr_hint.value = "Server stopped"
            _set_status("Server stopped", "info")
            return

        address = get_local_ip()
        url = f"http://{address}:{server.port}"
        w_state["display_url"] = url

        try:
            w_qr_image.src = qr_image_source(generate_qr_svg(url))
        except Exception as exc:  # noqa: BLE001 - a QR failure is not fatal
            log_func(f"Could not generate the QR code: {exc}", is_error=True)
            _set_status("Server running, but the QR code could not be drawn.", "error")
            return

        w_qr_image.visible = True
        w_qr_placeholder.visible = False
        if is_lan_address(address):
            _set_status(f"Running on {url}", "success")
        else:
            _set_status(
                "Server running, but no LAN address was found. Check the Wi-Fi connection.",
                "warning",
            )

    # -- processing finish -------------------------------------------------

    def _finish_processing(result: dict) -> None:
        """Restore UI state and show the summary.  UI thread only."""
        w_state["job_active"] = False
        finish_job(state)

        cancel_btn.visible = False
        cancel_btn.disabled = False
        cancel_btn.content = "Cancel Processing"

        server_running = bool(w_state["server"] is not None and w_state["server"].is_running)
        stop_btn.visible = server_running
        stop_btn.disabled = False
        start_btn.visible = not server_running
        start_btn.disabled = False

        progress_queue.put((0, 0, ""))

        if result.get("error"):
            log_func(f"Wi-Fi backup failed: {result['error']}", is_error=True)
            _set_status(f"Backup failed: {result['error']}", "error")
        elif result.get("cancelled"):
            log_func(
                "Wi-Fi backup cancelled: "
                f"{result.get('processed', 0)} copied, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors.",
                level="warning",
            )
            _set_status("Backup cancelled.", "warning")
        else:
            log_func(
                f"✅ Wi-Fi backup finished in {result.get('time', '?')}: "
                f"{result.get('processed', 0)} copied, "
                f"{result.get('skipped', 0)} skipped, "
                f"{result.get('errors', 0)} errors.",
                level="success",
            )
            _set_status("Backup complete.", "success")
            if result.get("total", 0) == 0 and not result.get("processed", 0):
                if result.get("errors", 0):
                    log_func(
                        "No files were copied because some uploads could not "
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

        w_staging_info.value = (
            f"Staging: {w_state['staging_root']} (cleaned after backup)"
            if w_state["staging_root"]
            else ""
        )

        show_scoreboard(
            page,
            result,
            title="Wi-Fi Backup Summary",
            destination=w_state["destination"],
            operation="backup",
            log_func=log_func,
        )
        page.update()

    # -- server snapshot callback (UI thread) ------------------------------

    def _on_server_state(snapshot: dict | None) -> None:
        if not snapshot:
            return

        stage = snapshot.get("stage")
        _render_stats(snapshot)

        if stage == "processing":
            if not w_state["job_active"]:
                w_state["job_active"] = True
                begin_job(state, cancel_btn)
                cancel_btn.content = "Cancel Processing"
                cancel_btn.visible = True
                cancel_btn.disabled = False
                start_btn.visible = False
                stop_btn.visible = True
                _set_status("Copying the uploaded files on this PC...", "organize")
                log_func(
                    "All uploads received; starting the Wi-Fi backup.",
                    level="organize",
                )
        elif stage in ("done", "error"):
            if w_state["job_active"]:
                _finish_processing(snapshot.get("result") or {})
            elif stage == "error":
                _set_status(
                    snapshot.get("error") or "The Wi-Fi backup failed.",
                    "error",
                )
            else:
                _set_status("No files needed backing up.", "info")
        elif stage == "uploading":
            received = int(snapshot.get("files_received") or 0)
            expected = int(snapshot.get("expected_files") or 0)
            if expected == 0:
                _set_status("The phone is already up to date.", "success")
            else:
                _set_status(
                    f"Receiving files from phone ({received}/{expected})...",
                    "organize",
                )
        elif stage == "waiting" and not w_state["job_active"]:
            _set_status("Waiting for a phone to connect...", "info")

        page.update()

    # -- server lifecycle --------------------------------------------------

    def _server_started(
        server: WiFiServer,
        staging: Path,
        app,
        ready: bool,
    ) -> None:
        if not ready and server.last_error:
            try:
                server.stop(timeout=1.0)
            except Exception:  # noqa: BLE001 - already failing
                pass
            shutil.rmtree(staging, ignore_errors=True)
            _server_failed(server.last_error)
            return

        w_state["starting"] = False
        w_state["server"] = server
        w_state["app"] = app
        w_state["staging_root"] = staging
        w_state["port"] = server.port
        w_state["display_url"] = ""

        update_qr_code()

        w_staging_info.value = f"Staging: {staging}"
        start_btn.visible = False
        start_btn.disabled = False
        stop_btn.visible = True
        stop_btn.disabled = False
        w_browse_btn.disabled = True
        _render_stats(None)

        if w_state["display_url"]:
            log_func(
                f"Wi-Fi server listening on {w_state['display_url']}.",
                level="organize",
            )
        if not ready:
            detail = server.last_error or "no ready signal within 5 seconds"
            _set_status("Server started, but it is not ready yet.", "warning")
            log_func(f"Wi-Fi server warning: {detail}", is_error=True)

        page.update()

    def _server_failed(message: str) -> None:
        w_state["starting"] = False
        w_state["server"] = None
        w_state["app"] = None
        w_state["staging_root"] = None
        w_state["display_url"] = ""

        start_btn.visible = True
        start_btn.disabled = False
        stop_btn.visible = False
        stop_btn.disabled = False
        w_browse_btn.disabled = False
        w_qr_image.visible = False
        w_qr_placeholder.visible = True
        w_qr_hint.value = "Server stopped"
        w_staging_info.value = ""

        _render_stats(None)
        _set_status("Server failed to start.", "error")
        log_func(f"Could not start the Wi-Fi server: {message}", is_error=True)
        show_snack(
            page,
            f"Could not start the Wi-Fi server: {message}",
            is_error=True,
        )
        page.update()

    def _server_stopped(staging: Path | None) -> None:
        w_state["server"] = None
        w_state["app"] = None
        w_state["staging_root"] = None
        w_state["display_url"] = ""
        stop_btn.visible = False
        stop_btn.disabled = False
        w_browse_btn.disabled = False

        # Refresh the QR/stats first; the final status line below wins.
        update_qr_code()
        _render_stats(None)

        if not w_state["job_active"]:
            if staging is not None:
                shutil.rmtree(staging, ignore_errors=True)
            w_staging_info.value = ""
            start_btn.visible = True
            start_btn.disabled = False
            _set_status("Server stopped.", "info")
        else:
            # The engine owns cleanup from here; it stops at the next file.
            w_staging_info.value = f"Finishing backup from: {staging}" if staging else ""
            _set_status(
                "Server stopped; finishing the current backup...",
                "warning",
            )

        log_func("Wi-Fi server stopped.", level="warning")
        page.update()

    def start_server() -> None:
        if w_state["starting"]:
            return
        if w_state["server"] is not None and w_state["server"].is_running:
            return
        if state.get("is_running"):
            show_snack(
                page,
                "Wait for the current backup to finish first.",
                is_error=True,
            )
            return

        destination = str(w_state["destination"] or "").strip()
        if not destination:
            log_func(
                "Choose a destination folder before starting the Wi-Fi server.",
                is_error=True,
            )
            show_snack(page, "Choose a destination folder first.", is_error=True)
            return

        destination_path = Path(destination).expanduser()
        try:
            destination_path.mkdir(parents=True, exist_ok=True)
        except OSError as exc:
            log_func(
                f"Cannot use {destination_path} as the destination: {exc}",
                is_error=True,
            )
            show_snack(
                page,
                f"Destination is not usable: {exc}",
                is_error=True,
            )
            return
        if not destination_path.is_dir():
            message = f"Destination is not a folder: {destination_path}"
            log_func(message, is_error=True)
            show_snack(page, message, is_error=True)
            return

        job_settings = {
            "sorting": w_state["sorting"],
            "filter": w_state["filter"],
            "conflict": w_state["conflict"],
        }
        _persist_profile()

        w_state["starting"] = True
        start_btn.disabled = True
        w_browse_btn.disabled = True
        _set_status("Starting the Wi-Fi server...", "organize")
        page.update()
        log_func("Starting the Wi-Fi server...", level="organize")

        def run() -> None:
            staging: Path | None = None
            server: WiFiServer | None = None
            try:
                staging = create_staging_dir()
                port = find_available_port(DEFAULT_PORT)
                app = create_app(
                    device_id=w_state["device_id"],
                    staging_root=staging,
                    dest_root=destination_path,
                    sort_mode=job_settings["sorting"],
                    media_filter=job_settings["filter"],
                    conflict_mode=job_settings["conflict"],
                    progress_queue=progress_queue,
                    cancel_event=cancel_event,
                    post_ui=post_ui,
                    on_state_changed=_on_server_state,
                    is_busy=lambda: bool(state.get("is_running")),
                )
                server = WiFiServer(port=port)
                server.start(app)
                ready = server.wait_until_ready(5.0)
                post_ui(lambda: _server_started(server, staging, app, ready))
            except Exception as exc:  # noqa: BLE001 - surface startup failures
                if server is not None:
                    try:
                        server.stop(timeout=2.0)
                    except Exception:  # noqa: BLE001
                        pass
                if staging is not None:
                    shutil.rmtree(staging, ignore_errors=True)
                message = str(exc) or exc.__class__.__name__
                post_ui(lambda: _server_failed(message))

        page.run_thread(run)

    def stop_server() -> None:
        server = w_state["server"]
        if server is None:
            _server_stopped(None)
            return

        _set_status("Stopping the Wi-Fi server...", "warning")
        stop_btn.disabled = True
        # Also aborts a running engine at its next file boundary.
        cancel_event.set()
        log_func("Stopping the Wi-Fi server...", level="warning")
        page.update()

        staging = w_state["staging_root"]

        def run() -> None:
            try:
                server.stop()
            finally:
                post_ui(lambda: _server_stopped(staging))

        page.run_thread(run)

    def cancel_processing() -> None:
        if not w_state["job_active"]:
            return
        cancel_event.set()
        cancel_btn.disabled = True
        cancel_btn.content = "Cancelling..."
        log_func(
            "Cancellation requested. Finishing the current file...",
            level="warning",
        )
        page.update()

    start_btn.on_click = lambda event: start_server()
    stop_btn.on_click = lambda event: stop_server()
    cancel_btn.on_click = lambda event: cancel_processing()

    register_job(state, start_btn, cancel_btn)

    # -- shutdown watchdog -------------------------------------------------

    def _shutdown_watch() -> None:
        shutdown_event = state.get("shutdown_event")
        if not isinstance(shutdown_event, threading.Event):
            return
        shutdown_event.wait()
        server = w_state.get("server")
        if server is not None:
            try:
                server.stop(timeout=2.0)
            except Exception:  # noqa: BLE001 - the process is exiting
                pass
        staging = w_state.get("staging_root")
        if staging:
            shutil.rmtree(staging, ignore_errors=True)

    if isinstance(state.get("shutdown_event"), threading.Event):
        threading.Thread(
            target=_shutdown_watch,
            name="syncup-wifi-shutdown",
            daemon=True,
        ).start()

    # -- layout ------------------------------------------------------------

    qr_column = ft.Column(
        [
            w_qr_placeholder,
            w_qr_image,
            ft.Text(
                "Scan this with your phone camera",
                size=11,
                color=ft.Colors.ON_SURFACE,
                opacity=0.6,
                text_align=ft.TextAlign.CENTER,
            ),
        ],
        spacing=8,
        horizontal_alignment=ft.CrossAxisAlignment.CENTER,
        width=320,
    )

    info_column = ft.Column(
        [
            ft.Row(
                [w_status_dot, w_status],
                spacing=8,
                vertical_alignment=ft.CrossAxisAlignment.CENTER,
            ),
            w_device_count,
            w_upload_stats,
            w_staging_info,
            ft.Text(
                "Phone and PC must be on the same Wi-Fi network. "
                "If prompted, allow the server port through your firewall.",
                size=11,
                color=ft.Colors.ON_SURFACE,
                opacity=0.65,
            ),
        ],
        spacing=10,
        expand=True,
    )

    server_card = card(
        ft.Column(
            [
                ft.Row(
                    [
                        ft.Text(
                            "Wi-Fi Backup",
                            weight=ft.FontWeight.BOLD,
                            size=16,
                            color=ft.Colors.ON_SURFACE,
                        ),
                        ft.Container(expand=True),
                        ft.Text(
                            "No cable needed",
                            size=12,
                            color=ft.Colors.ON_SURFACE,
                            opacity=0.6,
                        ),
                    ]
                ),
                ft.Row(
                    [qr_column, info_column],
                    spacing=20,
                    vertical_alignment=ft.CrossAxisAlignment.START,
                ),
                ft.Row(
                    [start_btn, stop_btn],
                    spacing=10,
                ),
            ],
            spacing=14,
        )
    )

    settings_card = card(
        ft.Column(
            [
                ft.Text(
                    "Backup settings",
                    weight=ft.FontWeight.BOLD,
                    size=16,
                    color=ft.Colors.ON_SURFACE,
                ),
                ft.Text(
                    "Destination",
                    weight=ft.FontWeight.BOLD,
                    size=13,
                    color=ft.Colors.ON_SURFACE,
                ),
                ft.Row(
                    [
                        w_dest,
                        ft.IconButton(
                            ft.Icons.FOLDER_OPEN,
                            tooltip="Open destination",
                            on_click=w_open_destination,
                        ),
                    ]
                ),
                w_browse_btn,
                ft.Text(
                    "Uploads are staged in a temporary folder and copied into "
                    "the destination with verified, atomic writes.",
                    size=11,
                    color=ft.Colors.ON_SURFACE,
                    opacity=0.65,
                ),
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                ft.Text(
                    "Sorting",
                    weight=ft.FontWeight.BOLD,
                    size=13,
                    color=ft.Colors.ON_SURFACE,
                ),
                w_sorting,
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                ft.Text(
                    "Filter",
                    weight=ft.FontWeight.BOLD,
                    size=13,
                    color=ft.Colors.ON_SURFACE,
                ),
                w_filter,
                ft.Divider(color=ft.Colors.OUTLINE_VARIANT),
                ft.Text(
                    "Conflict resolution",
                    weight=ft.FontWeight.BOLD,
                    size=13,
                    color=ft.Colors.ON_SURFACE,
                ),
                w_conflict,
            ],
            spacing=10,
        )
    )

    return ft.Column(
        [
            server_card,
            settings_card,
            ft.Row(
                [
                    ft.Container(expand=True),
                    cancel_btn,
                    ft.Container(expand=True),
                ]
            ),
        ],
        scroll=ft.ScrollMode.AUTO,
        spacing=10,
    )
