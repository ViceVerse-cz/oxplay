#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline release-planner regressions: synthetic repositories only, no GitHub."""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from unittest import mock

import release

ROOT = Path(__file__).resolve().parents[1]
TODAY = dt.date(2026, 10, 1)


class Repository:
    def __init__(self, base: Path):
        self.path = base / "repo"
        self.path.mkdir()
        self.git("init", "-q", "-b", "main")
        (self.path / "Cargo.toml").write_text('[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\n')
        self.commit("Initial import", name="Founder", email="founder@example.invalid")

    def git(self, *arguments):
        return subprocess.check_output(["git", "-C", str(self.path), *arguments], text=True, stderr=subprocess.PIPE)

    def commit(self, message, name="Dev", email="dev@example.invalid"):
        marker = self.path / "history.txt"
        marker.write_text(marker.read_text() + message + "\n" if marker.exists() else message + "\n")
        self.git("add", ".")
        self.git("-c", f"user.name={name}", "-c", f"user.email={email}", "-c", "commit.gpgsign=false",
                 "commit", "-qm", message)
        return self.git("rev-parse", "HEAD").strip()

    def plan(self, channel="production", **overrides):
        options = dict(dry_run=False, ref_name="main", run_number="7", repository="Example/oxplay", today=TODAY)
        options.update(overrides)
        return release.plan(channel, cwd=self.path, **options)


class CommitAnalysis(unittest.TestCase):
    def analysed(self, subject, body=""):
        return release.parse_commit(release.SEPARATOR.join(["a" * 40, "N", "n@example.invalid", subject, body]))

    def test_semantic_bumps_and_plain_subject_adaptation(self):
        for subject, body, expected in [
            ("fix(ui): fix scrolling", "", "patch"), ("feat(ui): add search", "", "minor"),
            ("feat!: change storage format", "", "major"), ("perf: faster decode", "", "patch"),
            ("fix: x", "BREAKING CHANGE: schema v7", "major"), ("docs: update guide", "", None),
            ("chore(deps): bump", "", None), ("ci: cache", "", None),
            ("Add ambient mode behind the video", "", "patch"),
        ]:
            with self.subTest(subject=subject):
                self.assertEqual(release.bump(self.analysed(subject, body)), expected)
        self.assertEqual(release.release_type([self.analysed("docs: a"), self.analysed("feat: b"),
                                               self.analysed("fix: c")]), "minor")
        self.assertIsNone(release.release_type([self.analysed("docs: a")]))

    def test_pull_request_suffix_and_scope_are_separated(self):
        commit = self.analysed("feat(player): add tabs (#149)")
        self.assertEqual((commit["type"], commit["scope"], commit["title"], commit["pull_request"]),
                         ("feat", "player", "add tabs", "149"))

    def test_increment(self):
        self.assertEqual(release.increment("0.1.0", "patch"), "0.1.1")
        self.assertEqual(release.increment("0.1.9", "minor"), "0.2.0")
        self.assertEqual(release.increment("0.9.3", "major"), "1.0.0")


class Planning(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Repository(Path(temporary.name))

    def test_first_release_uses_the_committed_workspace_version(self):
        self.repo.git("tag", "v0.1.0-dev.1")  # prerelease tags never count as releases
        plan = self.repo.plan("production")
        self.assertEqual((plan["version"], plan["tag"], plan["last_stable_tag"]), ("0.1.0", "v0.1.0", None))
        nightly = self.repo.plan("nightly")
        self.assertEqual(nightly["version"], "0.1.0-nightly.20261001.7")
        self.assertEqual(nightly["stable_version"], "0.1.0")

    def test_nightly_versions_follow_the_next_stable_version(self):
        self.repo.git("tag", "v0.1.0")
        self.repo.commit("feat(ui): add search")
        self.repo.git("tag", "v0.2.0-nightly.20260930.6")
        self.repo.commit("fix(ui): fix scrolling")
        plan = self.repo.plan("nightly", run_number="12")
        self.assertEqual(plan["version"], "0.2.0-nightly.20261001.12")
        self.assertEqual(plan["last_stable_tag"], "v0.1.0")
        # Nightly isolation: notes only cover changes since the previous nightly.
        self.assertIn("fix scrolling", plan["notes"])
        self.assertNotIn("add search", plan["notes"])
        production = self.repo.plan("production")
        self.assertEqual(production["version"], "0.2.0")
        self.assertIn("add search", production["notes"])
        self.assertIn("fix scrolling", production["notes"])
        self.assertIn("compare/v0.1.0...v0.2.0", production["notes"])

    def test_no_release_without_releasable_commits(self):
        self.repo.git("tag", "v0.1.0")
        self.assertIsNone(self.repo.plan("production"))
        self.repo.commit("docs: explain packaging")
        self.assertIsNone(self.repo.plan("nightly"))
        self.repo.commit("chore(release): 0.1.1 [skip ci]")
        self.assertIsNone(self.repo.plan("production"))

    def test_publishing_requires_main_but_dry_runs_do_not(self):
        with self.assertRaisesRegex(release.ReleaseError, "main"):
            self.repo.plan("production", ref_name="release-pipeline")
        plan = self.repo.plan("production", ref_name="release-pipeline", dry_run=True)
        self.assertTrue(plan["dry_run"])

    def test_existing_tag_is_never_reused(self):
        self.repo.git("tag", "v0.1.0-nightly.20261001.7")
        with self.assertRaisesRegex(release.ReleaseError, "already exists"):
            self.repo.plan("nightly")

    def test_notes_attribute_authors_link_pull_requests_and_list_new_contributors(self):
        self.repo.git("tag", "v0.1.0")
        self.repo.commit("feat(ui): add category settings (#149)", name="Release Note Author",
                         email="12345+noteauthor@users.noreply.github.com")
        self.repo.commit("fix(audio): bundle sounds (#140)", name="Founder", email="founder@example.invalid")
        self.repo.commit("Tidy the watch page layout")
        self.repo.commit("feat: bot change (#150)", name="renovate[bot]", email="bot@example.invalid")
        notes = self.repo.plan("production")["notes"]
        self.assertIn("### Features", notes)
        self.assertIn("### Bug Fixes", notes)
        self.assertIn("### Changes", notes)
        self.assertIn("**ui:** add category settings by [@noteauthor](https://github.com/noteauthor) "
                      "in [#149](https://github.com/Example/oxplay/pull/149)", notes)
        self.assertIn("Tidy the watch page layout by Dev", notes)
        self.assertIn("## New Contributors", notes)
        self.assertIn("[@noteauthor](https://github.com/noteauthor) made their first contribution in [#149]", notes)
        self.assertNotIn("Founder made their first", notes)
        self.assertNotIn("renovate[bot] made", notes)
        self.assertGreater(notes.index("## New Contributors"), notes.index("bundle sounds"))

    def test_breaking_changes_are_listed_first(self):
        self.repo.git("tag", "v0.1.0")
        self.repo.commit("feat!: change the storage schema")
        plan = self.repo.plan("production")
        self.assertEqual(plan["version"], "1.0.0")
        self.assertLess(plan["notes"].index("BREAKING CHANGES"), plan["notes"].index("### Features"))

    def test_workflow_outputs_must_be_single_line(self):
        with tempfile.NamedTemporaryFile("w+", delete=False) as output:
            path = output.name
        self.addCleanup(os.unlink, path)
        release.write_outputs(path, {"version": "0.1.0", "tag": "v0.1.0"})
        self.assertEqual(Path(path).read_text(), "version=0.1.0\ntag=v0.1.0\n")
        with self.assertRaises(release.ReleaseError):
            release.write_outputs(path, {"notes": "a\nb=c"})


class Manifests(unittest.TestCase):
    def test_versions_update_workspace_members_and_lock_only(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory)
            paths = ["Cargo.toml", "Cargo.lock"]
            workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
            paths += [f"{member}/Cargo.toml" for member in workspace["members"]]
            for path in paths:
                (fixture / path).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, fixture / path)
            original = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
            inherited = {tomllib.loads((ROOT / f"{m}/Cargo.toml").read_text())["package"]["name"]
                         for m in workspace["members"]
                         if tomllib.loads((ROOT / f"{m}/Cargo.toml").read_text())["package"].get("version") == {"workspace": True}}
            self.assertIn("oxplay", inherited)
            for version in ("1.2.3-nightly.20261001.9", "1.2.3"):
                release.apply_version(version, fixture)
                self.assertEqual(tomllib.loads((fixture / "Cargo.toml").read_text())["workspace"]["package"]["version"], version)
                packages = tomllib.loads((fixture / "Cargo.lock").read_text())["package"]
                self.assertEqual([p for p in packages if "source" in p], [p for p in original if "source" in p])
                for package, before in zip(packages, original):
                    expected = version if package["name"] in inherited and "source" not in package else before["version"]
                    self.assertEqual(package["version"], expected, package["name"])
            for invalid in ("1.2", "1.2.3-rc.1", "01.2.3", "1.2.3-nightly.2026101.1"):
                with self.subTest(invalid=invalid), self.assertRaises(release.ReleaseError):
                    release.apply_version(invalid, fixture)


class Publishing(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)

    def test_cask_version_and_checksum_follow_the_macos_asset(self):
        cask = self.base / "oxplay.rb"
        shutil.copyfile(ROOT / "Casks/oxplay.rb", cask)
        assets = self.base / "assets"
        assets.mkdir()
        archive = assets / "oxplay-v0.2.0-nightly.20261001.3-macOS-ARM64.zip"
        archive.write_bytes(b"synthetic zip")
        release.update_cask({"tag": "v0.2.0-nightly.20261001.3", "version": "0.2.0-nightly.20261001.3"}, assets, cask)
        text = cask.read_text()
        self.assertIn('  version "0.2.0-nightly.20261001.3"\n', text)
        self.assertIn(f'  sha256 "{hashlib.sha256(b"synthetic zip").hexdigest()}"\n', text)
        self.assertIn("oxplay-v#{version}-macOS-ARM64.zip", text)
        with self.assertRaisesRegex(release.ReleaseError, "missing"):
            release.update_cask({"tag": "v9.9.9", "version": "9.9.9"}, assets, cask)

    def test_dry_run_plans_never_reach_github(self):
        target = self.base / "target"
        target.mkdir()
        plan = {"schema": 1, "channel": "production", "dry_run": True, "version": "0.1.0", "tag": "v0.1.0",
                "notes": "## 0.1.0\n", "repository": "Example/oxplay", "git_head": "a" * 40}
        (target / "release-plan.json").write_text(json.dumps(plan))
        called = []
        previous = Path.cwd()
        os.chdir(self.base)
        self.addCleanup(os.chdir, previous)
        with mock.patch.object(release, "gh", side_effect=lambda *a, **k: called.append(a)), \
                mock.patch.object(release, "run", side_effect=lambda *a: called.append(a)), \
                mock.patch.dict(os.environ, {"GITHUB_REF_NAME": "main", "DRY_RUN": "true"}), \
                mock.patch("sys.argv", ["release.py", "publish", "--channel", "production", "--target", "b" * 40]):
            self.assertEqual(release.main(), 1)
        self.assertEqual(called, [])
        with mock.patch.dict(os.environ, {"MACOS_SIGNED": "false"}), \
                mock.patch("sys.argv", ["release.py", "notes", "--channel", "production"]):
            self.assertEqual(release.main(), 0)
        notes = (target / "release-notes.md").read_text()
        self.assertTrue(notes.startswith("## 0.1.0\n"))
        self.assertIn("xattr -dr com.apple.quarantine", notes)
        self.assertIn("SHA256SUMS.txt", notes)

    def test_dry_run_release_commit_stays_local(self):
        repo = Repository(self.base)
        (repo.path / "Casks").mkdir()
        shutil.copyfile(ROOT / "Casks/oxplay.rb", repo.path / "Casks/oxplay.rb")
        (repo.path / "Cargo.lock").write_text("version = 4\n")
        head = repo.commit("Add packaging files")
        (repo.path / "Cargo.toml").write_text((repo.path / "Cargo.toml").read_text().replace("0.1.0", "0.1.1"))
        assets = repo.path / "release-assets"
        assets.mkdir()
        (assets / "oxplay-v0.1.1-macOS-ARM64.zip").write_bytes(b"synthetic zip")
        (repo.path / "target").mkdir()
        (repo.path / "target/release-plan.json").write_text(json.dumps({
            "schema": 1, "channel": "production", "dry_run": True, "version": "0.1.1", "tag": "v0.1.1",
            "notes": "", "repository": "Example/oxplay", "git_head": head}))
        previous = Path.cwd()
        os.chdir(repo.path)
        self.addCleanup(os.chdir, previous)
        with mock.patch.object(release, "gh", side_effect=AssertionError("GitHub API used in a dry run")), \
                mock.patch.dict(os.environ, {"GITHUB_REF_NAME": "release-pipeline", "DRY_RUN": "true"}), \
                mock.patch("sys.argv", ["release.py", "commit-version", "--channel", "production"]):
            self.assertEqual(release.main(), 0)
        self.assertEqual(repo.git("rev-parse", "HEAD~1").strip(), head)
        self.assertEqual(repo.git("log", "-1", "--format=%s").strip(), "chore(release): 0.1.1 [skip ci]")
        self.assertEqual(sorted(repo.git("show", "--name-only", "--format=", "HEAD").split()),
                         ["Cargo.toml", "Casks/oxplay.rb"])
        self.assertIn('version "0.1.1"', repo.git("show", "HEAD:Casks/oxplay.rb"))

    def test_channel_change_after_planning_is_rejected(self):
        target = self.base / "target"
        target.mkdir()
        (target / "release-plan.json").write_text(json.dumps({"schema": 1, "channel": "nightly",
                                                              "version": "0.1.0-nightly.20261001.1",
                                                              "tag": "v0.1.0-nightly.20261001.1"}))
        previous = Path.cwd()
        os.chdir(self.base)
        self.addCleanup(os.chdir, previous)
        with self.assertRaisesRegex(release.ReleaseError, "channel changed"):
            release.load_plan("production")


if __name__ == "__main__":
    unittest.main()
