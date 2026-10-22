"""Packaging sanity checks."""

from __future__ import annotations

import unittest
from pathlib import Path

from core.constants import VERSION

ROOT = Path(__file__).resolve().parents[1]
DESKTOP_NAME = "io.github.lotus_projects_3421.SyncUp.desktop"


class PackagingTests(unittest.TestCase):
    def test_icon_is_a_real_png(self):
        data = (ROOT / "assets" / "syncup.png").read_bytes()
        self.assertTrue(data.startswith(b"\x89PNG\r\n\x1a\n"))
        self.assertGreater(len(data), 1000)

    def test_desktop_file_points_at_the_icon(self):
        text = (ROOT / "packaging" / DESKTOP_NAME).read_text(encoding="utf-8")
        self.assertIn("Icon=syncup", text)
        self.assertIn("Exec=SyncUp", text)

    def test_desktop_file_has_a_single_main_category(self):
        text = (ROOT / "packaging" / DESKTOP_NAME).read_text(encoding="utf-8")
        self.assertIn("Categories=Graphics;Photography;", text)

    def test_appstream_metadata_exists_and_is_built(self):
        metadata_path = (
            ROOT / "packaging" / "io.github.lotus_projects_3421.SyncUp.appdata.xml"
        )
        metadata = metadata_path.read_text(encoding="utf-8")
        self.assertIn("<id>io.github.lotus_projects_3421.SyncUp</id>", metadata)
        self.assertIn(
            f'<launchable type="desktop-id">{DESKTOP_NAME}</launchable>', metadata
        )
        self.assertIn("<release version=\"@VERSION@\"", metadata)
        script = (ROOT / "build.sh").read_text(encoding="utf-8")
        self.assertIn(metadata_path.name, script)
        self.assertIn(DESKTOP_NAME, script)
        self.assertIn("usr/share/metainfo", script)
        self.assertIn("FLET_VIEW_PATH", script)
        self.assertIn("usr/lib/syncup/flet", script)
        self.assertIn("usr/share/applications", script)
        self.assertIn("usr/share/icons/hicolor", script)

    def test_spec_includes_icon_and_flet_data(self):
        text = (ROOT / "SyncUp.spec").read_text(encoding="utf-8")
        self.assertIn("assets/syncup.png", text)
        self.assertIn("collect_data_files('flet')", text)
        self.assertIn("flet_desktop", text)

    def test_version_is_consistent(self):
        pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
        self.assertIn(f'version = "{VERSION}"', pyproject)


if __name__ == "__main__":
    unittest.main()
