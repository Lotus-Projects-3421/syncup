"""Tests for the phone backup engine."""

from __future__ import annotations

import os
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

from core.engine import run_backup
from core.storage import get_state_file


class EngineTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self._env = mock.patch.dict(
            os.environ, {"XDG_CONFIG_HOME": str(self.root / "config")}
        )
        self._env.start()
        self.addCleanup(self._env.stop)

        self.phone = self.root / "phone"
        self.phone_root = self.phone / "Internal shared storage"
        self.phone_root.mkdir(parents=True)
        self.dest = self.root / "dest"

    def _write(self, relative: str, data: bytes, mtime: float | None = None) -> Path:
        path = self.phone_root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        if mtime is not None:
            os.utime(path, (mtime, mtime))
        return path

    def _run(
        self,
        *,
        folders=("DCIM",),
        sort_mode="year",
        media_filter="both",
        conflict="keep",
        cancel_event=None,
        device="device-1",
        destination: Path | None = None,
    ):
        return run_backup(
            device,
            str(self.phone),
            list(folders),
            str(destination or self.dest),
            sort_mode,
            media_filter,
            conflict,
            lambda *args: None,
            cancel_event or threading.Event(),
        )

    def _photo_path(self) -> Path:
        return self.dest / "2024" / "2024-01-01_12-00-00_IMG_20240101_120000.jpg"

    def test_basic_backup(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        result = self._run()
        self.assertEqual(result["processed"], 1)
        self.assertEqual(result["errors"], 0)
        self.assertEqual(self._photo_path().read_bytes(), b"photo1")
        self.assertEqual(list(self.root.rglob("*.part-*")), [])

    def test_second_run_skips_already_backed_up_file(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        self._run()
        result = self._run()
        self.assertEqual(result["processed"], 0)
        self.assertEqual(result["skipped"], 1)
        self.assertEqual(result["total"], 0)

    def test_keep_both_when_source_changes(self):
        source = self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        self._run()

        source.write_bytes(b"changed")
        later = time.time() + 100
        os.utime(source, (later, later))

        result = self._run(conflict="keep")
        self.assertEqual(result["processed"], 1)
        self.assertEqual(self._photo_path().read_bytes(), b"photo1")
        alternate = self._photo_path().with_name(
            self._photo_path().stem + "__1.jpg"
        )
        self.assertEqual(alternate.read_bytes(), b"changed")

    def test_overwrite_replaces_only_after_verified_copy(self):
        source = self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        self._run()

        source.write_bytes(b"newer")
        later = time.time() + 200
        os.utime(source, (later, later))

        result = self._run(conflict="overwrite")
        self.assertEqual(result["processed"], 1)
        self.assertEqual(self._photo_path().read_bytes(), b"newer")
        self.assertEqual(list(self.dest.rglob("*.part-*")), [])

    def test_overwrite_does_not_lose_files_with_colliding_targets(self):
        self._write("DCIM/A/IMG_20240101_120000.jpg", b"one")
        self._write("DCIM/B/IMG_20240101_120000.jpg", b"two")

        result = self._run(conflict="overwrite")

        self.assertEqual(result["processed"], 2)
        target = self._photo_path()
        alternate = target.with_name(target.stem + "__1.jpg")
        self.assertEqual(
            sorted([target.read_bytes(), alternate.read_bytes()]),
            [b"one", b"two"],
        )

    def test_same_size_change_is_detected_by_mtime(self):
        source = self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        self._run()

        source.write_bytes(b"PHOTO2")  # same length
        later = time.time() + 300
        os.utime(source, (later, later))

        result = self._run()
        self.assertEqual(result["processed"], 1)
        alternate = self._photo_path().with_name(
            self._photo_path().stem + "__1.jpg"
        )
        self.assertEqual(alternate.read_bytes(), b"PHOTO2")

    def test_overlapping_selected_folders_copy_each_file_once(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo")

        result = self._run(folders=("DCIM", "DCIM/Camera"))

        self.assertEqual(result["processed"], 1)
        self.assertEqual(result["errors"], 0)
        self.assertEqual(list(self.dest.rglob("*__1*")), [])

    def test_media_filter(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo")
        self._write("DCIM/Camera/VID_20240102_130000.mp4", b"video")

        photo_result = self._run(media_filter="photos")
        self.assertEqual(photo_result["processed"], 1)
        self.assertTrue(self._photo_path().exists())
        self.assertFalse(
            (self.dest / "2024" / "2024-01-02_13-00-00_VID_20240102_130000.mp4").exists()
        )

        video_result = self._run(media_filter="videos")
        self.assertEqual(video_result["processed"], 1)
        self.assertTrue(
            (self.dest / "2024" / "2024-01-02_13-00-00_VID_20240102_130000.mp4").exists()
        )

    def test_cancel_before_copy(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"one")
        self._write("DCIM/Camera/IMG_20240102_130000.jpg", b"two")
        cancel = threading.Event()
        cancel.set()

        result = self._run(cancel_event=cancel)
        self.assertTrue(result["cancelled"])
        self.assertEqual(result["processed"], 0)
        # The destination root may be created during setup, but no media may
        # have been copied.
        copied = [path for path in self.dest.rglob("*") if path.is_file()] if self.dest.exists() else []
        self.assertEqual(copied, [])

    def test_legacy_state_is_respected(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        legacy = get_state_file("device-legacy")
        legacy.parent.mkdir(parents=True, exist_ok=True)
        legacy.write_text(
            '["DCIM/Camera/IMG_20240101_120000.jpg"]', encoding="utf-8"
        )

        result = self._run(device="device-legacy")
        self.assertEqual(result["processed"], 0)
        self.assertEqual(result["skipped"], 1)
        self.assertFalse(self._photo_path().exists())

    def test_destination_inside_phone_is_rejected(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo1")
        result = self._run(destination=self.phone_root / "backup")
        self.assertIn("inside the phone storage", result["error"])

    def test_missing_phone_is_reported(self):
        result = run_backup(
            "device-1",
            str(self.root / "missing-phone"),
            ["DCIM"],
            str(self.dest),
            "year",
            "both",
            "keep",
            lambda *args: None,
            threading.Event(),
        )
        self.assertIn("not found", result["error"])

    def test_missing_folders_is_reported(self):
        result = self._run(folders=("DoesNotExist",))
        self.assertIn("No readable folders", result["error"])

    def test_sort_modes(self):
        self._write("DCIM/Camera/IMG_20240101_120000.jpg", b"photo")
        result = self._run(sort_mode="day")
        self.assertEqual(result["processed"], 1)
        self.assertTrue(
            (
                self.dest
                / "2024"
                / "2024-01-01"
                / "2024-01-01_12-00-00_IMG_20240101_120000.jpg"
            ).exists()
        )


if __name__ == "__main__":
    unittest.main()
