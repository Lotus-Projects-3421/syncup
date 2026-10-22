"""Tests for settings, profiles, and SQLite backup state."""

from __future__ import annotations

import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from core.storage import (
    StateStore,
    get_app_data_dir,
    get_state_file,
    load_global_settings,
    load_profile,
    load_state,
    save_global_settings,
    save_profile,
    save_state,
)


class StorageTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self._env = mock.patch.dict(
            os.environ, {"XDG_CONFIG_HOME": str(self.root / "config")}
        )
        self._env.start()
        self.addCleanup(self._env.stop)

    def test_settings_defaults_and_merge(self):
        self.assertEqual(load_global_settings()["theme"], "dark")
        save_global_settings({"theme": "light", "window_width": 1000})
        loaded = load_global_settings()
        self.assertEqual(loaded["theme"], "light")
        self.assertEqual(loaded["window_width"], 1000)
        self.assertEqual(loaded["window_height"], 950)

    def test_profile_round_trip(self):
        save_profile("mtp://device/", {"destination": "/tmp/photos", "sorting": "day"})
        self.assertEqual(
            load_profile("mtp://device/"),
            {"destination": "/tmp/photos", "sorting": "day"},
        )

    def test_state_round_trip(self):
        with StateStore("device-1") as store:
            store.mark("DCIM/a.jpg", 10, 123456789)
            store.mark("DCIM/b.jpg", 20, 123456790, commit=True)
        loaded = load_state("device-1")
        self.assertEqual(loaded["DCIM/a.jpg"]["size"], 10)
        self.assertEqual(loaded["DCIM/b.jpg"]["mtime"], 123456790)

    def test_state_is_persistent_across_instances(self):
        with StateStore("device-1") as store:
            store.mark("a.jpg", 1, 2, commit=True)
        with StateStore("device-1") as store:
            self.assertIn("a.jpg", store.load())

    def test_legacy_json_state_is_migrated_without_deleting_it(self):
        legacy = get_state_file("device-legacy")
        legacy.parent.mkdir(parents=True, exist_ok=True)
        legacy.write_text(json.dumps(["DCIM/old.jpg"]), encoding="utf-8")

        loaded = load_state("device-legacy")

        self.assertIn("DCIM/old.jpg", loaded)
        self.assertIsNone(loaded["DCIM/old.jpg"]["size"])
        self.assertTrue(legacy.exists())

    def test_save_state_accepts_a_set(self):
        save_state("device-set", {"a.jpg", "b.jpg"})
        loaded = load_state("device-set")
        self.assertEqual(set(loaded), {"a.jpg", "b.jpg"})

    def test_locked_database_is_not_treated_as_corrupt(self):
        with (
            mock.patch(
                "core.storage.sqlite3.connect",
                side_effect=sqlite3.OperationalError("database is locked"),
            ),
            mock.patch("core.storage.os.replace") as replace,
        ):
            with self.assertRaises(sqlite3.OperationalError):
                StateStore("locked-device")
        replace.assert_not_called()

    def test_app_data_dir_is_created(self):
        self.assertTrue(get_app_data_dir().is_dir())


if __name__ == "__main__":
    unittest.main()
