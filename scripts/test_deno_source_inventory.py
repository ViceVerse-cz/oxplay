#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic parser boundaries only; no Homebrew cache, network, Cargo or Deno."""
import gzip
import io
import json
from pathlib import Path
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch

import deno_source_inventory as inventory


def archive(extra=(), *, form=tarfile.PAX_FORMAT, cli_version='version = "2.9.7"'):
    buffer = io.BytesIO()
    files = [
        ("deno/Cargo.toml", b'[workspace]\nmembers=["cli"]\n[workspace.package]\nversion="2.9.7"\n'),
        ("deno/cli/Cargo.toml", ('[package]\nname="deno"\n' + cli_version + '\n').encode()),
        ("deno/Cargo.lock", b'version=4\n[[package]]\nname="synthetic-crate"\nversion="1.0.0"\nsource="registry+https://example.invalid/index"\nchecksum="' + b'a' * 64 + b'"\n'),
        ("deno/LICENSE.md", b"Synthetic original notice, not a license grant.\n"),
    ]
    with tarfile.open(fileobj=buffer, mode="w", format=form) as tar:
        for name, raw in files + list(extra):
            if isinstance(name, tarfile.TarInfo):
                member = name
            else:
                member = tarfile.TarInfo(name)
            member.size = len(raw)
            tar.addfile(member, io.BytesIO(raw))
    return buffer.getvalue()


def inspect(raw):
    return inventory.inspect_tar(io.BytesIO(raw), time.monotonic() + 10)


class DenoInventoryTests(unittest.TestCase):
    def test_candidates_and_notices_never_assert_selected_closure(self):
        report, notices = inspect(archive())
        self.assertEqual(report["members"], 4)
        self.assertEqual(report["lock_candidates"][0]["selected_for_homebrew_build"], None)
        self.assertEqual(report["original_notice_files"][0]["path"], "deno/LICENSE.md")
        notice = report["original_notice_files"][0]
        self.assertEqual(inventory.digest(notices[notice["sha256"]]), notice["sha256"])
        self.assertNotIn("example.invalid", json.dumps(report))

    def test_workspace_cli_version(self):
        report, _ = inspect(archive(cli_version="version.workspace = true"))
        self.assertEqual(len(report["lock_candidates"]), 1)

    def test_archive_checksum_precedes_parsing_and_no_commands(self):
        raw = gzip.compress(archive(), mtime=0)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "input.tar.gz"
            path.write_bytes(raw)
            with patch("subprocess.Popen", side_effect=AssertionError("No external command allowed")):
                with self.assertRaisesRegex(inventory.InventoryError, "checksum"):
                    inventory.inventory(path)
                with patch.object(inventory, "SOURCE_SHA256", inventory.digest(raw)):
                    report, _ = inventory.inventory(path)
            self.assertFalse(report["complete_corresponding_source"])
            self.assertFalse(report["binary_source_association_verified"])
            self.assertEqual(report["selected_graph_status"], "absent_not_derivable_from_Cargo.lock")
            self.assertNotIn(temporary, json.dumps(report))

    def test_notice_copies_exact_bytes_and_refuse_existing_output(self):
        report, notices = inspect(archive())
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "new"
            inventory.write_output(output, report, notices)
            self.assertFalse((output / "INCOMPLETE").exists())
            for checksum, raw in notices.items():
                self.assertEqual((output / "original-notices" / (checksum + ".txt")).read_bytes(), raw)
            with self.assertRaises(FileExistsError):
                inventory.write_output(output, report, notices)

    def test_traversal_drive_and_duplicate_paths_rejected(self):
        for name in ("../escape", "/absolute", "C:/outside", "C:relative", "deno/../outside", "deno/LICENSE.md"):
            with self.subTest(name=name), self.assertRaises(inventory.InventoryError):
                inspect(archive([(name, b"bad")]))

    def test_links_never_supply_notice_content(self):
        member = tarfile.TarInfo("deno/NOTICE")
        member.type = tarfile.SYMTYPE
        member.linkname = "/outside/source"
        report, notices = inspect(archive([(member, b"")]))
        self.assertEqual(report["unfollowed_link_members"], 1)
        self.assertEqual(len(notices), 1)
        self.assertNotIn("/outside", json.dumps(report))

    def test_oversized_pax_rejected_before_payload_read(self):
        member = tarfile.TarInfo("pax")
        member.type = tarfile.XHDTYPE
        member.size = inventory.MAX_EXTENSION + 1
        with self.assertRaisesRegex(inventory.InventoryError, "extension"):
            inspect(member.tobuf())

    def test_pax_path_and_gnu_longname(self):
        path = "deno/" + "nested/" * 20 + "LICENSE-MIT"
        for form in (tarfile.PAX_FORMAT, tarfile.GNU_FORMAT):
            with self.subTest(form=form):
                report, _ = inspect(archive([(path, b"original synthetic notice")], form=form))
                self.assertIn(path, [item["path"] for item in report["original_notice_files"]])

    def test_sparse_and_unknown_extensions_fail_closed(self):
        member = tarfile.TarInfo("deno/sparse")
        member.type = tarfile.GNUTYPE_SPARSE
        with self.assertRaises(inventory.InventoryError):
            inspect(archive([(member, b"")], form=tarfile.GNU_FORMAT))
        for raw in (b"19 GNU.sparse.x=1\n", b"bad", b"11 path=x\n"):
            with self.subTest(raw=raw), self.assertRaises(inventory.InventoryError):
                inventory.pax_fields(raw)

    def test_metadata_aggregate_count_and_clock_bounds(self):
        for name, limit in (("MAX_COLLECTED", 1), ("MAX_MEMBERS", 2), ("MAX_EXPANDED", 1024)):
            with self.subTest(name=name), patch.object(inventory, name, limit), self.assertRaises(inventory.InventoryError):
                inspect(archive())
        with self.assertRaisesRegex(inventory.InventoryError, "time budget"):
            inventory.inspect_tar(io.BytesIO(archive()), time.monotonic() - 1)

    def test_root_identity_and_terminator_required(self):
        with self.assertRaisesRegex(inventory.InventoryError, "identity"):
            inspect(archive(cli_version='version = "0.0.0"'))
        with self.assertRaises(inventory.InventoryError):
            inspect(archive()[:-8192])
        with self.assertRaisesRegex(inventory.InventoryError, "after archive"):
            inspect(archive() + b"extra")

    def test_duplicate_lock_identity_and_invalid_records(self):
        candidate = {"name": "fixture", "version": "1.0.0"}
        for packages in ([candidate, candidate], [None], [{"name": "bad/path", "version": "1.0.0"}]):
            with self.subTest(packages=packages), self.assertRaises(inventory.InventoryError):
                inventory.lock_candidates({"package": packages})

    def test_symlink_source_input_not_followed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "real").write_bytes(gzip.compress(archive(), mtime=0))
            (root / "linked").symlink_to("real")
            with self.assertRaises(OSError):
                inventory.inventory(root / "linked")


if __name__ == "__main__":
    unittest.main()
