"""Tests for AppImage/desktop-entry argument sanitisation."""

from __future__ import annotations

import unittest

from core.cli import is_placeholder_arg, sanitize_argv


class CliTests(unittest.TestCase):
    def test_placeholder_tokens_are_removed(self):
        self.assertEqual(sanitize_argv(["/path/app", "%U"]), ["/path/app"])
        self.assertEqual(sanitize_argv(["/path/app", "%f"]), ["/path/app"])

    def test_real_paths_are_preserved(self):
        self.assertEqual(
            sanitize_argv(["/path/app", "--foo", "%F", "/real/path"]),
            ["/path/app", "--foo", "/real/path"],
        )

    def test_case_insensitive_and_empty_input(self):
        self.assertTrue(is_placeholder_arg("%u"))
        self.assertTrue(is_placeholder_arg(" %F "))
        self.assertFalse(is_placeholder_arg("/tmp/%U"))
        self.assertEqual(sanitize_argv([]), [])
        self.assertEqual(sanitize_argv(["app"]), ["app"])


if __name__ == "__main__":
    unittest.main()
