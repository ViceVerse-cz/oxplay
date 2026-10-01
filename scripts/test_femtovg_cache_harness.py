#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""GPU-free harness input regressions; production cache tests live in FemtoVG."""

import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from check_femtovg_cache_policy import prepare_package, RELEASE_LOCK_SHA256


ROOT = Path(__file__).resolve().parents[1]


class CacheHarnessInputs(unittest.TestCase):
    def test_original_release_lock_survives_clean_checkout(self):
        # Copy only Git-indexed paths, matching a fresh actions/checkout rather
        # than the developer's ignored archive files. This caught the hosted
        # failure: upstream's .gitignore silently excluded Cargo.lock.
        if not (ROOT / ".git").exists():
            self.skipTest("The source archive has no Git index; lock identity is checked by the harness")
        listing = subprocess.check_output(
            ["git", "-C", str(ROOT), "ls-files", "-z", "--", "vendor/femtovg"], timeout=10,
        )
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            checkout = folder / "checkout"
            for raw in listing.split(b"\0"):
                if not raw:
                    continue
                relative = Path(raw.decode()).relative_to("vendor/femtovg")
                target = checkout / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / "vendor/femtovg" / relative, target)
            manifest, source = prepare_package(checkout, folder / "standalone")
            self.assertEqual(hashlib.sha256((manifest.parent / "Cargo.lock").read_bytes()).hexdigest(),
                             RELEASE_LOCK_SHA256)
            self.assertEqual(source.read_bytes(), (ROOT / "vendor/femtovg/src/renderer/wgpu.rs").read_bytes())
            self.assertIn("\n[workspace]\n", manifest.read_text())

    def test_missing_lock_stops_before_creating_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            vendored = folder / "vendor"
            vendored.mkdir()
            package = folder / "package"
            with self.assertRaisesRegex(ValueError, "Missing retained"):
                prepare_package(vendored, package)
            self.assertFalse(package.exists())

    def test_changed_lock_cannot_silently_resolve_different_dependencies(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            vendored = folder / "vendor"
            vendored.mkdir()
            (vendored / "Cargo.lock").write_text("version = 4\n")
            package = folder / "package"
            with self.assertRaisesRegex(ValueError, "does not match"):
                prepare_package(vendored, package)
            self.assertFalse(package.exists())


if __name__ == "__main__":
    unittest.main()
