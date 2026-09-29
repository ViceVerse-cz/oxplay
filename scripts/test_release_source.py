#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic committed repositories only; no product builds or network."""
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

import release_source as release


class SourceReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        self.files = {
            "Cargo.toml": b'[workspace.package]\nversion="0.1.0"\n',
            "Cargo.lock": b"version = 4\n",
            "rust-toolchain.toml": b'[toolchain]\nchannel="1.98.1"\ncomponents=["rustfmt", "clippy"]\n',
            "LICENSE": b"Synthetic license fixture; no grant.\n",
            ".github/workflows/ci.yml": b"name: Synthetic CI\n",
            ".cargo/config.toml": b"[net]\noffline=true\n",
            ".gitignore": b"secrets/\n",
            "crates/app/src/main.rs": b"fn main() {}\n",
            "third_party/notices/SYNTHETIC.txt": b"Synthetic attribution fixture\n",
        }
        for name, data in self.files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        self.commit()

    def git(self, *arguments):
        return subprocess.check_output(["git", "-C", str(self.repo), *arguments], stderr=subprocess.PIPE)

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Synthetic fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "Synthetic source release")

    def make_release(self, name="output"):
        output = self.base / name
        return output, release.release(self.repo, "v0.1.0-dev.1", output)

    def test_only_exact_version_prerelease_tags_are_accepted(self):
        for channel in ("dev", "alpha", "beta", "rc"):
            release.validate_tag(f"v0.1.0-{channel}.1", "0.1.0")
        for tag in ("v0.1.0", "v0.1.1-dev.1", "v0.1.0-nightly.1", "v0.1.0-dev.01",
                    "v0.1.0-dev.-1", "v0.1.0-dev.1/extra", "v0.1.0-dev.1\n", "--output=x"):
            with self.subTest(tag=tag), self.assertRaises(release.ReleaseError):
                release.validate_tag(tag, "0.1.0")

    def test_exact_commit_includes_dotfiles_not_untracked_or_worktree_changes(self):
        revision = self.git("rev-parse", "HEAD").decode().strip()
        (self.repo / "secrets").mkdir()
        (self.repo / "secrets/session.txt").write_text("Synthetic private untracked token")
        (self.repo / "untracked.txt").write_text("Synthetic unrelated file")
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion="9.9.9"\n')
        (self.repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel="nightly"\n')
        output, metadata = self.make_release()
        self.assertEqual(metadata["revision"], revision)
        self.assertEqual(metadata["version"], "0.1.0")
        self.assertEqual(metadata["toolchain"]["channel"], "1.98.1")
        self.assertTrue(metadata["source_only"])
        self.assertFalse(metadata["production_qualified"])
        self.assertFalse(metadata["platform_qualified"])
        self.assertFalse(metadata["complete_corresponding_source"])
        self.assertFalse(metadata["contains_binaries"])
        prefix = "serein-v0.1.0-dev.1/"
        with tarfile.open(output / metadata["archive"], "r:gz") as archive:
            members = {member.name[len(prefix):]: member for member in archive if not member.isdir()}
            self.assertEqual(set(members), set(self.files))
            for name, expected in self.files.items():
                self.assertEqual(archive.extractfile(members[name]).read(), expected)
        self.assertEqual(json.loads((output / "release.json").read_text()), metadata)
        self.assertIn("not a production release", (output / "RELEASE_NOTES.md").read_text())

    def test_outputs_are_deterministic_and_checksums_cover_all_three_payloads(self):
        first, metadata = self.make_release("first")
        second, _ = self.make_release("second")
        self.assertEqual({p.name: p.read_bytes() for p in first.iterdir()},
                         {p.name: p.read_bytes() for p in second.iterdir()})
        raw = (first / metadata["archive"]).read_bytes()
        self.assertEqual(int.from_bytes(raw[4:8], "little"), 0)
        self.assertTrue(gzip.decompress(raw))
        rows = [line.split("  ") for line in (first / "SHA256SUMS").read_text().splitlines()]
        self.assertEqual({name for _, name in rows}, {metadata["archive"], "release.json", "RELEASE_NOTES.md"})
        for digest, name in rows:
            self.assertEqual(digest, hashlib.sha256((first / name).read_bytes()).hexdigest())

    def test_existing_output_is_untouched_and_invalid_tag_creates_nothing(self):
        output, _ = self.make_release()
        before = {p.name: p.read_bytes() for p in output.iterdir()}
        with self.assertRaises(FileExistsError):
            release.release(self.repo, "v0.1.0-dev.1", output)
        self.assertEqual(before, {p.name: p.read_bytes() for p in output.iterdir()})
        invalid = self.base / "invalid"
        with self.assertRaises(release.ReleaseError):
            release.release(self.repo, "v0.1.0", invalid)
        self.assertFalse(invalid.exists())

    def test_archive_cannot_omit_committed_license_via_export_ignore(self):
        (self.repo / ".gitattributes").write_text("LICENSE export-ignore\n")
        self.commit()
        with self.assertRaisesRegex(release.ReleaseError, "omitted committed"):
            self.make_release()

    def test_archive_substitutions_cannot_change_committed_bytes(self):
        (self.repo / ".gitattributes").write_text("version.txt export-subst\n")
        (self.repo / "version.txt").write_text("$Format:%H$\n")
        self.commit()
        with self.assertRaisesRegex(release.ReleaseError, "differs from a committed"):
            self.make_release()

    def test_workflow_outputs_are_fixed_single_line_fields(self):
        _, metadata = self.make_release()
        path = self.base / "github-output"
        path.write_text("previous=value\n")
        release.github_output(path, metadata)
        self.assertEqual(path.read_text().splitlines(), ["previous=value"] + [
            f"{name}={metadata[name]}" for name in ("revision", "tag", "archive")])


if __name__ == "__main__":
    unittest.main()
