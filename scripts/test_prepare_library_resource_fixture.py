# SPDX-License-Identifier: GPL-3.0-or-later
"""Preparation safety tests; raster subprocess is synthetic and never launched."""
import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("preparer", Path(__file__).with_name("prepare_library_resource_fixture.py"))
PREPARER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARER)


class Preparation(unittest.TestCase):
    def test_existing_destination_is_never_reused_or_mutated(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = parent / "existing"
            root.mkdir()
            sentinel = root / "sentinel"
            sentinel.write_bytes(b"preserve")
            clip = parent / "clip"
            clip.write_bytes(b"test")
            with patch.object(PREPARER.subprocess, "Popen") as process:
                with self.assertRaises(FileExistsError):
                    PREPARER.prepare(root, clip, clip)
                process.assert_not_called()
            self.assertEqual(list(root.iterdir()), [sentinel])
            self.assertEqual(sentinel.read_bytes(), b"preserve")

    def test_failed_raster_preparation_has_no_admission_marker(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = parent / "new"
            clip = parent / "clip"
            clip.write_bytes(b"synthetic local subprocess input")
            with patch.object(PREPARER.subprocess, "Popen") as process:
                process.return_value.wait.return_value = 1
                with self.assertRaises(ValueError):
                    PREPARER.prepare(root, clip, clip)
            self.assertFalse((root / ".serein-library-resource-fixture-v1").exists())
            self.assertEqual(root.stat().st_mode & 0o777, 0o700)
            database = root / "Serein/library.sqlite3"
            self.assertEqual(database.stat().st_mode & 0o777, 0o600)
            with sqlite3.connect(database) as connection:
                self.assertEqual(connection.execute("PRAGMA user_version").fetchone()[0], 4)
                self.assertEqual(connection.execute("SELECT count(*) FROM local_playlist_items").fetchone()[0], 10000)
                self.assertEqual(connection.execute("SELECT count(*) FROM local_playlist_items WHERE title NOT LIKE 'TEST FIXTURE%' ").fetchone()[0], 0)
                self.assertEqual(connection.execute("SELECT local_history,autoplay,thumbnail_previews,background_refresh,telemetry FROM local_preferences").fetchone(), (0, 0, 0, 0, 0))


if __name__ == "__main__":
    unittest.main()
