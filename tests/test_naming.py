"""Tests for filename date parsing and destination naming."""

from __future__ import annotations

import os
import struct
import tempfile
import unittest
from datetime import datetime
from pathlib import Path

from core.naming import (
    build_target_name,
    date_for_path,
    date_with_source,
    get_target_path,
    parse_datetime_from_name,
    read_exif_datetime,
    strip_generated_prefix,
)


def _jpeg_with_exif_date(date_text: str = "2023:04:05 06:07:08") -> bytes:
    """Build a tiny little-endian TIFF/EXIF block inside a JPEG APP1 segment."""
    ascii_value = date_text.encode("ascii") + b"\x00"
    exif_ifd_offset = 8 + 18  # IFD0: count + one entry + next-IFD pointer
    value_offset = exif_ifd_offset + 18  # Exif IFD size

    tiff = b"II" + struct.pack("<H", 42) + struct.pack("<I", 8)
    tiff += struct.pack("<H", 1)
    tiff += struct.pack("<HHI", 0x8769, 4, 1) + struct.pack("<I", exif_ifd_offset)
    tiff += struct.pack("<I", 0)
    tiff += struct.pack("<H", 1)
    tiff += struct.pack("<HHI", 0x9003, 2, len(ascii_value))
    tiff += struct.pack("<I", value_offset)
    tiff += struct.pack("<I", 0)
    tiff += ascii_value

    payload = b"Exif\x00\x00" + tiff
    return (
        b"\xff\xd8"
        + b"\xff\xe1"
        + struct.pack(">H", len(payload) + 2)
        + payload
        + b"\xff\xd9"
    )


class NamingTests(unittest.TestCase):
    def test_parses_compact_date_and_time(self):
        self.assertEqual(
            parse_datetime_from_name("IMG_20240101_123456"),
            datetime(2024, 1, 1, 12, 34, 56),
        )

    def test_parses_dashed_time(self):
        self.assertEqual(
            parse_datetime_from_name("VID_2024-01-02-13-14-15"),
            datetime(2024, 1, 2, 13, 14, 15),
        )

    def test_parses_dotted_time(self):
        self.assertEqual(
            parse_datetime_from_name("Screenshot_2024-01-03 14.15.16"),
            datetime(2024, 1, 3, 14, 15, 16),
        )

    def test_whatsapp_style_name_has_no_fake_time(self):
        self.assertEqual(
            parse_datetime_from_name("IMG-20240101-WA0001"),
            datetime(2024, 1, 1, 0, 0, 0),
        )

    def test_repairs_old_midnight_prefix(self):
        self.assertEqual(
            parse_datetime_from_name("2024-01-01_00-00-00_IMG_20240101_123456"),
            datetime(2024, 1, 1, 12, 34, 56),
        )

    def test_no_date_returns_none(self):
        self.assertIsNone(parse_datetime_from_name("holiday-photo"))

    def test_invalid_date_returns_none(self):
        self.assertIsNone(parse_datetime_from_name("IMG_20241340_000000"))

    def test_strip_multiple_generated_prefixes(self):
        self.assertEqual(
            strip_generated_prefix(
                "2024-01-01_12-00-00_2024-01-01_12-00-00_IMG_1234"
            ),
            "IMG_1234",
        )

    def test_target_name_is_idempotent(self):
        date_obj = datetime(2024, 1, 1, 12, 34, 56)
        first = build_target_name(Path("IMG_20240101_123456.jpg"), date_obj)
        second = build_target_name(Path(first), date_obj)
        self.assertEqual(first, second)
        self.assertEqual(
            first, "2024-01-01_12-34-56_IMG_20240101_123456.jpg"
        )

    def test_target_path_modes(self):
        date_obj = datetime(2024, 5, 6, 7, 8, 9)
        source = Path("IMG_20240102_130000.jpg")
        root = Path("/dest")

        self.assertEqual(
            get_target_path(source, root, "year", date_obj),
            root / "2024" / "2024-05-06_07-08-09_IMG_20240102_130000.jpg",
        )
        self.assertEqual(
            get_target_path(source, root, "month", date_obj),
            root / "2024" / "2024-05" / "2024-05-06_07-08-09_IMG_20240102_130000.jpg",
        )
        self.assertEqual(
            get_target_path(source, root, "day", date_obj),
            root
            / "2024"
            / "2024-05-06"
            / "2024-05-06_07-08-09_IMG_20240102_130000.jpg",
        )
        self.assertEqual(
            get_target_path(source, root, "flat", date_obj),
            root / "2024-05-06_07-08-09_IMG_20240102_130000.jpg",
        )
        self.assertEqual(
            get_target_path(source, root, "unknown", date_obj),
            root / "2024" / "2024-05-06_07-08-09_IMG_20240102_130000.jpg",
        )

    def test_date_for_path_falls_back_to_mtime(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "no-date.jpg"
            path.write_bytes(b"x")
            timestamp = 1_700_000_000
            os.utime(path, (timestamp, timestamp))
            self.assertEqual(
                date_for_path(path),
                datetime.fromtimestamp(timestamp),
            )

    def test_exif_datetime_is_read_and_counted(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "holiday.jpg"
            path.write_bytes(_jpeg_with_exif_date("2023:04:05 06:07:08"))

            self.assertEqual(
                read_exif_datetime(path),
                datetime(2023, 4, 5, 6, 7, 8),
            )
            date_obj, source = date_with_source(path)
            self.assertEqual(date_obj, datetime(2023, 4, 5, 6, 7, 8))
            self.assertEqual(source, "exif")

    def test_filename_still_wins_over_exif(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "IMG_20240101_120000.jpg"
            path.write_bytes(_jpeg_with_exif_date("2023:04:05 06:07:08"))

            date_obj, source = date_with_source(path)
            self.assertEqual(date_obj, datetime(2024, 1, 1, 12, 0, 0))
            self.assertEqual(source, "filename")

    def test_date_with_source_falls_back_to_mtime(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "holiday.png"
            path.write_bytes(b"not an image")
            timestamp = 1_700_000_000
            os.utime(path, (timestamp, timestamp))

            date_obj, source = date_with_source(path)
            self.assertEqual(date_obj, datetime.fromtimestamp(timestamp))
            self.assertEqual(source, "mtime")


if __name__ == "__main__":
    unittest.main()
