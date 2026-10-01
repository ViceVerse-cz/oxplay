#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic, offline source-coverage boundary tests; no installed cache access."""
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import source_coverage as coverage


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


class CoverageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.evidence = self.root / "BuildInfo"
        self.evidence.mkdir()
        self.cargo = self.root / "cargo"
        self.cargo.mkdir()
        self.brew = self.root / "brew"
        self.brew.mkdir()

    def fixture(self, *, registry=True):
        package = {"name": "synthetic", "version": "1.0.0", "source": None}
        lock = 'version = 4\n[[package]]\nname = "synthetic"\nversion = "1.0.0"\n'
        if registry:
            package["source"] = "registry+https://github.com/rust-lang/crates.io-index"
            lock += 'source = "' + package["source"] + '"\nchecksum = "' + digest(b"synthetic source archive") + '"\n'
        graph = json.dumps({"packages": [package]}).encode()
        (self.evidence / "cargo-graph.json").write_bytes(graph)
        archive = self.evidence / "application-source.tar"
        with tarfile.open(archive, "w") as tar:
            item = tarfile.TarInfo("Cargo.lock")
            item.size = len(lock.encode())
            tar.addfile(item, io.BytesIO(lock.encode()))
        manifest = {"application_source_archive_sha256": digest(archive.read_bytes()),
                    "cargo_lock_sha256": digest(lock.encode()),
                    "evidence_input_files": [{"path": "cargo-graph.json", "sha256": digest(graph)}],
                    "homebrew_provenance": []}
        (self.evidence / "build-manifest.json").write_text(json.dumps(manifest))
        return package, manifest

    def run_audit(self):
        return coverage.audit(self.evidence, self.cargo, self.brew, 1024 * 1024)

    def test_verified_registry_archive_is_not_complete_source_claim(self):
        self.fixture()
        cache = self.cargo / "registry/cache/test-registry"
        cache.mkdir(parents=True)
        (cache / "synthetic-1.0.0.crate").write_bytes(b"synthetic source archive")
        report = self.run_audit()
        self.assertEqual(report["cargo"][0]["status"], "verified_archive_bytes")
        self.assertFalse(report["complete_corresponding_source"])
        self.assertFalse(report["publisher_authenticated"])
        self.assertNotIn(str(self.root), json.dumps(report))
        self.assertEqual(report, self.run_audit())

    def test_default_audit_never_spawns_a_subprocess(self):
        self.fixture()
        with patch("subprocess.Popen", side_effect=AssertionError("Unexpected external command")):
            report = self.run_audit()
        self.assertNotIn("git_source_export", report)

    def test_absent_archive_does_not_promote_extracted_tree(self):
        self.fixture()
        (self.cargo / "registry/src/test-registry/synthetic-1.0.0").mkdir(parents=True)
        result = self.run_audit()["cargo"][0]
        self.assertEqual(result["status"], "absent")
        self.assertTrue(result["extracted_tree_present_unverified"])

    def test_mismatched_archive_and_budget_are_explicit(self):
        file = self.root / "source"
        file.write_bytes(b"wrong")
        self.assertEqual(coverage.verify_candidates([file], digest(b"correct"), coverage.Budget())["status"], "hash_mismatch")
        self.assertEqual(coverage.verify_candidates([file], digest(b"wrong"), coverage.Budget(1))["status"], "hash_budget_or_file_limit")

    def test_packaged_graph_and_lock_are_bound_to_manifest(self):
        self.fixture()
        (self.evidence / "cargo-graph.json").write_text('{"packages": []}')
        with self.assertRaisesRegex(coverage.CoverageError, "graph hash mismatch"):
            self.run_audit()
        _, manifest = self.fixture()
        manifest["cargo_lock_sha256"] = "0" * 64
        (self.evidence / "build-manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(coverage.CoverageError, "lockfile hash mismatch"):
            self.run_audit()

    def test_archive_traversal_link_and_duplicate_lock_rejected(self):
        for kind in ("traversal", "link", "duplicate"):
            with self.subTest(kind=kind):
                _, manifest = self.fixture()
                archive = self.evidence / "application-source.tar"
                with tarfile.open(archive, "a") as tar:
                    item = tarfile.TarInfo("../escape" if kind == "traversal" else "Cargo.lock" if kind == "duplicate" else "linked")
                    if kind == "link":
                        item.type = tarfile.SYMTYPE
                        item.linkname = "/etc/passwd"
                    tar.addfile(item)
                manifest["application_source_archive_sha256"] = digest(archive.read_bytes())
                (self.evidence / "build-manifest.json").write_text(json.dumps(manifest))
                with self.assertRaises(coverage.CoverageError):
                    self.run_audit()

    def test_symlinked_cache_and_special_file_not_followed(self):
        external = self.root / "external"
        external.write_bytes(b"not source")
        link = self.root / "linked"
        link.symlink_to(external)
        with self.assertRaises(coverage.CoverageError):
            coverage.child(self.root, "linked")
        self.assertNotIn(link, coverage.entries(self.root, directories=False))
        import os
        fifo = self.root / "fifo"
        os.mkfifo(fifo)
        with self.assertRaises(coverage.CoverageError):
            coverage.read(fifo)

    def test_huge_pax_record_rejected_before_payload_read(self):
        item = tarfile.TarInfo("metadata")
        item.type = tarfile.XHDTYPE
        item.size = coverage.MAX_FILE + 1
        archive = self.root / "oversized.tar"
        archive.write_bytes(item.tobuf() + bytes(1024))
        with self.assertRaisesRegex(coverage.CoverageError, "Oversized"):
            coverage.lock_from_archive(archive, digest(archive.read_bytes()), "0" * 64, coverage.Budget())

    def test_git_head_match_explicitly_leaves_tree_unverified(self):
        revision = "a" * 40
        git = self.cargo / "git/checkouts/repository/checkout/.git"
        (git / "refs/heads").mkdir(parents=True)
        (git / "HEAD").write_text("ref: refs/heads/master\n")
        (git / "refs/heads/master").write_text(revision + "\n")
        source = "git+https://example.invalid/synthetic#" + revision
        package = {"name": "synthetic", "version": "1", "source": source}
        result = coverage.cargo_coverage([package], {"package": [package]}, self.cargo, coverage.Budget())
        self.assertEqual(result[0]["status"], "checkout_head_matches_unverified_worktree")
        self.assertIn("submodules", result[0]["limitation"])
        (git / "HEAD").write_text("ref: refs/../../outside\n")
        self.assertIsNone(coverage.git_head(git.parent))

    def test_literal_formula_source_and_two_mpv_patches(self):
        self.assertEqual(coverage.label("python@3.14"), "python@3.14")
        with self.assertRaises(coverage.CoverageError):
            coverage.label("../../private")
        body = 'url "https://example.invalid/source.tar.gz"\nsha256 "' + digest(b"source") + '"\n'
        for revision, checksum in coverage.MPV_PATCHES.items():
            body += 'patch do\n url "https://github.com/mpv-player/mpv/commit/' + revision + '.patch?full_index=1"\n sha256 "' + checksum + '"\nend\n'
        result = coverage.formula_inputs(body.encode())
        self.assertEqual(len(result), 3)
        self.assertEqual({item["mpv_patch_revision"] for item in result[1:]}, set(coverage.MPV_PATCHES))
        self.assertEqual(result[0]["kind"], "source_or_resource")

    def native_media_fixture(self):
        _, manifest = self.fixture()
        revision = "a" * 40
        original = b"Synthetic original source archive; no executable code\n"
        pins = {"synthetic": {"repository": "example/fixture", "revision": revision,
                              "url": "https://example.invalid/source.tar.gz", "sha256": digest(original),
                              "bytes": len(original), "license": "Synthetic fixture", "destination": "synthetic"}}
        patch = b"Synthetic reviewed native patch\n"
        build = {"schema": 1, "status": "compiled_and_installed", "native_render_abi": 1,
                 "sources": pins, "patches": {"synthetic.patch": digest(patch)}}
        files = {"build-result.json": json.dumps(build).encode(),
                 "macos-sources.json": json.dumps({"schema": 1, "sources": pins}).encode(),
                 f"sources/synthetic-{revision}.tar.gz": original,
                 "patches/synthetic.patch": patch, "licenses/synthetic/LICENSE": b"Synthetic license fixture\n"}
        media = {"prefix": "/private/not-an-audit-input", "root": "/private/not-an-audit-input",
                 "backend": "metal", "native_render_abi": 1, "evidence_bundle_path": "native-media", "evidence": []}
        for target, raw in files.items():
            path = self.evidence / "native-media" / target
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
            media["evidence"].append({"target": target, "sha256": digest(raw), "source": "/private/original-input"})
            manifest["evidence_input_files"].append({"path": "native-media/" + target, "sha256": digest(raw)})
        manifest["native_media"] = media
        (self.evidence / "build-manifest.json").write_text(json.dumps(manifest))
        return manifest, files

    def test_retained_native_sources_patches_and_licenses_have_separate_coverage(self):
        self.native_media_fixture()
        report = self.run_audit()
        native = report["native_media_inputs"]
        self.assertEqual(native["backend"], "metal")
        self.assertEqual(native["sources"][0]["archive"]["status"], "verified_archive_bytes")
        self.assertEqual(native["patches"][0]["status"], "verified_file_bytes")
        self.assertEqual(native["sources"][0]["license_evidence"][0]["status"], "verified_file_bytes")
        self.assertFalse(report["complete_corresponding_source"])
        self.assertFalse(report["publisher_authenticated"])
        self.assertNotIn("/private/", json.dumps(report))
        self.assertNotIn("example.invalid", json.dumps(native))
        self.assertEqual(report, self.run_audit())

    def test_native_archive_missing_tampered_or_budget_limited_never_verifies(self):
        manifest, files = self.native_media_fixture()
        relative = next(name for name in files if name.startswith("sources/"))
        archive = self.evidence / "native-media" / relative
        archive.unlink()
        report = coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())
        self.assertEqual(report["sources"][0]["archive"]["status"], "absent")
        archive.write_bytes(b"Synthetic changed source bytes")
        report = coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())
        self.assertEqual(report["sources"][0]["archive"]["status"], "hash_mismatch")
        archive.write_bytes(files[relative])
        report = coverage.native_media_coverage(manifest, self.evidence, coverage.Budget(1))
        self.assertEqual(report["sources"][0]["archive"]["status"], "hash_budget_or_file_limit")

    def test_native_metadata_and_inventory_must_agree_before_archive_verification(self):
        manifest, _ = self.native_media_fixture()
        path = self.evidence / "native-media/build-result.json"
        path.write_bytes(path.read_bytes() + b"\n")
        with self.assertRaisesRegex(coverage.CoverageError, "metadata hash mismatch"):
            coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())
        manifest, _ = self.native_media_fixture()
        manifest["native_media"]["evidence"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(coverage.CoverageError, "bound to package inventory"):
            coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())

    def test_native_build_pin_disagreement_and_unsafe_targets_are_rejected(self):
        manifest, _ = self.native_media_fixture()
        path = self.evidence / "native-media/build-result.json"
        build = json.loads(path.read_bytes())
        build["sources"]["synthetic"]["revision"] = "b" * 40
        raw = json.dumps(build).encode()
        path.write_bytes(raw)
        for item in manifest["native_media"]["evidence"]:
            if item["target"] == "build-result.json":
                item["sha256"] = digest(raw)
        for item in manifest["evidence_input_files"]:
            if item["path"] == "native-media/build-result.json":
                item["sha256"] = digest(raw)
        with self.assertRaisesRegex(coverage.CoverageError, "configuration disagree"):
            coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())
        for target in ("../outside", "/outside", "sources/../outside"):
            manifest, _ = self.native_media_fixture()
            manifest["native_media"]["evidence"][0]["target"] = target
            with self.subTest(target=target), self.assertRaises(coverage.CoverageError):
                coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())
        manifest, _ = self.native_media_fixture()
        archive = next((self.evidence / "native-media/sources").iterdir())
        archive.unlink()
        archive.symlink_to(self.evidence / "cargo-graph.json")
        with self.assertRaises(coverage.CoverageError):
            coverage.native_media_coverage(manifest, self.evidence, coverage.Budget())

    def test_bottles_do_not_count_and_native_formula_tampering_rejected(self):
        raw = b'source bytes'
        url = 'https://example.invalid/source.tar.gz'
        formula = ('url "' + url + '"\nsha256 "' + digest(raw) + '"\n').encode()
        directory = self.evidence / "homebrew/synthetic/1"
        directory.mkdir(parents=True)
        file = directory / "synthetic.rb"
        file.write_bytes(formula)
        manifest = {"homebrew_provenance": [{"formula": "synthetic", "installed_version": "1", "metadata": {"synthetic.rb": {"sha256": digest(formula)}}}]}
        (self.brew / (digest(url.encode()) + "--synthetic.bottle.tar.gz")).write_bytes(raw)
        result = coverage.native_coverage(manifest, self.evidence, self.brew, coverage.Budget())
        self.assertEqual(result[0]["inputs"][0]["status"], "absent")
        (self.brew / (digest(url.encode()) + "--source.tar.gz")).write_bytes(raw)
        result = coverage.native_coverage(manifest, self.evidence, self.brew, coverage.Budget())
        self.assertEqual(result[0]["inputs"][0]["status"], "verified_archive_bytes")
        file.write_bytes(formula + b"# altered")
        with self.assertRaises(coverage.CoverageError):
            coverage.native_coverage(manifest, self.evidence, self.brew, coverage.Budget())

    def test_computed_formula_url_and_duplicate_json_not_accepted_as_coverage(self):
        result = coverage.formula_inputs(('url "https://example.invalid/#{version}.tar.gz"\nsha256 "' + "a" * 64 + '"\n').encode())
        self.assertEqual(result[0]["status"], "computed_url_unsupported")
        for raw in (b'{"a": 1, "a": 2}', b'{"a": NaN}'):
            with self.assertRaises(coverage.CoverageError):
                coverage.parse_json(raw)

    def test_cli_refuses_overwrite_without_disclosing_local_path(self):
        output = self.root / "private-local-label.json"
        output.write_text("preserve this")
        message = io.StringIO()
        argv = ["source_coverage.py", "--evidence", str(self.evidence), "--output", str(output)]
        with patch("sys.argv", argv), patch.object(coverage, "audit", return_value={"safe": True}), redirect_stdout(message):
            self.assertEqual(coverage.main(), 2)
        self.assertEqual(output.read_text(), "preserve this")
        self.assertNotIn(str(self.root), message.getvalue())
        self.assertNotIn(output.name, message.getvalue())


if __name__ == "__main__":
    unittest.main()
