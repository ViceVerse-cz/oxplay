#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic package tampering fixtures; no subprocesses, network or native code."""
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import verify_package as verifier


class BundleFixture:
    def __init__(self, root):
        self.bundle = root / "Synthetic.app"
        self.inventory = root / "Synthetic.app.inventory.json"
        self.evidence = self.bundle / verifier.BUILD
        self.evidence.mkdir(parents=True)
        binary = self.bundle / "Contents/MacOS/serein"
        binary.parent.mkdir()
        binary.write_bytes(b"Synthetic fixture: never an executable")
        certificate = self.bundle / "Contents/Resources/Certificates/mozilla.pem"
        certificate.parent.mkdir()
        certificate.write_bytes(b"Synthetic trust fixture: not a certificate")
        source = {"Cargo.lock": b"synthetic lock", "crates/app/src/main.rs": b"// synthetic source"}
        self.source_path = self.bundle / verifier.SOURCE
        with tarfile.open(self.source_path, "w") as archive:
            for name, content in source.items():
                record = tarfile.TarInfo(name)
                record.size = len(content)
                archive.addfile(record, io.BytesIO(content))
        fingerprint = verifier.sha256(json.dumps([(name, verifier.sha256(content)) for name, content in source.items()],
                                                separators=(",", ":")).encode())
        self.manifest = {
            "schema": 1, "development_only": True, "portable": False, "target": "aarch64-apple-darwin",
            "source_fingerprint": fingerprint, "cargo_lock_sha256": verifier.sha256(source["Cargo.lock"]),
            "application_source_archive_sha256": verifier.sha256(self.source_path.read_bytes()),
            "media_certificate_resource": {"bundle_path": str(certificate.relative_to(self.bundle)),
                                           "sha256": verifier.sha256(certificate.read_bytes())},
            "evidence_input_files": [{"path": "application-source.tar", "sha256": verifier.sha256(self.source_path.read_bytes())}],
        }
        self.write_manifest()

    def write_manifest(self):
        payload = {key: value for key, value in self.manifest.items() if key != "input_fingerprint"}
        self.manifest["input_fingerprint"] = verifier.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode())
        (self.bundle / verifier.MANIFEST).write_text(json.dumps(self.manifest))
        self.reindex()

    def reindex(self):
        self.index = {"schema": 1, "development_only": True, "bundle": self.bundle.name,
                      "input_fingerprint": self.manifest["input_fingerprint"],
                      "files": [{"path": str(path.relative_to(self.bundle)), "bytes": path.stat().st_size,
                                 "sha256": verifier.sha256(path.read_bytes())}
                                for path in sorted(self.bundle.rglob("*")) if path.is_file()]}
        self.write_index()

    def write_index(self):
        self.inventory.write_text(json.dumps(self.index))

    def verify(self, expected=None):
        return verifier.verify(self.bundle, self.inventory, expected)


class VerificationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.fixture = BundleFixture(self.root)

    def test_first_party_dns_helper_binds_original_input_and_final_inventory(self):
        helper = self.fixture.bundle / "Contents/Helpers/serein-dns"
        helper.parent.mkdir()
        helper.write_bytes(b"Synthetic signed helper; never executed")
        original = "1" * 64
        source = "/synthetic/target/release/serein-dns"
        self.fixture.manifest["native_inputs"] = {source: {"sha256": original}}
        record = {"name": "serein-dns", "cargo_package": "serein-network",
                  "bundle_path": "Contents/Helpers/serein-dns", "license": "GPL-3.0-or-later",
                  "original_sha256": original, "source": source, "source_build_performed": True}
        self.fixture.manifest["first_party_helpers"] = [record]
        self.fixture.write_manifest()
        self.assertEqual(self.fixture.verify()["integrity"], "passed")
        for key, bad in (("bundle_path", "Contents/MacOS/serein"), ("source", "/unknown/input"),
                         ("original_sha256", "2" * 64), ("source_build_performed", "true")):
            with self.subTest(key=key):
                previous = record[key]
                record[key] = bad
                self.fixture.write_manifest()
                with self.assertRaises(verifier.VerificationError):
                    self.fixture.verify()
                record[key] = previous
        helper.unlink()
        self.fixture.write_manifest()
        with self.assertRaises(verifier.VerificationError):
            self.fixture.verify()

    def test_valid_bundle_is_read_only_and_move_safe(self):
        before = {path: (path.stat().st_mtime_ns, path.read_bytes()) for path in self.root.rglob("*") if path.is_file()}
        checksum = verifier.sha256(self.fixture.inventory.read_bytes())
        report = self.fixture.verify(checksum)
        self.assertEqual(report["integrity"], "passed")
        self.assertTrue(report["supplied_inventory_digest_matched"])
        self.assertFalse(report["publisher_authenticated"])
        self.assertFalse(report["code_signatures_checked"])
        after = {path: (path.stat().st_mtime_ns, path.read_bytes()) for path in self.root.rglob("*") if path.is_file()}
        self.assertEqual(before, after)
        moved = self.root / "Moved with spaces.app"
        self.fixture.bundle.rename(moved)
        self.assertEqual(verifier.verify(moved, self.fixture.inventory)["files"], report["files"])

    def test_same_length_tamper_missing_and_extra_files_fail(self):
        binary = self.fixture.bundle / "Contents/MacOS/serein"
        original = binary.read_bytes()
        binary.write_bytes(b"X" * len(original))
        with self.assertRaisesRegex(verifier.VerificationError, "hash"):
            self.fixture.verify()
        binary.unlink()
        with self.assertRaisesRegex(verifier.VerificationError, "tree"):
            self.fixture.verify()
        binary.write_bytes(original)
        (self.fixture.bundle / "extra").write_bytes(b"unlisted")
        with self.assertRaisesRegex(verifier.VerificationError, "tree"):
            self.fixture.verify()

    def test_inventory_digest_and_manifest_binding_fail(self):
        with self.assertRaisesRegex(verifier.VerificationError, "supplied"):
            self.fixture.verify("0" * 64)
        self.fixture.index["input_fingerprint"] = "0" * 64
        self.fixture.write_index()
        with self.assertRaisesRegex(verifier.VerificationError, "fingerprint"):
            self.fixture.verify()

    def test_unsafe_duplicate_and_case_ambiguous_inventory_paths_fail(self):
        for path in ("/absolute", "../escape", "Contents/../escape", "Contents//x", "Contents/./x", "Contents\\x", "C:/x"):
            with self.subTest(path=path):
                with self.assertRaises(verifier.VerificationError):
                    verifier.records([{"path": path, "sha256": "0" * 64}])
        for path in ("Contents/a", "contents/A"):
            self.fixture.index["files"].append({"path": path, "sha256": "0" * 64, "bytes": 0})
        self.fixture.write_index()
        with self.assertRaisesRegex(verifier.VerificationError, "ambiguous"):
            self.fixture.verify()

    def test_duplicate_json_keys_nonfinite_and_unsupported_schema_fail(self):
        for data in (b'{"schema":1,"schema":1}', b'{"number":NaN}', b'[]'):
            with self.subTest(data=data), self.assertRaises(verifier.VerificationError):
                verifier.strict_json(data)
        self.fixture.index["schema"] = True
        self.fixture.write_index()
        with self.assertRaisesRegex(verifier.VerificationError, "schema"):
            self.fixture.verify()

    def test_symlinks_internal_external_directory_and_broken_fail(self):
        binary = self.fixture.bundle / "Contents/MacOS/serein"
        external = self.root / "external"
        external.write_bytes(binary.read_bytes())
        for target in (external, "missing", "Contents/MacOS", "Contents/MacOS/serein"):
            link = self.fixture.bundle / "link"
            link.symlink_to(target)
            with self.subTest(target=target), self.assertRaisesRegex(verifier.VerificationError, "symlink"):
                self.fixture.verify()
            link.unlink()
        # Even replacing a recorded regular file with an identical external link fails.
        binary.unlink()
        binary.symlink_to(external)
        with self.assertRaises(verifier.VerificationError):
            self.fixture.verify()

    def test_fifo_hardlink_and_root_symlink_fail_without_reading_them(self):
        fifo = self.fixture.bundle / "fifo"
        os.mkfifo(fifo)
        with self.assertRaisesRegex(verifier.VerificationError, "Special"):
            self.fixture.verify()
        fifo.unlink()
        binary = self.fixture.bundle / "Contents/MacOS/serein"
        os.link(binary, self.root / "outside-hardlink")
        with self.assertRaisesRegex(verifier.VerificationError, "Hard-linked"):
            self.fixture.verify()
        link = self.root / "Linked.app"
        link.symlink_to(self.fixture.bundle)
        with self.assertRaises(OSError):
            verifier.verify(link, self.fixture.inventory)

    def test_reindexed_manifest_tamper_still_fails_its_internal_fingerprint(self):
        path = self.fixture.bundle / verifier.MANIFEST
        manifest = json.loads(path.read_text())
        manifest["source_fingerprint"] = "0" * 64
        path.write_text(json.dumps(manifest))
        self.fixture.reindex()
        with self.assertRaisesRegex(verifier.VerificationError, "input fingerprint"):
            self.fixture.verify()

    def test_reindexed_wrong_source_lock_or_certificate_fails(self):
        for key in ("source_fingerprint", "cargo_lock_sha256"):
            original = self.fixture.manifest[key]
            self.fixture.manifest[key] = "0" * 64
            self.fixture.write_manifest()
            with self.subTest(key=key), self.assertRaises(verifier.VerificationError):
                self.fixture.verify()
            self.fixture.manifest[key] = original
        self.fixture.manifest["media_certificate_resource"]["bundle_path"] = "../untrusted.pem"
        self.fixture.write_manifest()
        with self.assertRaisesRegex(verifier.VerificationError, "trust resource path"):
            self.fixture.verify()

    def test_source_archive_never_extracts_traversal_or_link_members(self):
        for name, kind in (("../escaped", tarfile.REGTYPE), ("safe-link", tarfile.SYMTYPE), ("hard-link", tarfile.LNKTYPE)):
            with tarfile.open(self.fixture.source_path, "w") as archive:
                item = tarfile.TarInfo(name)
                item.type = kind
                item.linkname = "../outside"
                archive.addfile(item)
            checksum = verifier.sha256(self.fixture.source_path.read_bytes())
            self.fixture.manifest["application_source_archive_sha256"] = checksum
            self.fixture.manifest["evidence_input_files"][0]["sha256"] = checksum
            self.fixture.write_manifest()
            with self.subTest(kind=kind), self.assertRaises(verifier.VerificationError):
                self.fixture.verify()
            self.assertFalse((self.root / "escaped").exists())

    def test_bounds_and_extra_evidence_reference_fail(self):
        self.fixture.index["files"][0]["bytes"] = verifier.MAX_FILE_BYTES + 1
        self.fixture.write_index()
        with self.assertRaisesRegex(verifier.VerificationError, "size"):
            self.fixture.verify()
        self.fixture.manifest["evidence_input_files"].append({"path": "missing", "sha256": "0" * 64})
        self.fixture.write_manifest()
        with self.assertRaisesRegex(verifier.VerificationError, "evidence"):
            self.fixture.verify()

    def test_helper_original_native_hash_is_distinct_from_final_resource_hash(self):
        rows = []
        for name, native in (("Contents/Helpers/yt-dlp", True), ("Contents/Helpers/deno", True),
                             ("Contents/Resources/HelperRuntime/module.py", False)):
            path = self.fixture.bundle / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"Synthetic helper fixture; never executed")
            rows.append({"target": name, "macho": native, "bytes": path.stat().st_size,
                         "sha256": "0" * 64 if native else verifier.sha256(path.read_bytes())})
        self.fixture.manifest["bundled_helper_resources"] = {"files": rows}
        self.fixture.write_manifest()
        self.assertEqual(self.fixture.verify()["integrity"], "passed")
        resource = self.fixture.bundle / rows[-1]["target"]
        resource.write_bytes(b"Changed and reindexed resource, but original helper record retained")
        self.fixture.reindex()
        with self.assertRaisesRegex(verifier.VerificationError, "Helper resource"):
            self.fixture.verify()

    def test_change_after_initial_hash_scan_is_rejected(self):
        original = verifier.Tree.data

        def mutate_after_hash(tree, path, limit):
            (self.fixture.bundle / "Contents/MacOS/serein").write_bytes(b"Changed during verification")
            return original(tree, path, limit)
        with patch.object(verifier.Tree, "data", mutate_after_hash):
            with self.assertRaisesRegex(verifier.VerificationError, "changed during"):
                self.fixture.verify()


if __name__ == "__main__":
    unittest.main()
