"""Tests for duplicate detection, manifests, and undo."""

from __future__ import annotations

import json
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from core.dedup_engine import find_latest_manifest, run_dedup, undo_dedup


class DedupTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self.scan = self.root / "scan"
        self.scan.mkdir()
        self.cancel = threading.Event()

    def _write(self, relative: str, data: bytes) -> Path:
        path = self.scan / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def _run(self, dry_run: bool):
        return run_dedup(
            str(self.scan),
            lambda *args: None,
            self.cancel,
            dry_run=dry_run,
        )

    def test_preview_does_not_move_files(self):
        self._write("a/one.jpg", b"same")
        self._write("b/two.jpg", b"same")
        self._write("c/unique.jpg", b"different")

        result = self._run(dry_run=True)

        self.assertEqual(result["duplicates"], 1)
        self.assertEqual(result["groups"], 1)
        self.assertEqual(result["moved"], 0)
        self.assertTrue(result["dry_run"])
        self.assertTrue((self.scan / "a" / "one.jpg").exists())
        self.assertTrue((self.scan / "b" / "two.jpg").exists())
        self.assertEqual(list(self.scan.glob("_Duplicates_*")), [])

    def test_move_creates_manifest_and_undo_restores(self):
        first = self._write("a/one.jpg", b"same")
        second = self._write("b/two.jpg", b"same")
        unique = self._write("c/unique.jpg", b"different")

        result = self._run(dry_run=False)

        self.assertEqual(result["moved"], 1)
        self.assertEqual(result["errors"], 0)
        manifest_path = Path(result["manifest"])
        self.assertTrue(manifest_path.exists())
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(len(data["entries"]), 1)
        self.assertEqual(find_latest_manifest(str(self.scan)), manifest_path)

        remaining = [path for path in (first, second) if path.exists()]
        self.assertEqual(len(remaining), 1)
        self.assertTrue(unique.exists())

        undo_result = undo_dedup(
            str(self.scan),
            lambda *args: None,
            self.cancel,
        )

        self.assertEqual(undo_result["restored"], 1, undo_result)
        self.assertTrue(first.exists())
        self.assertTrue(second.exists())
        self.assertEqual(first.read_bytes(), b"same")
        self.assertEqual(second.read_bytes(), b"same")

    def test_manifest_is_written_before_moves(self):
        first = self._write("a/one.jpg", b"same")
        second = self._write("b/two.jpg", b"same")

        with mock.patch(
            "core.dedup_engine.shutil.move", side_effect=OSError("disk full")
        ):
            result = self._run(dry_run=False)

        self.assertEqual(result["moved"], 0)
        self.assertEqual(result["errors"], 1)
        manifest_path = Path(result["manifest"])
        self.assertTrue(manifest_path.exists())
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(len(data["entries"]), 1)
        self.assertTrue(first.exists())
        self.assertTrue(second.exists())

    def test_no_duplicates(self):
        self._write("a/one.jpg", b"one")
        self._write("b/two.jpg", b"two")
        result = self._run(dry_run=True)
        self.assertEqual(result["duplicates"], 0)
        self.assertEqual(result["groups"], 0)

    def test_cancel_during_hashing(self):
        self._write("a/one.jpg", b"same")
        self._write("b/two.jpg", b"same")
        self.cancel.set()
        result = self._run(dry_run=True)
        self.assertTrue(result["cancelled"])
        self.assertEqual(result["moved"], 0)

    def test_undo_malformed_entry_is_reported(self):
        duplicate_dir = self.scan / "_Duplicates_1"
        duplicate_dir.mkdir()
        manifest = duplicate_dir / "_manifest.json"
        manifest.write_text(json.dumps({"entries": ["bad"]}), encoding="utf-8")

        result = undo_dedup(
            str(self.scan), lambda *args: None, self.cancel
        )

        self.assertEqual(result["errors"], 1)
        self.assertEqual(result["restored"], 0)

    def test_undo_without_manifest_reports_error(self):
        result = undo_dedup(
            str(self.scan),
            lambda *args: None,
            self.cancel,
        )
        self.assertIn("error", result)

    def test_duplicate_folder_is_not_rescanned(self):
        self._write("a/one.jpg", b"same")
        self._write("b/two.jpg", b"same")
        self._run(dry_run=False)

        # A second run must ignore files inside _Duplicates_ and find nothing.
        result = self._run(dry_run=True)
        self.assertEqual(result["duplicates"], 0)

    def test_undo_does_not_overwrite_an_existing_original(self):
        self._write("a/one.jpg", b"same")
        self._write("b/two.jpg", b"same")
        result = self._run(dry_run=False)
        manifest = Path(result["manifest"])
        data = json.loads(manifest.read_text(encoding="utf-8"))
        original = Path(data["entries"][0]["original"])

        original.parent.mkdir(parents=True, exist_ok=True)
        original.write_bytes(b"replacement")

        undo_result = undo_dedup(
            str(self.scan),
            lambda *args: None,
            self.cancel,
        )

        self.assertEqual(undo_result["restored"], 0)
        self.assertEqual(undo_result["skipped"], 1)
        self.assertEqual(original.read_bytes(), b"replacement")


if __name__ == "__main__":
    unittest.main()
