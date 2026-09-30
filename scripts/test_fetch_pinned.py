#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Pinned-download checks with synthetic bytes; no network."""
import hashlib
import io
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock
import zipfile

import fetch_pinned as fetch


def fake_downloader(payload: bytes):
    def download(url, destination, limit):
        destination.write_bytes(payload)
    return download


class PinTable(unittest.TestCase):
    def test_every_pin_is_https_hashed_bounded_and_documented(self):
        for name, pin in fetch.PINS.items():
            with self.subTest(name=name):
                self.assertTrue(pin["url"].startswith("https://"))
                self.assertRegex(pin["sha256"], r"^[0-9a-f]{64}$")
                self.assertIn(pin["kind"], ("file", "zip", "7z"))
                self.assertLessEqual(pin["max_bytes"], 128 * fetch.MIB)
                self.assertTrue(pin["version"] and pin["license"] and pin["source"].startswith("https://"))
                self.assertIn(pin["version"].split("-")[0], pin["url"])
                self.assertEqual(Path(pin["output"]).name, pin["output"])

    def test_helpers_match_the_macos_bundle_versions(self):
        self.assertEqual({fetch.PINS[n]["version"] for n in ("yt-dlp-linux-x86_64", "yt-dlp-windows-x86_64")},
                         {"2026.08.19"})
        self.assertEqual({fetch.PINS[n]["version"] for n in ("deno-linux-x86_64", "deno-windows-x86_64")}, {"2.9.7"})


class Fetching(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name) / "pinned"

    def pin(self, payload: bytes, **extra):
        pin = {"version": "1", "kind": "file", "output": "yt-dlp", "max_bytes": 1024, "url": "https://example.invalid/x",
               "sha256": hashlib.sha256(payload).hexdigest(), "license": "x", "source": "https://example.invalid"}
        pin.update(extra)
        return mock.patch.dict(fetch.PINS, {"synthetic": pin})

    def test_matching_file_is_installed_executable(self):
        with self.pin(b"helper"):
            output = fetch.fetch("synthetic", self.directory, fake_downloader(b"helper"))
        self.assertEqual(output.read_bytes(), b"helper")
        self.assertTrue(output.stat().st_mode & 0o111)
        with self.pin(b"helper"), self.assertRaisesRegex(fetch.PinError, "overwrite"):
            fetch.fetch("synthetic", self.directory, fake_downloader(b"helper"))

    def test_mismatch_and_oversize_leave_nothing_behind(self):
        with self.pin(b"helper"), self.assertRaisesRegex(fetch.PinError, "SHA-256"):
            fetch.fetch("synthetic", self.directory, fake_downloader(b"tampered"))
        with self.pin(b"x" * 2048), self.assertRaisesRegex(fetch.PinError, "size"):
            fetch.fetch("synthetic", self.directory, fake_downloader(b"x" * 2048))
        self.assertEqual(list(self.directory.iterdir()), [])
        with self.assertRaisesRegex(fetch.PinError, "Unknown"):
            fetch.fetch("not-pinned", self.directory, fake_downloader(b""))

    def test_zip_member_is_extracted_only_after_verification(self):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as bundle:
            bundle.writestr("deno", b"runtime")
            bundle.writestr("README", b"ignored")
        payload = buffer.getvalue()
        with self.pin(payload, kind="zip", member="deno", output="deno"):
            output = fetch.fetch("synthetic", self.directory, fake_downloader(payload))
        self.assertEqual(output.read_bytes(), b"runtime")
        with self.pin(payload, kind="zip", member="deno.exe", output="deno.exe"), \
                self.assertRaisesRegex(fetch.PinError, "member"):
            fetch.fetch("synthetic", self.directory, fake_downloader(payload))

    def test_7z_requires_the_tool(self):
        with self.pin(b"7z", kind="7z", output="mpv-dev"), mock.patch("shutil.which", return_value=None), \
                self.assertRaisesRegex(fetch.PinError, "7-Zip"):
            fetch.fetch("synthetic", self.directory, fake_downloader(b"7z"))

    def test_provenance_records_the_exact_pin(self):
        record = fetch.provenance(["deno-linux-x86_64"])[0]
        self.assertEqual(record["sha256"], fetch.PINS["deno-linux-x86_64"]["sha256"])
        self.assertTrue(re.match(r"^https://github.com/denoland/deno/", record["url"]))


if __name__ == "__main__":
    unittest.main()
