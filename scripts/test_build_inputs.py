#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Small synthetic repositories only; no project builds, network or native app."""
import json
import hashlib
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_inputs as inventory


class BuildInputTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.git = inventory.Git(self.root)
        self.git.run(["init", "-q"])
        for name, raw in {
            "Cargo.toml": b"[workspace]\nmembers=[]\n", "Cargo.lock": b"version=4\n",
            "rust-toolchain.toml": b'[toolchain]\nchannel="stable"\n',
            ".cargo/config.toml": b"[net]\noffline=true\n",
            "crates/app/src/main.rs": b"fn main() {}\n",
            "crates/media/src/native_child.m": b"/* synthetic native input */\n",
            "crates/media/build.rs": b"fn main() {}\n",
            "crates/app/ui/icons/synthetic.svg": b"<svg/>\n",
            "README.md": b"not a selected build input\n",
            ".gitignore": b"artifacts/\ntarget/\ncrates/private/\n",
        }.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        self.git.run(["add", "."])
        self.git.run(["-c", "user.name=Synthetic fixture", "-c", "user.email=fixture@example.invalid",
                      "-c", "commit.gpgsign=false", "commit", "-qm", "Synthetic build inputs"])

    def test_complete_commit_match_includes_native_assets_and_configuration(self):
        report = inventory.capture(self.root)
        self.assertTrue(report["inputs_match_commit"])
        self.assertFalse(report["binary_build_association_verified"])
        self.assertIsNone(report["repository_clean"])
        self.assertEqual(report["file_count"], 8)
        self.assertEqual(report, inventory.capture(self.root))
        paths = {item["path"] for item in report["files"]}
        self.assertIn("crates/media/src/native_child.m", paths)
        self.assertIn("crates/media/build.rs", paths)
        self.assertIn("crates/app/ui/icons/synthetic.svg", paths)
        self.assertIn(".cargo/config.toml", paths)
        self.assertNotIn(str(self.root), json.dumps(report))

    def test_modified_untracked_and_added_index_are_distinct(self):
        (self.root / "crates/app/src/main.rs").write_text("fn main() { panic!(); }\n")
        (self.root / "crates/app/src/new.rs").write_text("// synthetic untracked\n")
        (self.root / "crates/app/src/staged.rs").write_text("// synthetic staged\n")
        self.git.run(["add", "crates/app/src/staged.rs"])
        report = inventory.capture(self.root)
        states = {item["path"]: item["state"] for item in report["files"]}
        self.assertEqual(states["crates/app/src/main.rs"], "modified")
        self.assertEqual(states["crates/app/src/new.rs"], "untracked")
        self.assertEqual(states["crates/app/src/staged.rs"], "added_index")
        self.assertFalse(report["inputs_match_commit"])

    def test_vendored_renderer_and_native_build_configuration_are_attested(self):
        files = {
            "vendor/femtovg/Cargo.toml": b'[package]\nname="femtovg"\n',
            "vendor/femtovg/src/renderer/wgpu.rs": b"// synthetic renderer implementation\n",
            "vendor/femtovg/LICENSE-MIT": b"Synthetic attribution fixture\n",
            "vendor/slint-femtovg/Cargo.toml": b'[package]\nname="i-slint-renderer-femtovg"\n',
            "vendor/slint-femtovg/itemrenderer.rs": b"// synthetic rounded image clipping\n",
            "vendor/slint-femtovg/LICENSE-GPL-3.0": b"Synthetic renderer license fixture\n",
            "scripts/native-media/macos.py": b"# synthetic native build recipe\n",
            "scripts/native-media/macos-sources.json": b'{"sources": {}}\n',
            "scripts/native-media/patches/common.patch": b"Synthetic native patch\n",
            "scripts/native-media/check_abi.c": b"/* synthetic native ABI check */\n",
        }
        for name, raw in files.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        self.git.run(["add", "vendor", "scripts/native-media"])
        self.git.run(["-c", "user.name=Synthetic fixture", "-c", "user.email=fixture@example.invalid",
                      "-c", "commit.gpgsign=false", "commit", "-qm", "Synthetic native build inputs"])
        report = inventory.capture(self.root)
        self.assertTrue(report["inputs_match_commit"])
        rows = {item["path"]: item for item in report["files"]}
        for name, raw in files.items():
            self.assertEqual(rows[name]["sha256"], hashlib.sha256(raw).hexdigest(), name)
            self.assertEqual(rows[name]["state"], "matches_commit", name)

        # A path-dependency edit or applied native patch must invalidate the
        # generic source attestation even when application Rust is unchanged.
        for name in ("vendor/femtovg/src/renderer/wgpu.rs", "vendor/slint-femtovg/itemrenderer.rs",
                     "scripts/native-media/patches/common.patch"):
            (self.root / name).write_bytes(files[name] + b"Synthetic changed bytes\n")
        report = inventory.capture(self.root)
        self.assertFalse(report["inputs_match_commit"])
        states = {item["path"]: item["state"] for item in report["files"]}
        self.assertEqual(states["vendor/femtovg/src/renderer/wgpu.rs"], "modified")
        self.assertEqual(states["vendor/slint-femtovg/itemrenderer.rs"], "modified")
        self.assertEqual(states["scripts/native-media/patches/common.patch"], "modified")

        # Downloaded archives and installed libraries are separately attested
        # by native build/package manifests, not read as checkout source.
        output = self.root / "artifacts/native-media/prefix/lib/libmpv.dylib"
        output.parent.mkdir(parents=True)
        output.write_bytes(b"Synthetic private build output\n")
        self.assertEqual(report, inventory.capture(self.root))

    def test_staged_deletion_remains_in_committed_input_comparison(self):
        self.git.run(["rm", "-q", "crates/app/src/main.rs"])
        report = inventory.capture(self.root)
        row = next(item for item in report["files"] if item["path"] == "crates/app/src/main.rs")
        self.assertEqual(row["state"], "missing")
        self.assertNotIn("sha256", row)
        self.assertFalse(report["inputs_match_commit"])

    def test_ignored_and_unrelated_files_do_not_fake_dirty_inputs(self):
        for name in ("artifacts/private.json", "target/output", "crates/private/secret", "README.md"):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("Synthetic excluded bytes\n")
        report = inventory.capture(self.root)
        self.assertTrue(report["inputs_match_commit"])
        self.assertFalse(any("private" in item["path"] for item in report["files"]))

    def test_symlink_and_fifo_are_not_read(self):
        link = self.root / "crates/app/src/link.rs"
        link.symlink_to(self.root / "Cargo.toml")
        with self.assertRaises((inventory.InputError, OSError)):
            inventory.capture(self.root)
        link.unlink()
        # Git does not list an untracked FIFO. Replacing a committed input
        # ensures the capture must inspect its current type and fail closed.
        fifo = self.root / "crates/app/src/main.rs"
        fifo.unlink()
        os.mkfifo(fifo)
        with self.assertRaises(inventory.InputError):
            inventory.capture(self.root)

    def test_untracked_fifo_is_not_listed_or_read(self):
        fifo = self.root / "crates/app/src/untracked-fifo.rs"
        os.mkfifo(fifo)
        with patch.object(inventory, "inspect_file", wraps=inventory.inspect_file) as inspect:
            report = inventory.capture(self.root)
        self.assertTrue(report["inputs_match_commit"])
        name = fifo.relative_to(self.root).as_posix()
        self.assertNotIn(name, {item["path"] for item in report["files"]})
        self.assertNotIn(name, {call.args[1] for call in inspect.call_args_list})

    def test_symlink_parent_cannot_escape_checkout(self):
        source = self.root / "crates/app/src/main.rs"
        source.unlink()
        source.parent.rmdir()
        source.parent.symlink_to(self.root)
        with self.assertRaises(OSError):
            inventory.capture(self.root)

    def test_oversized_file_and_aggregate_are_rejected(self):
        with patch.object(inventory, "MAX_FILE", 8), self.assertRaises(inventory.InputError):
            inventory.capture(self.root)
        with patch.object(inventory, "MAX_TOTAL", 8), self.assertRaises(inventory.InputError):
            inventory.capture(self.root)

    def test_commit_tree_modes_and_raw_bytes_not_index_contents(self):
        path = self.root / "crates/media/build.rs"
        path.chmod(0o755)
        report = inventory.capture(self.root)
        row = next(item for item in report["files"] if item["path"] == "crates/media/build.rs")
        self.assertEqual(row["state"], "modified")
        self.assertEqual(row["mode"], "100755")

    def test_subdirectory_and_unsafe_paths_rejected(self):
        with self.assertRaises(inventory.InputError):
            inventory.capture(self.root / "crates")
        for raw in (b"/absolute", b"../outside", b"crates/../outside", b"C:/outside", b"crates/new\nname"):
            with self.subTest(raw=raw), self.assertRaises(inventory.InputError):
                inventory.relative(raw)

    def test_git_output_cap_is_enforced(self):
        with patch.object(inventory, "MAX_GIT_OUTPUT", 2), self.assertRaises(inventory.InputError):
            self.git.run(["--version"])

    def test_expired_file_budget_rejects_before_reading(self):
        descriptor = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaisesRegex(inventory.InputError, "read time budget"):
                inventory.inspect_file(descriptor, "Cargo.toml", "sha1", 0)
        finally:
            os.close(descriptor)

    def test_timeout_uses_the_shared_anchored_group_cleanup(self):
        from types import SimpleNamespace
        read_fd, write_fd = os.pipe()
        stream = os.fdopen(read_fd, "rb")
        self.addCleanup(os.close, write_fd)
        process = SimpleNamespace(stdout=stream)
        self.git.deadline = 1
        with patch.object(inventory.subprocess, "Popen", return_value=process), \
             patch.object(inventory.time, "monotonic", side_effect=[0, 0, 2]), \
             patch.object(inventory, "reap_group") as cleanup:
            with self.assertRaisesRegex(inventory.InputError, "timed out"):
                self.git.run(["--version"])
            cleanup.assert_called_once_with(process)
        self.assertTrue(stream.closed)


if __name__ == "__main__":
    unittest.main()
