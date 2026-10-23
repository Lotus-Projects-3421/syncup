"""Tests for structured progress telemetry."""

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

from core.dedup_engine import run_dedup
from core.engine import run_backup
from core.takeout_engine import run_takeout
from core.telemetry import ProgressEmitter


class TelemetryEmitterTests(unittest.TestCase):
    def test_event_shape_and_percentage(self):
        events = []
        emitter = ProgressEmitter(lambda c, t, m, e: events.append(e))
        event = emitter.emit(
            "transfer",
            "Copying a.jpg...",
            kind="progress",
            current=45,
            total=300,
            level="organize",
            metrics={"filename": "a.jpg"},
            bytes_done=100,
        )

        self.assertIs(events[0], event)
        self.assertEqual(event["version"], 1)
        self.assertEqual(event["phase"], "transfer")
        self.assertEqual(event["kind"], "progress")
        self.assertEqual(event["current"], 45)
        self.assertEqual(event["total"], 300)
        self.assertEqual(event["percent"], 15.0)
        self.assertFalse(event["indeterminate"])
        self.assertEqual(event["metrics"]["filename"], "a.jpg")
        self.assertEqual(event["metrics"]["bytes_done"], 100)
        # The payload must always be JSON serialisable.
        json.dumps(event)

    def test_legacy_three_argument_callback_still_works(self):
        calls = []
        emitter = ProgressEmitter(lambda c, t, m: calls.append((c, t, m)))
        emitter.emit("scan", "Scanning...", current=0, total=0)
        self.assertEqual(calls, [(0, 0, "Scanning...")])

    def test_indeterminate_defaults_for_zero_totals(self):
        events = []
        emitter = ProgressEmitter(lambda c, t, m, e: events.append(e))
        event = emitter.emit("scan", "Scanning...")
        self.assertTrue(event["indeterminate"])
        self.assertEqual(event["percent"], 0.0)


class EngineTelemetryTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self._env = mock.patch.dict(
            os.environ, {"XDG_CONFIG_HOME": str(self.root / "config")}
        )
        self._env.start()
        self.addCleanup(self._env.stop)

    def test_backup_emits_phases_and_additive_result_fields(self):
        phone = self.root / "phone"
        phone_root = phone / "Internal shared storage" / "DCIM"
        phone_root.mkdir(parents=True)
        (phone_root / "IMG_20240101_120000.jpg").write_bytes(b"photo")
        dest = self.root / "dest"
        events = []

        result = run_backup(
            "telemetry-device",
            str(phone),
            ["DCIM"],
            str(dest),
            "year",
            "both",
            "keep",
            lambda c, t, m, e: events.append(e),
            threading.Event(),
        )

        phases = {event["phase"] for event in events}
        self.assertTrue({"scan", "state", "transfer", "organize", "done"} <= phases)
        self.assertEqual(result["detected"]["total"], 1)
        self.assertEqual(result["date_sources"]["filename"], 1)
        self.assertIn("started_at", result)
        self.assertIn("finished_at", result)
        self.assertIn("duration_seconds", result)
        self.assertIn("oldest_backup", result)
        self.assertTrue(any("Checking database" in event["message"] for event in events))
        self.assertTrue(
            any("Extracted dates:" in event["message"] for event in events)
        )

    def test_takeout_emits_zip_counts_and_date_sources(self):
        source = self.root / "source"
        source.mkdir()
        media = source / "IMG_20240102_130000.jpg"
        media.write_bytes(b"photo")
        timestamp = datetime(2024, 5, 6, 7, 8, 9).timestamp()
        media.with_name(media.name + ".json").write_text(
            json.dumps({"photoTakenTime": {"timestamp": str(int(timestamp))}}),
            encoding="utf-8",
        )
        archive_path = source / "takeout.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            archive.writestr("Google Photos/VID_20240103_140000.mp4", b"video")

        events = []
        result = run_takeout(
            [source],
            str(self.root / "dest"),
            "year",
            "both",
            "keep",
            lambda c, t, m, e: events.append(e),
            threading.Event(),
        )

        self.assertEqual(result["detected"]["zips"], 1)
        self.assertEqual(result["detected"]["loose"], 1)
        self.assertEqual(result["detected"]["zip_members"], 1)
        self.assertEqual(result["date_sources"]["json"], 1)
        self.assertEqual(result["date_sources"]["filename"], 1)
        messages = " ".join(event["message"] for event in events)
        self.assertIn("ZIP files", messages)
        self.assertIn("media entries", messages)
        self.assertIn("Extracted dates:", messages)

    def test_dedup_logs_exact_duplicate_bytes_before_moving(self):
        scan = self.root / "scan"
        (scan / "a").mkdir(parents=True)
        (scan / "b").mkdir(parents=True)
        (scan / "a" / "one.jpg").write_bytes(b"same")
        (scan / "b" / "two.jpg").write_bytes(b"same")
        events = []

        result = run_dedup(
            str(scan),
            lambda c, t, m, e: events.append(e),
            threading.Event(),
            dry_run=True,
        )

        byte_events = [
            event
            for event in events
            if event["kind"] == "log" and event["metrics"].get("bytes") == 4
        ]
        self.assertTrue(byte_events)
        self.assertEqual(result["bytes"], 4)
        self.assertIn("4 bytes", byte_events[0]["message"])
        self.assertIn("started_at", result)
        self.assertIn("duration_seconds", result)


if __name__ == "__main__":
    unittest.main()
