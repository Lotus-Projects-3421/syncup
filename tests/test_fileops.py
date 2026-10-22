"""Tests for atomic copy helpers."""

from __future__ import annotations

import io
import json
import os
import tempfile
import unittest
from pathlib import Path

from core import fileops


class FileOpsTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)

    def _partial_files(self):
        return list(self.root.rglob("*.part-*")) + list(self.root.rglob("*.part-*"))

    def test_copy_file_atomic_copies_content_and_mtime(self):
        source = self.root / "source.jpg"
        source.write_bytes(b"hello")
        os.utime(source, (1_700_000_000, 1_700_000_000))
        destination = self.root / "nested" / "destination.jpg"

        result = fileops.copy_file_atomic(source, destination)

        self.assertEqual(result, destination)
        self.assertEqual(destination.read_bytes(), b"hello")
        self.assertEqual(int(destination.stat().st_mtime), 1_700_000_000)
        self.assertEqual(list(self.root.rglob("*.part-*")), [])

    def test_copy_file_atomic_overwrites_only_after_success(self):
        source = self.root / "source.jpg"
        source.write_bytes(b"new")
        destination = self.root / "destination.jpg"
        destination.write_bytes(b"old")

        fileops.copy_file_atomic(source, destination)
        self.assertEqual(destination.read_bytes(), b"new")

    def test_failed_copy_preserves_destination_and_cleans_temp(self):
        destination = self.root / "destination.jpg"
        destination.write_bytes(b"old")

        with self.assertRaises(OSError):
            fileops.copy_file_atomic(self.root / "missing.jpg", destination)

        self.assertEqual(destination.read_bytes(), b"old")
        self.assertEqual(list(self.root.rglob("*.part-*")), [])

    def test_copy_stream_atomic_verifies_size(self):
        destination = self.root / "stream.bin"
        fileops.copy_stream_atomic(io.BytesIO(b"abc"), destination, expected_size=3)
        self.assertEqual(destination.read_bytes(), b"abc")

        bad_destination = self.root / "bad.bin"
        with self.assertRaises(OSError):
            fileops.copy_stream_atomic(
                io.BytesIO(b"abc"), bad_destination, expected_size=99
            )
        self.assertFalse(bad_destination.exists())

    def test_unique_path(self):
        destination = self.root / "photo.jpg"
        destination.write_bytes(b"1")
        self.assertEqual(fileops.unique_path(destination).name, "photo__1.jpg")
        (self.root / "photo__1.jpg").write_bytes(b"2")
        self.assertEqual(fileops.unique_path(destination).name, "photo__2.jpg")

    def test_atomic_write_json(self):
        target = self.root / "data" / "state.json"
        fileops.atomic_write_json(target, {"a": 1})
        self.assertEqual(json.loads(target.read_text()), {"a": 1})
        self.assertEqual(list(self.root.rglob("*.part-*")), [])

    def test_is_same_file_uses_size_and_mtime(self):
        source = self.root / "a.jpg"
        destination = self.root / "b.jpg"
        source.write_bytes(b"abc")
        destination.write_bytes(b"abc")
        os.utime(destination, (source.stat().st_atime, source.stat().st_mtime))
        self.assertTrue(fileops.is_same_file(source, destination))

        destination.write_bytes(b"abcd")
        self.assertFalse(fileops.is_same_file(source, destination))

    def test_human_size(self):
        self.assertEqual(fileops.human_size(0), "0 B")
        self.assertEqual(fileops.human_size(1024), "1.0 KB")
        self.assertEqual(fileops.human_size(1024 * 1024), "1.0 MB")


if __name__ == "__main__":
    unittest.main()
