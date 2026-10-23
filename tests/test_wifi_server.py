"""Tests for the Wi-Fi backup server helpers and engine integration."""

from __future__ import annotations

import os
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from core.engine import run_wifi_backup
from core.wifi_models import ManifestEntry
from core.wifi_server import compute_missing_files, safe_relative_path


class SafeRelativePathTests(unittest.TestCase):
    def test_accepts_normal_relative_path(self):
        result = safe_relative_path("DCIM/Camera/photo.jpg")
        self.assertIsNotNone(result)
        self.assertEqual(result.as_posix(), "DCIM/Camera/photo.jpg")

    def test_rejects_traversal(self):
        self.assertIsNone(safe_relative_path("../secret.txt"))
        self.assertIsNone(safe_relative_path("a/../b.jpg"))
        self.assertIsNone(safe_relative_path("a/.."))

    def test_rejects_absolute_and_drive(self):
        self.assertIsNone(safe_relative_path("/etc/passwd"))
        self.assertIsNone(safe_relative_path("C:\\foo.jpg"))
        self.assertIsNone(safe_relative_path("~/x.jpg"))

    def test_rejects_hidden_and_partial(self):
        self.assertIsNone(safe_relative_path(".hidden/x.jpg"))
        self.assertIsNone(safe_relative_path("DCIM/.thumb.jpg"))
        self.assertIsNone(safe_relative_path("DCIM/.x.part-1"))


class ComputeMissingFilesTests(unittest.TestCase):
    def _entry(self, rel, size, mtime):
        return ManifestEntry(rel_path=rel, size=size, mtime_ns=mtime)

    def test_diffs_manifest_against_state(self):
        manifest = [
            self._entry("a.jpg", 10, 100),
            self._entry("b.jpg", 20, 200),
            self._entry("c.jpg", 30, 300),
        ]
        state = {
            "a.jpg": {"size": 10, "mtime": 100},   # already backed up
            "b.jpg": {"size": 19, "mtime": 200},   # modified (size differs)
            "legacy.jpg": {"size": None, "mtime": None},  # trusted, not in manifest
        }
        missing, total_bytes, ignored = compute_missing_files(manifest, state, "both")

        rels = [m.rel_path for m in missing]
        self.assertIn("b.jpg", rels)
        self.assertIn("c.jpg", rels)
        self.assertNotIn("a.jpg", rels)
        # b.jpg (20) + c.jpg (30)
        self.assertEqual(total_bytes, 50)
        self.assertEqual(ignored, 0)

        statuses = {m.rel_path: m.status for m in missing}
        self.assertEqual(statuses["b.jpg"], "modified")
        self.assertEqual(statuses["c.jpg"], "new")

    def test_legacy_size_none_row_is_trusted(self):
        manifest = [self._entry("legacy.jpg", 5, 50)]
        state = {"legacy.jpg": {"size": None, "mtime": None}}
        missing, total_bytes, ignored = compute_missing_files(manifest, state, "both")
        self.assertEqual(missing, [])
        self.assertEqual(total_bytes, 0)

    def test_mtime_tolerance(self):
        manifest = [self._entry("a.jpg", 10, 100)]
        # within 2s tolerance => not modified
        state = {"a.jpg": {"size": 10, "mtime": 100 + 1_000_000_000}}
        missing, _, _ = compute_missing_files(manifest, state, "both")
        self.assertEqual(missing, [])

    def test_media_filter_and_unsafe_ignored(self):
        manifest = [
            self._entry("photo.jpg", 10, 100),
            self._entry("clip.mp4", 20, 200),
            self._entry("../escape.jpg", 5, 50),
        ]
        missing, total_bytes, ignored = compute_missing_files(manifest, {}, "photos")
        rels = [m.rel_path for m in missing]
        self.assertEqual(rels, ["photo.jpg"])
        self.assertEqual(total_bytes, 10)
        # clip.mp4 (filter) + ../escape.jpg (unsafe) => 2 ignored
        self.assertEqual(ignored, 2)

    def test_duplicates_ignored(self):
        manifest = [
            self._entry("a.jpg", 10, 100),
            self._entry("a.jpg", 10, 100),
        ]
        missing, total_bytes, ignored = compute_missing_files(manifest, {}, "both")
        self.assertEqual(len(missing), 1)
        self.assertEqual(ignored, 1)


class RunWiFiBackupTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self._env = mock.patch.dict(
            os.environ, {"XDG_CONFIG_HOME": str(self.root / "config")}
        )
        self._env.start()
        self.addCleanup(self._env.stop)

        self.staging = self.root / "staging"
        self.staging.mkdir()
        self.dest = self.root / "dest"

    def _write(self, relative: str, data: bytes) -> Path:
        path = self.staging / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def test_moves_file_from_staging_to_dest(self):
        self._write("DCIM/Camera/photo.jpg", b"hello")
        result = run_wifi_backup(
            "wifi-device",
            str(self.staging),
            ["DCIM"],
            str(self.dest),
            "year",
            "both",
            "keep",
            lambda *args, **kwargs: None,
            threading.Event(),
        )
        self.assertFalse(result.get("error"), result.get("error"))
        self.assertEqual(result["processed"], 1)
        self.assertEqual(result["errors"], 0)
        # file moved/copied into dest
        copied = list(self.dest.rglob("*.jpg"))
        self.assertEqual(len(copied), 1)
        self.assertEqual(copied[0].read_bytes(), b"hello")
        # staging not deleted by engine
        self.assertTrue((self.staging / "DCIM" / "Camera" / "photo.jpg").exists())

    def test_second_run_skips(self):
        self._write("DCIM/Camera/photo.jpg", b"hello")
        run_wifi_backup(
            "wifi-device",
            str(self.staging),
            ["DCIM"],
            str(self.dest),
            "year",
            "both",
            "keep",
            lambda *args, **kwargs: None,
            threading.Event(),
        )
        result = run_wifi_backup(
            "wifi-device",
            str(self.staging),
            ["DCIM"],
            str(self.dest),
            "year",
            "both",
            "keep",
            lambda *args, **kwargs: None,
            threading.Event(),
        )
        self.assertEqual(result["processed"], 0)
        self.assertEqual(result["skipped"], 1)

    def test_flat_staging_fallback(self):
        self._write("photo.jpg", b"flat")
        result = run_wifi_backup(
            "wifi-device2",
            str(self.staging),
            [],
            str(self.dest),
            "year",
            "both",
            "keep",
            lambda *args, **kwargs: None,
            threading.Event(),
        )
        self.assertEqual(result["processed"], 1)
        self.assertTrue(list(self.dest.rglob("*.jpg")))

    def test_missing_staging_returns_error(self):
        result = run_wifi_backup(
            "wifi-device3",
            str(self.root / "nope"),
            [],
            str(self.dest),
            "year",
            "both",
            "keep",
            lambda *args, **kwargs: None,
            threading.Event(),
        )
        self.assertTrue(result.get("error"))
        self.assertEqual(result["processed"], 0)


if __name__ == "__main__":
    unittest.main()
