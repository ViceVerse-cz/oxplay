#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline synthetic boundary tests; no installed build or account mutation."""
import gzip
import io
import json
import os
from pathlib import Path
import stat
import shutil
import sys
import tarfile
import tempfile
import unittest
import urllib.request
from unittest.mock import patch

import mpv_timer_diagnostic as diagnostic


def archive_bytes(entries):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, kind, data in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.mode = 0o4777
            if kind == tarfile.REGTYPE:
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
            else:
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    member.linkname = data.decode()
                archive.addfile(member)
    return gzip.compress(raw.getvalue())


class InputTests(unittest.TestCase):
    def test_system_pkgconfig_captures_reviewed_policy_and_exact_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            brew = Path(temporary)
            homebrew = brew / "Library/Homebrew"
            policy = homebrew / "extend/os/mac/extend/ENV/super.rb"
            shim = homebrew / "shims/mac/super/bin/pkg-config"
            for path in (policy, shim):
                path.parent.mkdir(parents=True)
            policy.write_text("#{HOMEBREW_LIBRARY}/Homebrew/os/mac/pkgconfig/#{MacOS.version}")
            shim.write_text("--define-variable=homebrew_sdkroot=${HOMEBREW_SDKROOT}")
            directory = homebrew / "os/mac/pkgconfig/27"
            directory.mkdir(parents=True)
            for name in ("zlib.pc", "bzip2.pc"):
                (directory / name).write_text("synthetic metadata")
            evidence, selected = diagnostic.system_pkgconfig(brew, "27")
            self.assertEqual(selected, directory)
            self.assertEqual(set(evidence["files"]), {"zlib.pc", "bzip2.pc"})
            self.assertNotIn(str(brew), json.dumps(evidence))
            policy.write_text("changed policy")
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "policy changed"):
                diagnostic.system_pkgconfig(brew, "27")

    def test_system_pkgconfig_rejects_unreviewed_version_before_path_access(self):
        for version in ("../27", "10", "27.1", "future"):
            with self.subTest(version=version), self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.system_pkgconfig(Path("/synthetic-unread"), version)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def test_offline_input_hash_and_missing_do_not_download(self):
        p = self.root / "input"
        p.write_bytes(b"synthetic verified bytes")
        with patch.object(diagnostic, "download") as download:
            self.assertEqual(diagnostic.checked_input(p, "unused", diagnostic.digest(p.read_bytes()), 100, False), p.read_bytes())
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "checksum"):
                diagnostic.checked_input(p, "unused", "0" * 64, 100, True)
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "absent"):
                diagnostic.checked_input(self.root / "absent", "unused", "0" * 64, 100, False)
            download.assert_not_called()

    def test_download_only_after_explicit_flag_still_verifies(self):
        with patch.object(diagnostic, "download", return_value=b"pinned") as download:
            self.assertEqual(diagnostic.checked_input(None, "url", diagnostic.digest(b"pinned"), 100, True), b"pinned")
            download.assert_called_once_with("url", 100)
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "checksum"):
                diagnostic.checked_input(None, "url", "0" * 64, 100, True)

    def test_regular_file_bounds_and_no_follow(self):
        path = self.root / "file"
        path.write_bytes(b"abc")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.read_file(path, 2)
        link = self.root / "link"
        link.symlink_to(path)
        with self.assertRaises(OSError):
            diagnostic.read_file(link, 100)
        if hasattr(os, "mkfifo"):
            fifo = self.root / "fifo"
            os.mkfifo(fifo)
            with self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.read_file(fifo, 100)

    def test_safe_extraction_preserves_only_executable_intent(self):
        data = archive_bytes([("mpv-0.41.0", tarfile.DIRTYPE, b""),
                              ("mpv-0.41.0/src/test.c", tarfile.REGTYPE, b"fixture")])
        destination = self.root / "source"
        diagnostic.extract_source(data, destination)
        result = destination / "src/test.c"
        self.assertEqual(result.read_bytes(), b"fixture")
        self.assertEqual(stat.S_IMODE(result.stat().st_mode), 0o700)
        self.assertNotIn(str(self.root), json.dumps(diagnostic.source_inventory(destination)))
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "already exists"):
            diagnostic.extract_source(data, destination)

    def test_all_paths_preflight_before_any_source_write(self):
        for name in ["mpv-0.41.0/../escape", "/mpv-0.41.0/file", "other/file", "mpv-0.41.0/a\\b"]:
            with self.subTest(name=name):
                destination = self.root / "source"
                data = archive_bytes([("mpv-0.41.0/first", tarfile.REGTYPE, b"first"), (name, tarfile.REGTYPE, b"bad")])
                with self.assertRaises(diagnostic.DiagnosticError):
                    diagnostic.extract_source(data, destination)
                self.assertFalse(destination.exists())

    def test_links_special_entries_duplicates_and_file_parents_rejected(self):
        cases = [
            [("mpv-0.41.0/link", tarfile.SYMTYPE, b"/tmp/outside")],
            [("mpv-0.41.0/link", tarfile.LNKTYPE, b"mpv-0.41.0/first")],
            [("mpv-0.41.0/fifo", tarfile.FIFOTYPE, b"")],
            [("mpv-0.41.0/a", tarfile.REGTYPE, b"1"), ("mpv-0.41.0/a", tarfile.REGTYPE, b"2")],
            [("mpv-0.41.0/a", tarfile.REGTYPE, b"1"), ("mpv-0.41.0/a/b", tarfile.REGTYPE, b"2")],
        ]
        for entries in cases:
            with self.subTest(entries=entries):
                with self.assertRaises(diagnostic.DiagnosticError):
                    diagnostic.extract_source(archive_bytes(entries), self.root / "source")
                self.assertFalse((self.root / "source").exists())

    def test_expansion_and_pax_metadata_limits_precede_extraction(self):
        data = archive_bytes([("mpv-0.41.0/large", tarfile.REGTYPE, bytes(4096))])
        with patch.object(diagnostic, "MAX_EXPANDED", 1024):
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "Expanded archive"):
                diagnostic.extract_source(data, self.root / "source")
        header = tarfile.TarInfo("pax")
        header.type = tarfile.XHDTYPE
        header.size = 65537
        data = gzip.compress(header.tobuf() + bytes(66048) + bytes(1024))
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "PAX metadata"):
            diagnostic.extract_source(data, self.root / "source")
        self.assertFalse((self.root / "source").exists())

    def test_member_size_and_entry_count_are_bounded(self):
        data = archive_bytes([("mpv-0.41.0/a", tarfile.REGTYPE, b"123")])
        with patch.object(diagnostic, "MAX_MEMBER", 2):
            with self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.extract_source(data, self.root / "source")
        with patch.object(diagnostic, "MAX_ENTRIES", 0):
            with self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.extract_source(data, self.root / "source")

    def test_native_inventory_labels_and_boundary(self):
        brew = self.root / "brew"
        keg = brew / "Cellar/mpv/0.41.0_10"
        (keg / "lib/pkgconfig").mkdir(parents=True)
        receipt = {"source": {"versions": {"stable": "0.41.0"}}, "arch": "arm64", "runtime_dependencies": []}
        (keg / "INSTALL_RECEIPT.json").write_text(json.dumps(receipt))
        (keg / "lib/libmpv.2.dylib").write_bytes(b"synthetic dylib bytes")
        inventory, pc_dirs = diagnostic.native_inventory(brew, keg)
        self.assertNotIn(str(self.root), json.dumps(inventory))
        self.assertIn("cellar/mpv/0.41.0_10/lib/libmpv.2.dylib", inventory["files"])
        self.assertEqual(pc_dirs, [keg / "lib/pkgconfig"])
        with patch.object(diagnostic, "MAX_NATIVE_BYTES", 1):
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "budget"):
                diagnostic.native_inventory(brew, keg)
        (keg / "lib/escape.dylib").symlink_to(self.root / "outside")
        (self.root / "outside").write_bytes(b"outside")
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "escapes"):
            diagnostic.native_inventory(brew, keg)

    def test_tool_probe_absence_never_installs(self):
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "Missing prerequisite: meson"):
            diagnostic.tool_paths([self.root])

    def test_recipe_args_require_exact_reviewed_formula_and_std_body(self):
        args = ["--sysconfdir=#{etc}", "-Dbuild-date=false", "-Dhtml-build=enabled", "-Djavascript=enabled", "-Dlibmpv=true", "-Dlua=luajit", "-Dlibarchive=enabled", "-Duchardet=enabled", "-Dvulkan=enabled"]
        formula = ("def install\n args = %W[\n" + "\n".join(args) + "\n]\nend\n").encode()
        std = ("def std_meson_args(prefix: self.prefix, libdir: \"lib\")\n" + diagnostic.STD_MESON + "\nend\n").encode()
        with patch.object(diagnostic, "FORMULA_SHA", diagnostic.digest(formula)):
            self.assertEqual(diagnostic.recipe_arguments(formula, std)[0], args)
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "standard Meson"):
                diagnostic.recipe_arguments(formula, std.replace(b"release", b"debug"))
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "recipe differs"):
                diagnostic.recipe_arguments(formula + b"# changed", std)

    def test_pair_plan_same_environment_and_recipe_except_isolated_prefix_and_timer(self):
        tools = {"meson": Path("/synthetic/bin/meson")}
        env = {"CC": "/synthetic/clang", "HOME": str(self.root / "home")}
        plans = diagnostic.plans(self.root, tools, ["--sysconfdir=#{etc}", "-Dlibmpv=true"], env)
        self.assertEqual([p["variant"] for p in plans], ["on", "off"])
        self.assertEqual(plans[0]["environment"], plans[1]["environment"])
        normalized = []
        for plan in plans:
            commands = plan["commands"]
            self.assertEqual(len(commands), 3)
            self.assertIn("--wrap-mode=nofallback", commands[0])
            self.assertIn("--buildtype=release", commands[0])
            self.assertIn("--no-rebuild", commands[2])
            normalized.append(json.dumps(commands).replace("build-" + plan["variant"], "build-VARIANT")
                              .replace("prefix-" + plan["variant"], "prefix-VARIANT")
                              .replace("-Dgpu-pass-timers=" + ("true" if plan["variant"] == "on" else "false"), "-Dgpu-pass-timers=VARIANT"))
        self.assertEqual(*normalized)

    def test_tool_execution_preserves_multicall_alias(self):
        target = self.root / "driver"
        target.write_text("#!/bin/sh\nexit 0\n")
        target.chmod(0o700)
        for name in (*diagnostic.TOOLS, "rst2html"):
            (self.root / name).symlink_to(target)
        tools = diagnostic.tool_paths([self.root])
        self.assertEqual(tools["clang++"].name, "clang++")
        self.assertEqual(tools["swiftc"].name, "swiftc")
        self.assertEqual(tools["clang++"].resolve(), target)

    def test_patch_inside_enclosing_git_repository_changes_only_private_source(self):
        git = shutil.which("git")
        if git is None:
            self.skipTest("git is required for this isolated regression")
        outer = self.root / "outer"
        outer.mkdir()
        env = {"PATH": "/usr/bin:/bin", "HOME": str(self.root),
               "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
        diagnostic.command([git, "init", "--quiet"], env, outer)
        (outer / "fixture.txt").write_text("outer untouched\n")
        output = outer / "artifacts/run"
        source = output / "source"
        source.mkdir(parents=True)
        (source / "fixture.txt").write_text("before\n")
        patch_file = output / "fixture.patch"
        patch_file.write_text("--- a/fixture.txt\n+++ b/fixture.txt\n@@ -1 +1 @@\n-before\n+after\n")
        env["GIT_CEILING_DIRECTORIES"] = str(output)
        diagnostic.command([git, "apply", "--check", str(patch_file)], env, source)
        diagnostic.command([git, "apply", str(patch_file)], env, source)
        self.assertEqual((source / "fixture.txt").read_text(), "after\n")
        self.assertEqual((outer / "fixture.txt").read_text(), "outer untouched\n")

    def test_probe_output_and_timeout_bounds(self):
        env = {"PATH": "/usr/bin:/bin", "HOME": str(self.root)}
        self.assertEqual(diagnostic.command([sys.executable, "-c", "print('fixture')"], env, self.root), b"fixture\n")
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "output limit"):
            diagnostic.command([sys.executable, "-c", "print('x'*10000)"], env, self.root, max_output=100)
        with self.assertRaisesRegex(diagnostic.DiagnosticError, "timed out"):
            diagnostic.command([sys.executable, "-c", "import time; time.sleep(60)"], env, self.root, timeout=0.1)

    def test_redirects_allow_only_pinned_https_hosts_and_no_credentials(self):
        handler = diagnostic.PinnedRedirects()
        request = urllib.request.Request("https://github.com/mpv-player/mpv/source")
        for url in ["http://github.com/source", "https://evil.invalid/source",
                    "https://user:password@github.com/source", "https://github.com:8443/source"]:
            with self.subTest(url=url):
                with self.assertRaises(diagnostic.DiagnosticError):
                    handler.redirect_request(request, None, 302, "redirect", {}, url)
        redirected = handler.redirect_request(request, None, 302, "redirect", {},
                                                "https://codeload.github.com/mpv-player/mpv/tar.gz/v0.41.0")
        self.assertEqual(redirected.host, "codeload.github.com")

    def test_download_parent_deadline_and_clean_environment(self):
        name, url, _, limit = diagnostic.INPUTS[0]
        with patch.object(diagnostic, "command", return_value=b"synthetic archive") as command:
            self.assertEqual(diagnostic.download(url, limit), b"synthetic archive")
        args, kwargs = command.call_args
        self.assertEqual(args[0][-2:], ["--internal-fetch", "0"])
        self.assertEqual(kwargs, {"timeout": 60, "max_output": limit})
        self.assertEqual(args[1]["PATH"], "/usr/bin:/bin")
        self.assertNotIn("HTTPS_PROXY", args[1])

    def test_pinned_download_refuses_arbitrary_url_before_process_launch(self):
        with patch.object(diagnostic, "command") as command:
            with self.assertRaisesRegex(diagnostic.DiagnosticError, "not a pinned"):
                diagnostic.download("https://example.invalid/source", 100)
            command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
