#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic-only tests; never use an account, network or the user's Cargo cache."""
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock
import zlib

import git_source_export as export
import mpv_timer_diagnostic as diagnostic
from source_coverage import CoverageError


class Objects:
    def __init__(self, root):
        self.cargo = root / "cargo"
        self.db = self.cargo / "git/db/fixture"
        self.objects = self.db / "objects"
        (self.objects / "info").mkdir(parents=True)
        (self.objects / "pack").mkdir()

    def put(self, kind, raw):
        raw_object = f"{kind} {len(raw)}\0".encode() + raw
        oid = hashlib.sha1(raw_object).hexdigest()
        path = self.objects / oid[:2] / oid[2:]
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(zlib.compress(raw_object))
        return oid

    def tree(self, entries):
        raw = b"".join(mode.encode() + b" " + name.encode() + b"\0" + bytes.fromhex(oid)
                       for name, mode, oid in sorted(entries))
        return self.put("tree", raw)

    def commit(self, tree):
        raw = (f"tree {tree}\nauthor Synthetic <test@invalid> 0 +0000\n"
               "committer Synthetic <test@invalid> 0 +0000\n\nSynthetic source fixture\n")
        return self.put("commit", raw.encode())

    def basic(self):
        content = self.put("blob", b"immutable source\n")
        revision = self.commit(self.tree([("source.txt", "100644", content)]))
        return revision, content


def packages(revision):
    return [{"kind": "git", "revision": revision, "name": name, "version": "1.0.0"}
            for name in ("fixture-a", "fixture-b")]


class PureTests(unittest.TestCase):
    def test_zombie_group_permission_requires_reap_and_positive_absence(self):
        for module in (export, diagnostic):
            with self.subTest(module=module.__name__):
                process = mock.Mock(pid=123)
                with mock.patch.object(module.os, "killpg", side_effect=[PermissionError(), ProcessLookupError()]) as kill:
                    module.reap_group(process)
                process.wait.assert_called_once()
                self.assertEqual([call.args[1] for call in kill.call_args_list], [module.signal.SIGKILL, 0])

    def test_permission_does_not_turn_a_surviving_group_into_success(self):
        for module, error in ((export, CoverageError), (diagnostic, diagnostic.DiagnosticError)):
            with self.subTest(module=module.__name__):
                process = mock.Mock(pid=123)
                with mock.patch.object(module.os, "killpg", side_effect=PermissionError()), \
                     mock.patch.object(module.time, "monotonic", side_effect=[0, 3]), \
                     self.assertRaises(error):
                    module.reap_group(process)
                process.wait.assert_called_once()

    def test_paths_reject_traversal_metadata_and_controls(self):
        for path in (b"../file", b"/file", b"a//b", b"a/.git/config", b"a\\b", b"a\nb",
                     b"C:/outside", b"C:relative", b"file:stream"):
            with self.subTest(path=path), self.assertRaises(CoverageError):
                export.safe_path(path)
        self.assertEqual(export.safe_path(b"src/main.rs"), "src/main.rs")

    def test_links_reject_direct_and_composed_escape(self):
        for target in (b"../../outside", b"/outside", b"../.git/config", b"C:/outside", b"C:relative"):
            with self.subTest(target=target), self.assertRaises(CoverageError):
                export.safe_link("directory/link", target)
        # Both links are lexically contained; expansion through root alias is not.
        links = {"alias": ".", "directory/link": "../alias/../outside"}
        with self.assertRaises(CoverageError):
            export.verify_link_chains(links)
        with self.assertRaises(CoverageError):
            export.verify_link_chains({"a": "b", "b": "a"})
        export.verify_link_chains({"directory/link": "../source.txt"})

    def test_case_and_normalization_aliases_cannot_bypass_link_containment(self):
        for alias, spelling in (("Alias", "alias"), ("\u00e9", "e\u0301")):
            with self.subTest(alias=alias), self.assertRaises(CoverageError):
                export.verify_link_chains({alias: ".", "directory/link": f"../{spelling}/../outside"})
        with self.assertRaises(CoverageError):
            export.reject_member_aliases(["Directory/file", "directory/file"])
        with self.assertRaises(CoverageError):
            export.reject_member_aliases(["\u00e9/file", "e\u0301/file"])

    def test_listing_rejects_duplicate_or_special(self):
        oid = "1" * 40
        row = f"100644 blob {oid}\tfile\0".encode()
        with self.assertRaises(CoverageError):
            export.parse_listing(row + row)
        with self.assertRaises(CoverageError):
            export.parse_listing(f"010644 blob {oid}\tfile\0".encode())

    def test_tar_content_tamper_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "changed.tar"
            expected = {"file": ("100644", "blob", export.object_hash("blob", b"expected"))}
            with tarfile.open(path, "w") as archive:
                member = tarfile.TarInfo("file")
                member.mode = 0o644
                member.size = 7
                archive.addfile(member, io.BytesIO(b"changed"))
            with self.assertRaises(CoverageError):
                export.verify_archive(path, expected)

    def test_tar_missing_member_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "empty.tar"
            with tarfile.open(path, "w"):
                pass
            with self.assertRaises(CoverageError):
                export.verify_archive(path, {"file": ("100644", "blob", "1" * 40)})


@unittest.skipUnless(Path("/usr/bin/git").is_file() and hasattr(os, "WNOWAIT"),
                     "Requires supported local Git supervision")
class ObjectExportTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.fixture = Objects(self.root)

    def run_export(self, revision, name="export"):
        return export.export_git_sources(packages(revision), self.fixture.cargo, self.root / name)

    def test_exact_objects_not_modified_checkout_and_deduplicated(self):
        revision, _ = self.fixture.basic()
        checkout = self.fixture.cargo / "git/checkouts/fixture/modified"
        checkout.mkdir(parents=True)
        (checkout / "source.txt").write_text("editable wrong source")
        before = {str(p.relative_to(self.fixture.cargo)): p.read_bytes()
                  for p in self.fixture.cargo.rglob("*") if p.is_file()}
        report = self.run_export(revision)
        self.assertEqual(len(report["exports"]), 1)
        row = report["exports"][0]
        self.assertEqual(len(row["packages"]), 2)
        self.assertEqual(row["status"], "verified_locked_git_tree")
        with tarfile.open(self.root / "export" / row["archive"]) as archive:
            self.assertEqual(archive.extractfile("source.txt").read(), b"immutable source\n")
        after = {str(p.relative_to(self.fixture.cargo)): p.read_bytes()
                 for p in self.fixture.cargo.rglob("*") if p.is_file()}
        self.assertEqual(before, after)
        manifest = (self.root / "export/git-source-manifest.json").read_text()
        self.assertNotIn(str(self.root), manifest)
        self.assertFalse(json.loads(manifest)["complete_corresponding_source"])
        self.assertFalse((self.root / "export/INCOMPLETE").exists())

    def test_attributes_do_not_transform_and_modes_symlinks_preserved(self):
        f = self.fixture
        source = f.put("blob", b"$Format:%H$\n")
        attributes = f.put("blob", b"source export-ignore export-subst\n")
        link = f.put("blob", b"source")
        revision = f.commit(f.tree([(".gitattributes", "100644", attributes),
                                    ("link", "120000", link), ("source", "100755", source)]))
        row = self.run_export(revision)["exports"][0]
        with tarfile.open(self.root / "export" / row["archive"]) as archive:
            self.assertEqual(archive.extractfile("source").read(), b"$Format:%H$\n")
            self.assertEqual(archive.getmember("source").mode, 0o755)
            self.assertEqual(archive.getmember("link").linkname, "source")
        row2 = self.run_export(revision, "second")["exports"][0]
        self.assertEqual(row["sha256"], row2["sha256"])

    def test_gitlink_is_explicitly_uncovered(self):
        f = self.fixture
        revision = f.commit(f.tree([("submodule", "160000", "a" * 40)]))
        row = self.run_export(revision)["exports"][0]
        self.assertEqual(row["gitlinks"], [{"path": "submodule", "revision": "a" * 40,
                                         "status": "gitlink_source_not_exported"}])

    def test_missing_revision_is_unavailable_not_success_tree(self):
        report = self.run_export("a" * 40)
        self.assertEqual(report["exports"][0]["status"], "local_revision_unavailable")
        self.assertNotIn("archive", report["exports"][0])

    def test_existing_destination_is_never_reused(self):
        revision, _ = self.fixture.basic()
        (self.root / "export").mkdir()
        with self.assertRaises(FileExistsError):
            self.run_export(revision)

    def test_alternates_are_not_followed(self):
        revision, _ = self.fixture.basic()
        (self.fixture.objects / "info/alternates").write_text("/unreviewed/object/database\n")
        with self.assertRaises(CoverageError):
            self.run_export(revision)
        self.assertTrue((self.root / "export/INCOMPLETE").exists())

    def test_aggregate_byte_budget_fails_closed(self):
        revision, _ = self.fixture.basic()
        with mock.patch.object(export, "MAX_TOTAL", 4), self.assertRaises(CoverageError):
            self.run_export(revision)
        self.assertFalse((self.root / "export/git-source-manifest.json").exists())

    def test_corrupt_blob_object_never_gets_success_manifest(self):
        revision, blob = self.fixture.basic()
        # A plausible loose-object file under the selected identity is insufficient.
        (self.fixture.objects / blob[:2] / blob[2:]).write_bytes(zlib.compress(b"blob 8\0tampered"))
        with self.assertRaises(CoverageError):
            self.run_export(revision)
        self.assertTrue((self.root / "export/INCOMPLETE").exists())

    def test_enclosing_repository_config_and_replacements_are_ignored(self):
        revision, _ = self.fixture.basic()
        outer = self.root / ".git"
        outer.mkdir()
        (outer / "config").write_text("[alias]\ncat-file = !exit 99\n")
        (self.fixture.db / "config").write_text("[core]\nrepositoryformatversion = 999\n")
        replacement = self.fixture.db / "refs/replace"
        replacement.mkdir(parents=True)
        (replacement / revision).write_text("0" * 40 + "\n")
        self.assertEqual(self.run_export(revision)["exports"][0]["revision"], revision)

    def test_escaping_symlink_never_gets_success_manifest(self):
        f = self.fixture
        revision = f.commit(f.tree([("link", "120000", f.put("blob", b"../outside"))]))
        with self.assertRaises(CoverageError):
            self.run_export(revision)
        self.assertTrue((self.root / "export/INCOMPLETE").exists())

    def test_lfs_pointers_are_not_claimed_materialized(self):
        f = self.fixture
        pointer = f.put("blob", b"version https://git-lfs.github.com/spec/v1\noid sha256:" + b"1" * 64 + b"\nsize 10\n")
        revision = f.commit(f.tree([("asset", "100644", pointer)]))
        row = self.run_export(revision)["exports"][0]
        self.assertEqual(row["git_lfs_pointer_paths_not_materialized"], ["asset"])


if __name__ == "__main__":
    unittest.main()
