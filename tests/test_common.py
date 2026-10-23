"""Lightweight import/API smoke tests for the UI helpers."""

from __future__ import annotations

import queue
import threading
import unittest
from unittest import mock

import flet as ft

from views.common import (
    begin_job,
    card,
    error_lines,
    finish_job,
    human_size,
    level_color,
    open_folder,
    progress_callback_for,
)


class DummyButton:
    def __init__(self):
        self.disabled = False
        self.visible = False
        self.content = "Cancel"


class CommonTests(unittest.TestCase):
    def test_imports_do_not_start_the_app(self):
        import views.dedup_view  # noqa: F401
        import views.organizer_view  # noqa: F401
        import views.phone_view  # noqa: F401

    def test_card_builds(self):
        control = card(ft.Text("hello"))
        self.assertIsInstance(control, ft.Container)

    def test_job_button_lifecycle(self):
        start = DummyButton()
        cancel = DummyButton()
        state = {
            "is_running": False,
            "cancel_event": threading.Event(),
            "start_buttons": [start],
            "cancel_buttons": [cancel],
        }

        begin_job(state, cancel)
        self.assertTrue(state["is_running"])
        self.assertTrue(start.disabled)
        self.assertTrue(cancel.visible)

        finish_job(state)
        self.assertFalse(state["is_running"])
        self.assertFalse(start.disabled)
        self.assertFalse(cancel.visible)
        self.assertEqual(cancel.content, "Cancel")

    def test_error_lines_are_capped(self):
        result = {"error_details": [f"file{i}: failed" for i in range(3)]}
        lines = error_lines(result, limit=2)
        self.assertEqual(len(lines), 3)
        self.assertIn("more errors", lines[-1])

    def test_human_size_is_reexported(self):
        self.assertEqual(human_size(1024), "1.0 KB")

    def test_level_colors_switch_with_theme(self):
        self.assertEqual(level_color("success", True), ft.Colors.GREEN_400)
        self.assertEqual(level_color("success", False), ft.Colors.GREEN_700)
        self.assertEqual(level_color("error", True), ft.Colors.RED_400)
        self.assertEqual(level_color("info", True), ft.Colors.ON_SURFACE)

    def test_open_folder_missing_path_calls_error_callback(self):
        messages = []
        result = open_folder("/definitely/not/a/real/folder", on_error=messages.append)
        self.assertFalse(result)
        self.assertEqual(len(messages), 1)
        self.assertIn("does not exist", messages[0])

    def test_open_folder_launches_opener_without_blocking(self):
        with (
            mock.patch("views.common.sys.platform", "linux"),
            mock.patch("views.common.shutil.which", return_value="/usr/bin/xdg-open"),
            mock.patch("views.common.subprocess.Popen") as popen,
        ):
            result = open_folder("/tmp")

        self.assertTrue(result)
        popen.assert_called_once()
        self.assertEqual(popen.call_args.args[0][0], "/usr/bin/xdg-open")

    def test_open_folder_failure_is_reported_not_raised(self):
        messages = []
        with (
            mock.patch("views.common.sys.platform", "linux"),
            mock.patch("views.common.shutil.which", return_value="/usr/bin/xdg-open"),
            mock.patch("views.common.subprocess.Popen", side_effect=OSError("boom")),
        ):
            result = open_folder("/tmp", on_error=messages.append)

        self.assertFalse(result)
        self.assertEqual(len(messages), 1)
        self.assertIn("Could not open", messages[0])

    def test_progress_callback_forwards_and_logs_phase_events(self):
        progress_queue = queue.Queue()
        logs = []
        callback = progress_callback_for(
            progress_queue,
            lambda message, is_error=False, level=None: logs.append(
                (message, is_error, level)
            ),
        )
        event = {
            "phase": "scan",
            "kind": "log",
            "level": "warning",
            "message": "Scanning 2 folders...",
        }

        callback(0, 0, event["message"], event)

        queued = progress_queue.get_nowait()
        self.assertEqual(queued, (0, 0, "Scanning 2 folders...", event))
        self.assertEqual(logs, [("Scanning 2 folders...", False, "warning")])

    def test_progress_callback_does_not_log_every_file(self):
        progress_queue = queue.Queue()
        logs = []
        callback = progress_callback_for(
            progress_queue,
            lambda message, is_error=False, level=None: logs.append(message),
        )
        callback(3, 10, "Copying a.jpg...", {"phase": "transfer", "kind": "progress"})
        self.assertEqual(logs, [])
        self.assertEqual(progress_queue.qsize(), 1)


if __name__ == "__main__":
    unittest.main()
