#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Repository trust-boundary checks; native signing runs in the publishing workflow
(packaging/repositories/smoke-deb.sh exercises real gpg/apt on Ubuntu)."""
import argparse
import hashlib
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


build = load("repository_build", "packaging/repositories/build.py")
downloads = load("repository_downloads", "packaging/repositories/verify_downloads.py")


class RepositoryInputs(unittest.TestCase):
    def test_downloads_require_complete_matching_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "oxplay.deb"
            package.write_bytes(b"synthetic package")
            manifest = root / "SHA256SUMS.txt"
            entry = hashlib.sha256(package.read_bytes()).hexdigest() + "  ./oxplay.deb\n"
            manifest.write_text(entry + "0" * 64 + "  ./macOS.zip\n")
            downloads.verify(root)
            package.write_bytes(b"corrupted")
            with self.assertRaisesRegex(ValueError, "mismatch"):
                downloads.verify(root)
            package.write_bytes(b"synthetic package")
            (root / "unlisted.rpm").write_bytes(b"unlisted")
            with self.assertRaisesRegex(ValueError, "missing"):
                downloads.verify(root)
            (root / "unlisted.rpm").unlink()
            manifest.write_text(entry + entry)
            with self.assertRaisesRegex(ValueError, "duplicate"):
                downloads.verify(root)
            manifest.write_text("0" * 64 + "  ../escape.deb\n")
            with self.assertRaisesRegex(ValueError, "unsafe"):
                downloads.verify(root)
            if os.name != "nt":
                manifest.write_text(entry)
                package.unlink()
                package.symlink_to(manifest)
                with self.assertRaisesRegex(ValueError, "unsafe"):
                    downloads.verify(root)

    def test_rejects_unsafe_paths_keys_and_urls(self):
        good = dict(distribution="ubuntu-24.04", architecture="amd64",
                    key="A" * 40, base_url="https://viceverse-cz.github.io/oxplay")
        build.validate(argparse.Namespace(**good))
        for field, value in [("distribution", "../escape"), ("architecture", "/amd64"),
                             ("key", "ABC123"), ("key", "A" * 40 + "\n"),
                             ("base_url", "http://example.org"),
                             ("base_url", "https://user:secret@example.org"),
                             ("base_url", "https://example.org/\nenabled=0"),
                             ("base_url", "https://example.org/?token=secret"),
                             ("base_url", "https://example.org/$(id)")]:
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                build.validate(argparse.Namespace(**(good | {field: value})))

    @unittest.skipIf(shutil.which("sh") is None, "POSIX shell unavailable")
    def test_setup_script_syntax_and_fingerprint_placeholder(self):
        setup = ROOT / "packaging/repositories/setup.sh"
        for script in (setup, ROOT / "packaging/repositories/smoke-deb.sh"):
            result = subprocess.run(["sh", "-n", str(script)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
        content = setup.read_text()
        self.assertIn('EXPECTED_FINGERPRINT="${OXPLAY_FINGERPRINT:-@FINGERPRINT@}"', content)
        self.assertIn("ubuntu:24.04) REPO_PATH=\"ubuntu-24.04/amd64/apt\"", content)
        # An unsubstituted placeholder must stop before any download or sudo.
        result = subprocess.run(["sh", str(setup)], capture_output=True, text=True,
                                env={"PATH": os.environ.get("PATH", "/usr/bin:/bin")})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No signing key fingerprint", result.stderr)

    def test_generate_index(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "site"
            build.generate_index(destination, "https://packages.example.org/oxplay")
            content = (destination / "index.html").read_text()
            self.assertIn("Oxplay Linux Repositories", content)
            self.assertIn("https://packages.example.org/oxplay/setup.sh", content)


if __name__ == "__main__":
    unittest.main()
