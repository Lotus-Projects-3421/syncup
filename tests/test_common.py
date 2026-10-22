"""Lightweight import/API smoke tests for the UI helpers."""

from __future__ import annotations

import threading
import unittest

import flet as ft

from views.common import begin_job, card, error_lines, finish_job, human_size


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


if __name__ == "__main__":
    unittest.main()
