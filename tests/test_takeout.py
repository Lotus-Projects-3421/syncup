"""Tests for the Takeout organizer, including ZIP streaming."""

from __future__ import annotations

import json
import os
import tempfile
import threading
import unittest
import zipfile
from datetime import datetime
from pathlib import Path
from unittest import mock

from core.takeout_engine import get_date_from_json_data, run_takeout


class TakeoutTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self._env = mock.patch.dict(
            os.environ, {"XDG_CONFIG_HOME": str(self.root / "config")}
        )
        self._env.start()
        self.addCleanup(self._env.stop)
        self.source = self.root / "source"
        self.source.mkdir()
        self.dest = self.root / "dest"
        self.cancel = threading.Event()

    def _run(self, inputs, *, sort_mode="year", preserve=True, filter_mode="both"):
        return run_takeout(
            inputs,
            str(self.dest),
            sort_mode,
            filter_mode,
            "keep",
            lambda *args: None,
            self.cancel,
            preserve_metadata=preserve,
        )

    def _write_json(self, path: Path, timestamp: float) -> None:
        path.write_text(
            json.dumps({"photoTakenTime": {"timestamp": str(int(timestamp))}}),
            encoding="utf-8",
        )

    def test_loose_file_uses_sidecar_timestamp_and_preserves_metadata(self):
        media = self.source / "Google Photos" / "IMG_20240102_130000.jpg"
        media.parent.mkdir(parents=True)
        media.write_bytes(b"photo")
        timestamp = datetime(2024, 5, 6, 7, 8, 9).timestamp()
        self._write_json(media.with_name(media.name + ".json"), timestamp)

        result = self._run([self.source], sort_mode="day")

        self.assertEqual(result["processed"], 1, result)
        self.assertEqual(result["errors"], 0, result)
        target = (
            self.dest
            / "2024"
            / "2024-05-06"
            / "2024-05-06_07-08-09_IMG_20240102_130000.jpg"
        )
        self.assertEqual(target.read_bytes(), b"photo")
        self.assertTrue(target.with_name(target.name + ".json").exists())

    def test_supplemental_metadata_sidecar_is_found(self):
        media = self.source / "IMG_20240102_130000.jpg"
        media.write_bytes(b"photo")
        timestamp = datetime(2024, 5, 6, 7, 8, 9).timestamp()
        self._write_json(
            media.with_name(media.name + ".supplemental-metadata.json"),
            timestamp,
        )

        result = self._run([self.source], sort_mode="day")

        self.assertEqual(result["processed"], 1, result)
        target = (
            self.dest
            / "2024"
            / "2024-05-06"
            / "2024-05-06_07-08-09_IMG_20240102_130000.jpg"
        )
        self.assertEqual(target.read_bytes(), b"photo")
        self.assertTrue(target.with_name(target.name + ".json").exists())

    def test_zip_supplemental_metadata_sidecar_is_found(self):
        archive_path = self.source / "takeout.zip"
        timestamp = datetime(2024, 6, 7, 8, 9, 10).timestamp()
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("Google Photos/VID_20240103_140000.mp4", b"video")
            archive.writestr(
                "Google Photos/VID_20240103_140000.mp4.supplemental-metadata.json",
                json.dumps({"photoTakenTime": {"timestamp": str(int(timestamp))}}),
            )

        result = self._run([archive_path], sort_mode="year")

        self.assertEqual(result["processed"], 1, result)
        target = (
            self.dest / "2024" / "2024-06-07_08-09-10_VID_20240103_140000.mp4"
        )
        self.assertEqual(target.read_bytes(), b"video")
        self.assertTrue(target.with_name(target.name + ".json").exists())

    def test_rerun_is_idempotent(self):
        media = self.source / "Google Photos" / "IMG_20240102_130000.jpg"
        media.parent.mkdir(parents=True)
        media.write_bytes(b"photo")
        timestamp = datetime(2024, 5, 6, 7, 8, 9).timestamp()
        self._write_json(media.with_name(media.name + ".json"), timestamp)

        first = self._run([self.source], sort_mode="day")
        second = self._run([self.source], sort_mode="day")

        self.assertEqual(first["processed"], 1)
        self.assertEqual(second["processed"], 0)
        self.assertEqual(second["skipped"], 1)
        self.assertEqual(list(self.dest.rglob("*__1*")), [])

    def test_zip_is_streamed_and_sidecar_is_preserved(self):
        archive_path = self.source / "takeout.zip"
        timestamp = datetime(2024, 6, 7, 8, 9, 10).timestamp()
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("Google Photos/VID_20240103_140000.mp4", b"video")
            archive.writestr(
                "Google Photos/VID_20240103_140000.mp4.json",
                json.dumps({"photoTakenTime": {"timestamp": str(int(timestamp))}}),
            )

        result = self._run([archive_path], sort_mode="year")

        self.assertEqual(result["processed"], 1, result)
        target = (
            self.dest
            / "2024"
            / "2024-06-07_08-09-10_VID_20240103_140000.mp4"
        )
        self.assertEqual(target.read_bytes(), b"video")
        self.assertTrue(target.with_name(target.name + ".json").exists())

        second = self._run([archive_path], sort_mode="year")
        self.assertEqual(second["processed"], 0)
        self.assertEqual(second["skipped"], 1)

    def test_overwrite_does_not_lose_files_with_colliding_targets(self):
        first = self.source / "A" / "IMG_20240102_130000.jpg"
        second = self.source / "B" / "IMG_20240102_130000.jpg"
        first.parent.mkdir(parents=True)
        second.parent.mkdir(parents=True)
        first.write_bytes(b"one")
        second.write_bytes(b"two")

        result = run_takeout(
            [self.source],
            str(self.dest),
            "year",
            "both",
            "overwrite",
            lambda *args: None,
            self.cancel,
        )

        self.assertEqual(result["processed"], 2)
        target = self.dest / "2024" / "2024-01-02_13-00-00_IMG_20240102_130000.jpg"
        alternate = target.with_name(target.stem + "__1.jpg")
        self.assertEqual(
            sorted([target.read_bytes(), alternate.read_bytes()]),
            [b"one", b"two"],
        )

    def test_metadata_can_be_disabled(self):
        media = self.source / "IMG_20240102_130000.jpg"
        media.write_bytes(b"photo")
        self._write_json(
            media.with_name(media.name + ".json"),
            datetime(2024, 5, 6, 7, 8, 9).timestamp(),
        )

        result = self._run([media], preserve=False)
        self.assertEqual(result["processed"], 1)
        self.assertEqual(list(self.dest.rglob("*.json")), [])

    def test_input_folder_inside_destination_is_processed(self):
        source = self.dest / "Takeout"
        source.mkdir(parents=True)
        (source / "IMG_20240102_130000.jpg").write_bytes(b"photo")

        result = self._run([source], sort_mode="year")

        self.assertEqual(result["processed"], 1, result)
        self.assertEqual(
            (self.dest / "2024" / "2024-01-02_13-00-00_IMG_20240102_130000.jpg")
            .read_bytes(),
            b"photo",
        )

    def test_invalid_zip_is_reported_without_crashing(self):
        bad_zip = self.source / "bad.zip"
        bad_zip.write_bytes(b"this is not a zip")
        result = self._run([bad_zip])
        self.assertEqual(result["processed"], 0)
        self.assertEqual(result["errors"], 1)

    def test_sidecar_failure_is_counted(self):
        media = self.source / "IMG_20240102_130000.jpg"
        media.write_bytes(b"photo")
        self._write_json(
            media.with_name(media.name + ".json"),
            datetime(2024, 5, 6, 7, 8, 9).timestamp(),
        )

        with mock.patch(
            "core.takeout_engine._copy_loose_sidecar",
            return_value="permission denied",
        ):
            result = self._run([self.source])

        self.assertEqual(result["processed"], 1, result)
        self.assertEqual(result["errors"], 1, result)
        self.assertTrue(any("metadata copy failed" in line for line in result["error_details"]))

    def test_no_matching_media(self):
        (self.source / "notes.txt").write_text("hello")
        result = self._run([self.source])
        self.assertEqual(result["total"], 0)
        self.assertEqual(result["errors"], 0)

    def test_cancel_before_processing(self):
        media = self.source / "IMG_20240102_130000.jpg"
        media.write_bytes(b"photo")
        self.cancel.set()
        result = self._run([self.source])
        self.assertTrue(result["cancelled"])
        self.assertEqual(result["processed"], 0)

    def test_bad_json_is_ignored(self):
        self.assertIsNone(get_date_from_json_data({"photoTakenTime": {"timestamp": "x"}}))
        self.assertIsNone(get_date_from_json_data("not a dict"))
        self.assertIsNone(get_date_from_json_data({}))


if __name__ == "__main__":
    unittest.main()
