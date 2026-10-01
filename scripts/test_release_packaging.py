#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic Linux/AppImage/Windows packaging checks; no native package tools,
network or application launch (the launcher runs against a stub executable)."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load(name: str, path: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


linux = load("package", "packaging/linux/package.py")
appimage = load("appimage_build", "packaging/appimage/build.py")
windows = load("windows_package", "packaging/windows/package.py")
import_lib = load("mpv_import_lib", "packaging/windows/mpv_import_lib.py")


def helpers_directory(base: Path, windows_names: bool = False) -> Path:
    directory = base / "pinned"
    directory.mkdir()
    if windows_names:
        pe = b"MZ" + b"\0" * 58 + (64).to_bytes(4, "little") + b"PE\0\0" + (0x8664).to_bytes(2, "little") + b"\0" * 64
        for name in ("yt-dlp.exe", "deno.exe"):
            (directory / name).write_bytes(pe)
        names = ["yt-dlp-windows-x86_64", "deno-windows-x86_64", "mpv-dev-windows-x86_64"]
    else:
        for name in ("yt-dlp", "deno"):
            (directory / name).write_bytes(b"\x7fELF\x02\x01synthetic helper")
        names = ["yt-dlp-linux-x86_64", "deno-linux-x86_64"]
    fetch = load("fetch_pinned", "scripts/fetch_pinned.py")
    (directory / "pinned-inputs.json").write_text(json.dumps(fetch.provenance(names)))
    return directory


class LinuxPayload(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.binary = self.base / "oxplay"
        self.binary.write_bytes(b"\x7fELF\x02\x01synthetic application")
        self.helpers = helpers_directory(self.base)

    def test_package_versions_sort_below_the_final_release(self):
        self.assertEqual(linux.deb_version("0.2.0-nightly.20261001.12"), "0.2.0~nightly.20261001.12-1")
        self.assertEqual(linux.deb_version("0.2.0"), "0.2.0-1")
        self.assertEqual(linux.rpm_version("0.2.0-nightly.20261001.12"), "0.2.0~nightly.20261001.12")
        self.assertEqual(linux.arch_version("0.2.0-nightly.20261001.12"), "0.2.0pre.nightly.20261001.12")
        self.assertTrue(linux.SEMVER.fullmatch("0.2.0-nightly.20261001.12"))
        self.assertFalse(linux.SEMVER.fullmatch("0.2.0; rm -rf /"))

    def test_system_payload_is_allowlisted_with_exact_modes(self):
        stage = self.base / "stage"
        linux.stage_payload(stage, self.binary, self.helpers, None)
        self.assertEqual(linux.payload_files(stage), sorted([
            "usr/bin/oxplay", "usr/lib/oxplay/oxplay", "usr/lib/oxplay/yt-dlp", "usr/lib/oxplay/deno",
            "usr/share/applications/cz.viceverse.oxplay.desktop", "usr/share/icons/hicolor/scalable/apps/oxplay.svg",
            "usr/share/doc/oxplay/LICENSE", "usr/share/doc/oxplay/README.md",
            "usr/share/doc/oxplay/THIRD-PARTY-NOTICES.md", "usr/share/doc/oxplay/licenses/deno-LICENSE.md",
            "usr/share/doc/oxplay/licenses/yt-dlp-LICENSE",
            "usr/share/doc/oxplay/licenses/yt-dlp-THIRD_PARTY_LICENSES.txt"]))
        executable = linux.executables("usr")
        for path in [stage, *stage.rglob("*")]:
            relative = path.relative_to(stage).as_posix()
            expected = 0o755 if path.is_dir() or relative in executable else 0o644
            self.assertEqual(path.stat().st_mode & 0o777, expected, relative)
        self.assertIn("root=/usr\n", (stage / "usr/bin/oxplay").read_text())
        notices = (stage / "usr/share/doc/oxplay/THIRD-PARTY-NOTICES.md").read_text()
        self.assertIn("distribution's libmpv", notices)
        self.assertIn("yt-dlp-linux-x86_64 2026.08.19", notices)
        with self.assertRaisesRegex(ValueError, "not a Linux executable"):
            (self.helpers / "deno").write_bytes(b"#!/bin/sh\n")
            linux.stage_payload(self.base / "other", self.binary, self.helpers, None)

    def test_missing_provenance_refuses_to_package(self):
        (self.helpers / "pinned-inputs.json").unlink()
        with self.assertRaisesRegex(ValueError, "provenance"):
            linux.stage_payload(self.base / "stage", self.binary, self.helpers, None)

    @unittest.skipIf(os.name == "nt", "POSIX launcher")
    def test_launchers_add_packaged_helpers_unless_the_caller_chose_them(self):
        stage = self.base / "Portable Dir"
        linux.stage_payload(stage, self.binary, self.helpers, None, portable=True)
        stub = stage / "usr/lib/oxplay/oxplay"
        stub.write_text('#!/bin/sh\nfor a in "$@"; do printf "%s\\n" "$a"; done\n')
        stub.chmod(0o755)
        launcher = stage / "usr/bin/oxplay"
        link = self.base / "linked-oxplay"
        link.symlink_to(launcher)
        lib = str((stage / "usr/lib/oxplay").resolve())

        def arguments(*extra, via=launcher):
            return subprocess.run(["sh", str(via), *extra], capture_output=True, text=True, check=True).stdout.splitlines()

        self.assertEqual(arguments("--url", "x"), ["--yt-dlp", f"{lib}/yt-dlp", "--deno", f"{lib}/deno", "--url", "x"])
        self.assertEqual(arguments(via=link), ["--yt-dlp", f"{lib}/yt-dlp", "--deno", f"{lib}/deno"])
        self.assertEqual(arguments("--deno", "/opt/deno"), ["--yt-dlp", f"{lib}/yt-dlp", "--deno", "/opt/deno"])
        self.assertEqual(arguments("--yt-dlp", "/opt/y"), ["--deno", f"{lib}/deno", "--yt-dlp", "/opt/y"])
        self.assertEqual(arguments("--pip-smoke-test", "--local", "/v.mp4"), ["--pip-smoke-test", "--local", "/v.mp4"])
        self.assertEqual(arguments("--help"), ["--help"])
        system = linux.launcher(linux.SYSTEM_ROOT, "lib/oxplay/oxplay")
        self.assertIn('exec "$root/lib/oxplay/oxplay" "$@"', system)
        self.assertEqual(subprocess.run(["sh", "-n"], input=system, text=True).returncode, 0)


class AppImage(unittest.TestCase):
    def test_update_information_follows_the_channel(self):
        self.assertEqual(appimage.update_information("0.2.0", "v0.2.0"),
                         ("oxplay-v0.2.0-Linux-X64.AppImage",
                          "gh-releases-zsync|ViceVerse-cz|oxplay|latest|oxplay-*-Linux-X64.AppImage.zsync"))
        name, information = appimage.update_information("0.2.0-nightly.20261001.3", "v0.2.0-nightly.20261001.3")
        self.assertIn("|latest-pre|", information)
        self.assertEqual(appimage.update_information("0.2.0", "development")[0], "oxplay-development-Linux-X64.AppImage")
        with self.assertRaises(ValueError):
            appimage.update_information("0.2.0", "v0.3.0")

    def test_host_libraries_are_excluded_and_media_libraries_bundled(self):
        for name in ("libc.so.6", "ld-linux-x86-64.so.2", "libGL.so.1", "libEGL_mesa.so.0", "libX11.so.6",
                     "libXext.so.6", "libxcb-shm.so.0", "libwayland-client.so.0", "libasound.so.2",
                     "libpulse.so.0", "libva-drm.so.2", "libvulkan.so.1", "libstdc++.so.6", "libfontconfig.so.1",
                     "libz.so.1", "libdrm.so.2", "libgcc_s.so.1"):
            self.assertTrue(appimage.EXCLUDED.match(name), name)
        for name in ("libmpv.so.2", "libavcodec.so.60", "libswscale.so.7", "libass.so.9", "libplacebo.so.338",
                     "libluajit-5.1.so.2", "libdav1d.so.7", "libzvbi.so.0", "libxml2.so.2", "libcrypto.so.3"):
            self.assertFalse(appimage.EXCLUDED.match(name), name)

    def test_portable_tarball_is_normalized_and_has_an_entry_point(self):
        with tempfile.TemporaryDirectory() as directory:
            appdir = Path(directory) / "Oxplay.AppDir"
            (appdir / "usr/bin").mkdir(parents=True)
            (appdir / "AppRun").write_text("#!/bin/sh\n")
            (appdir / "AppRun").chmod(0o755)
            (appdir / "usr/bin/oxplay").write_text("#!/bin/sh\n")
            archive = appimage.tarball(appdir, Path(directory) / "oxplay-v0.2.0-Linux-X64.tar.gz")
            with tarfile.open(archive) as contents:
                names = contents.getnames()
                self.assertEqual(names[0], "oxplay-v0.2.0-Linux-X64/oxplay")
                self.assertIn("oxplay-v0.2.0-Linux-X64/usr/bin/oxplay", names)
                self.assertNotIn("oxplay-v0.2.0-Linux-X64/AppRun", names)
                self.assertTrue(all(m.uid == 0 and m.gid == 0 and not m.uname for m in contents.getmembers()))


class Windows(unittest.TestCase):
    DUMPBIN = """
Dump of file libmpv-2.dll

    ordinal hint RVA      name

          1    0 000A1B20 mpv_abort_async_command
          2    1 000A1C00 mpv_client_api_version
          3    2 000A1D10 mpv_create
          4    3 000A1E00 mpv_render_context_create

  Summary
"""

    def test_import_library_definition_comes_from_dll_exports(self):
        names = import_lib.exports_from_dumpbin(self.DUMPBIN)
        self.assertEqual(names, ["mpv_abort_async_command", "mpv_client_api_version", "mpv_create",
                                 "mpv_render_context_create"])
        definition = import_lib.definition(names)
        self.assertTrue(definition.startswith("LIBRARY libmpv-2.dll\nEXPORTS\n"))
        self.assertIn("    mpv_create\n", definition)
        with self.assertRaises(ValueError):
            import_lib.definition(["unrelated_symbol"])

    def test_payload_is_flat_verified_and_zipped_deterministically(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            helpers = helpers_directory(base, windows_names=True)
            mpv = base / "mpv-dev"
            mpv.mkdir()
            pe = (helpers / "deno.exe").read_bytes()
            (mpv / "libmpv-2.dll").write_bytes(pe)
            binary = base / "oxplay.exe"
            binary.write_bytes(pe)
            staged = windows.stage(binary, mpv, helpers, "0.2.0-nightly.20261001.3", base / "dist")
            self.assertEqual(sorted(p.name for p in staged.iterdir()), sorted([
                "oxplay.exe", "libmpv-2.dll", "yt-dlp.exe", "deno.exe", "LICENSE.txt", "README.txt",
                "THIRD-PARTY-NOTICES.txt", "BUILD-INFO.json", "licenses"]))
            info = json.loads((staged / "BUILD-INFO.json").read_text())
            self.assertEqual(info["files"]["libmpv-2.dll"], hashlib.sha256(pe).hexdigest())
            self.assertIn("shinchiro", (staged / "THIRD-PARTY-NOTICES.txt").read_text())
            first = windows.archive(staged, base / "a.zip")
            second = windows.archive(staged, base / "b.zip")
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as bundle:
                self.assertIn("oxplay.exe", bundle.namelist())
                self.assertIn("licenses/deno-LICENSE.md", bundle.namelist())
            with self.assertRaisesRegex(ValueError, "overwrite"):
                windows.stage(binary, mpv, helpers, "0.2.0", base / "dist")
            (mpv / "libmpv-2.dll").write_bytes(b"\x7fELF not a DLL")
            with self.assertRaisesRegex(ValueError, "not a Windows executable"):
                windows.stage(binary, mpv, helpers, "0.2.0", base / "dist2")

    def test_installer_script_names_every_payload_file_for_uninstall(self):
        script = (ROOT / "packaging/windows/installer.nsi").read_text()
        for name in windows.PAYLOAD + ("LICENSE.txt", "README.txt", "THIRD-PARTY-NOTICES.txt", "BUILD-INFO.json"):
            self.assertIn(f'Delete "$INSTDIR\\{name}"', script)
        self.assertIn("RequestExecutionLevel user", script)
        self.assertNotIn('RMDir /r "$INSTDIR"', script)


@unittest.skipIf(os.name == "nt", "bash signing regression")
class MacSigning(unittest.TestCase):
    def test_synthetic_developer_id_signing_regression(self):
        result = subprocess.run(["bash", str(ROOT / "packaging/macos/test_sign_release.sh"),
                                 str(ROOT / "packaging/macos/sign-release.sh")],
                                cwd=ROOT, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Mac signing regression passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
