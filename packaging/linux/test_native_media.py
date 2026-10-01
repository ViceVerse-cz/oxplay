"""Offline release contracts; no graphics, native compiler, package install or network."""
import hashlib
import io
import json
import os
import re
from pathlib import Path
import sys
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package as linux

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/native-media"))
import platform_build
import windows as windows_builder
sys.path.insert(0, str(ROOT / "packaging/appimage"))
import build as appimage_builder


class NativeReleaseContracts(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def prefix(self):
        prefix = self.root / "prefix"
        header = prefix / "include/mpv/render_vk.h"
        header.parent.mkdir(parents=True)
        header.write_text("#define OXPLAY_NATIVE_RENDER_ABI 1\n")
        record = {"native_render_abi": 1, "platform": "linux-vulkan", "mpv": {"revision": "pinned"},
                  "libplacebo": {"version": "7.360.1"}, "ffmpeg": {"version": "9.0.2"}}
        (prefix / "native-media-provenance.json").write_text(json.dumps(record))
        (prefix / "native-media-inventory.json").write_text(json.dumps([
            {"path": "include/mpv/render_vk.h", "sha256": hashlib.sha256(header.read_bytes()).hexdigest()}]))
        return prefix

    def test_native_input_tampering_is_rejected(self):
        prefix = self.prefix()
        self.assertEqual(linux.native_record(prefix)["platform"], "linux-vulkan")
        (prefix / "include/mpv/render_vk.h").write_text("#define OXPLAY_NATIVE_RENDER_ABI 1\nchanged")
        with self.assertRaisesRegex(ValueError, "differs from its build inventory"):
            linux.native_record(prefix)

    def test_other_native_platform_is_rejected(self):
        prefix = self.prefix()
        record = json.loads((prefix / "native-media-provenance.json").read_text())
        record["platform"] = "windows-d3d11"
        (prefix / "native-media-provenance.json").write_text(json.dumps(record))
        with self.assertRaisesRegex(ValueError, "Vulkan ABI 1"):
            linux.native_record(prefix)

    def test_inventory_cannot_escape_prefix(self):
        prefix = self.prefix()
        (prefix / "native-media-inventory.json").write_text(json.dumps([
            {"path": "../other", "sha256": "0" * 64}]))
        with self.assertRaisesRegex(ValueError, "Unsafe native inventory"):
            linux.native_record(prefix)

    def test_shared_library_aliases_resolve_to_one_private_soname(self):
        prefix = self.root / "sdk"
        (prefix / "lib").mkdir(parents=True)
        for name in ("libmpv.so.2", "libplacebo.so.360", "libavcodec.so.63"):
            (prefix / "lib" / name).write_bytes(b"ELF")
        (prefix / "lib/libmpv.so").symlink_to("libmpv.so.2")
        with patch.object(linux, "native_elf"), patch.object(linux, "output", side_effect=lambda *args: args[-1].resolve().name):
            self.assertEqual(set(linux.private_libraries(prefix)),
                             {"libmpv.so.2", "libplacebo.so.360", "libavcodec.so.63"})

    def test_source_bundle_contains_upstream_modifications_and_recipes(self):
        prefix = self.root / "sdk"
        work = self.root / "work"
        (work / "source-archives").mkdir(parents=True)
        (work / "source-archives/mpv.tar.gz").write_bytes(b"authenticated source")
        applied = [{"file": name, "sha256": hashlib.sha256((platform_build.PATCHES / name).read_bytes()).hexdigest()}
                   for name in ("common-mpv-gpu-next.patch", "linux-vulkan-interop.patch")]
        platform_build.source_bundle(prefix, work, applied)
        manifest = json.loads((prefix / "share/oxplay-native/sources.json").read_text())
        self.assertEqual(manifest["platform"], "linux-vulkan")
        archive = prefix / "share/oxplay-native/sources.tar.gz"
        self.assertEqual(manifest["bundle_sha256"], hashlib.sha256(archive.read_bytes()).hexdigest())
        with tarfile.open(archive) as sources:
            members = set(sources.getnames())
        self.assertTrue({"upstream/mpv.tar.gz", "patches/common-mpv-gpu-next.patch",
                         "patches/linux-vulkan-interop.patch", "build/linux.py",
                         "build/install-build-deps-linux.sh", "build/check_software_av1.c",
                         "build/fixtures/moving-av1-64x64.ivf", "build/fixtures/moving-av1-64x64.json"} <= members)

    def test_software_av1_fixture_and_probe_evidence_are_required(self):
        prefix, work = self.root / "prefix", self.root / "work"
        work.mkdir()
        fixture = platform_build.PATCHES.parent / "fixtures/moving-av1-64x64.ivf"
        metadata = json.loads(fixture.with_suffix(".json").read_text())
        self.assertEqual(hashlib.sha256(fixture.read_bytes()).hexdigest(), metadata["sha256"])
        self.assertEqual(fixture.stat().st_size, metadata["bytes"])
        self.assertEqual(fixture.read_bytes()[:4], b"DKIF")
        expected = "Software AV1 decode PASS: decoder=libdav1d frames=8 distinct=8 size=64x64 format=yuv420p"
        with patch.object(platform_build, "run") as compile_probe, patch.object(
                platform_build.subprocess, "check_output", return_value="-I/private/include -L/private/lib -lavcodec -lavformat -lavutil"), patch.object(
                platform_build.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout=expected + "\n")) as execute:
            platform_build.check_software_av1(prefix, work, {"CC": "gcc"}, windows=True)
        self.assertIn("-Werror", compile_probe.call_args.args[0])
        self.assertEqual(execute.call_args.kwargs["timeout"], 30)
        self.assertTrue(execute.call_args.kwargs["check"])
        self.assertEqual(Path(execute.call_args.args[0][0]).suffix, ".exe")
        evidence = json.loads((prefix / "share/oxplay-native/software-av1-decode.json").read_text())
        self.assertEqual(evidence["fixture_sha256"], metadata["sha256"])
        self.assertEqual(evidence["result"]["frames"], 8)
        self.assertFalse(evidence["result"]["hardware"])

    def test_failed_or_static_software_av1_probe_cannot_publish_evidence(self):
        for outcome in (subprocess.CalledProcessError(1, "probe"),
                        subprocess.CompletedProcess([], 0, stdout="decoder=libdav1d frames=8 distinct=1")):
            prefix, work = self.root / "failed", self.root / "work"
            work.mkdir(exist_ok=True)
            with patch.object(platform_build, "run"), patch.object(
                    platform_build.subprocess, "check_output", return_value="-lavcodec -lavformat -lavutil"), patch.object(
                    platform_build.subprocess, "run", **({"side_effect": outcome} if isinstance(outcome, Exception) else {"return_value": outcome})):
                with self.assertRaises((RuntimeError, subprocess.CalledProcessError)):
                    platform_build.check_software_av1(prefix, work, {}, windows=False)
            self.assertFalse((prefix / "share/oxplay-native/software-av1-decode.json").exists())

    def test_private_ffmpeg_requires_software_av1_on_both_platforms(self):
        for name in ("linux.py", "windows.py"):
            recipe = (platform_build.PATCHES.parent / name).read_text()
            self.assertIn('"--enable-libdav1d"', recipe)
            self.assertIn("check_software_av1(prefix,", recipe)
        dependencies = (ROOT / "packaging/linux/install-build-deps.sh").read_text()
        for package in ("libdav1d-dev", "dav1d-devel", "gnutls dav1d libass"):
            self.assertIn(package, dependencies)

    def windows_fixture(self):
        prefix, dependencies, system = [self.root / name for name in ("native", "dependencies", "windows")]
        for path in (prefix / "bin", dependencies / "bin", system / "System32"):
            path.mkdir(parents=True)
        for path in (prefix / "bin/mpv-2.dll", prefix / "bin/libplacebo-360.dll",
                     dependencies / "bin/libcodec.dll", dependencies / "bin/unused.dll",
                     system / "System32/KERNEL32.dll"):
            path.write_bytes(path.name.encode())
        return prefix, dependencies, system

    def test_windows_copies_only_transitive_imports_and_normalizes_mpv(self):
        prefix, dependencies, system = self.windows_fixture()
        imports = {"mpv-2.dll": ["libplacebo-360.dll", "KERNEL32.dll"],
                   "libplacebo-360.dll": ["libcodec.dll"], "libcodec.dll": []}
        with patch.dict(os.environ, {"SystemRoot": str(system)}), patch.object(
                windows_builder, "imported_dlls", side_effect=lambda path: imports[path.name]):
            records = windows_builder.dll_closure(prefix, dependencies)
        self.assertEqual({item["dll"] for item in records},
                         {"libmpv-2.dll", "libplacebo-360.dll", "libcodec.dll"})
        self.assertFalse((prefix / "unused.dll").exists())
        self.assertFalse((prefix / "KERNEL32.dll").exists())
        self.assertFalse((prefix / "mpv-2.dll").exists())

    def test_windows_missing_import_fails_closed(self):
        prefix, dependencies, system = self.windows_fixture()
        with patch.dict(os.environ, {"SystemRoot": str(system)}), patch.object(
                windows_builder, "imported_dlls", return_value=["missing.dll"]):
            with self.assertRaisesRegex(RuntimeError, "Unresolved native Windows DLL"):
                windows_builder.dll_closure(prefix, dependencies)

    def test_windows_checkout_preserves_audited_patch_bytes(self):
        repo = self.root / "checkout"
        repo.mkdir()
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        relative = Path("scripts/native-media/patches/common-mpv-gpu-next.patch")
        source = ROOT / relative
        target = repo / relative
        target.parent.mkdir(parents=True)
        target.write_bytes(source.read_bytes())
        (repo / ".gitattributes").write_bytes((ROOT / ".gitattributes").read_bytes())
        subprocess.run(["git", "-c", "core.autocrlf=true", "add", "."], cwd=repo, check=True)
        checkout = self.root / "windows-checkout"
        subprocess.run(["git", "-c", "core.autocrlf=true", "checkout-index", "-a",
                        "--prefix=" + str(checkout) + os.sep], cwd=repo, check=True)
        self.assertEqual((checkout / relative).read_bytes(), source.read_bytes())
        # Negative control: without the explicit attribute Git really does
        # materialize CRLF on this same checkout path/configuration.
        (repo / ".gitattributes").unlink()
        subprocess.run(["git", "-c", "core.autocrlf=true", "add", "."], cwd=repo, check=True)
        negative = self.root / "windows-negative"
        subprocess.run(["git", "-c", "core.autocrlf=true", "checkout-index", "-a",
                        "--prefix=" + str(negative) + os.sep], cwd=repo, check=True)
        self.assertIn(b"\r\n", (negative / relative).read_bytes())

    def test_dynamic_windows_tests_find_private_dlls_before_stock_sdk(self):
        prefix, dependencies = self.root / "native", self.root / "sdk"
        old = {"PATH": "ambient-sdk", "PKG_CONFIG_PATH": "ambient-pkgconfig"}
        with patch.dict(os.environ, old, clear=True):
            environment = windows_builder.windows_build_environment(prefix, dependencies)
        self.assertEqual(environment["PATH"].split(os.pathsep),
                         [str(prefix / "bin"), str(dependencies / "bin"), "ambient-sdk"])
        self.assertTrue(environment["PKG_CONFIG_PATH"].startswith(str(prefix / "lib/pkgconfig")))

    def test_windows_plan_preserves_required_native_os_and_media_features(self):
        # Exercise the public portable plan, which shares its option list with
        # the actual Meson setup. All auto-disabled Windows essentials must be
        # explicit, including the OS timer's required native thread-ID backend.
        plan = json.loads(subprocess.check_output([
            sys.executable, str(ROOT / "scripts/native-media/windows.py"),
            "--prefix", str(self.root / "prefix"), "--work-dir", str(self.root / "work"),
            "--dependencies", str(self.root / "sdk"), "--plan"], text=True))
        required = {"-Dwin32-threads=enabled", "-Dwasapi=enabled", "-Dd3d11=enabled",
                    "-Dd3d-hwaccel=enabled", "-Dshaderc=enabled", "-Dspirv-cross=enabled",
                    "-Dlibavdevice=enabled", "-Dtests=true"}
        self.assertTrue(required <= set(plan["mpv_options"]))
        self.assertEqual(plan["api"], "d3d11")

    def test_portable_source_collection_follows_actual_bundled_closure(self):
        appdir, prefix, system = [self.root / name for name in ("appdir", "prefix", "system")]
        executable = appdir / "usr/lib/oxplay/oxplay"
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b"ELF")
        prefix.mkdir()
        (prefix / "native-media-provenance.json").write_text("{}")
        system.mkdir()
        libraries = {"libmpv.so.2": str(prefix / "libmpv.so.2"),
                     "libass.so.9": str(system / "libass.so.9"),
                     "libsystemd.so.0": str(system / "libsystemd.so.0"),
                     "libapparmor.so.1": str(system / "libapparmor.so.1")}
        for path in libraries.values():
            Path(path).write_bytes(b"ELF")

        def fake_ldd(path, library_path):
            if path == executable:
                if library_path:
                    return libraries
                return {name: str(appdir / "usr/lib" / name) if name in ("libmpv.so.2", "libass.so.9")
                        else original for name, original in libraries.items()}
            self.assertEqual(path.name, "libsystemd.so.0")
            return {"libapparmor.so.1": libraries["libapparmor.so.1"]}

        with patch.object(appimage_builder, "ldd", side_effect=fake_ldd), patch.object(
                appimage_builder.package, "checked"), patch.object(
                appimage_builder, "collect_distribution_sources") as collect:
            bundled = appimage_builder.bundle_libraries(appdir, prefix)
        self.assertEqual(set(bundled), {"libmpv.so.2", "libass.so.9"})
        collect.assert_called_once_with(appdir, prefix,
                                        {name: libraries[name] for name in bundled})
        self.assertFalse((appdir / "usr/lib/libapparmor.so.1").exists())

    def test_rpm_filter_matches_only_bundled_soname_dependencies(self):
        pattern = linux.rpm_private_requires_pattern(["libmpv.so.2", "libplacebo.so.360", "libavcodec.so.63"])
        # No backslash can be consumed by RPM's macro expansion stage.
        self.assertNotIn("\\", pattern)
        for dependency in ("libmpv.so.2()(64bit)", "libavcodec.so.63(LIBAVCODEC_63)(64bit)",
                           "libplacebo.so.360", "libplacebo.so.360()(64bit)"):
            self.assertRegex(dependency, pattern)
        for dependency in ("libmpvXsoX2()(64bit)", "libmpv.so.20()(64bit)",
                           "libvulkan.so.1()(64bit)", "libass.so.9()(64bit)"):
            self.assertIsNone(re.match(pattern, dependency))

    def test_msys_source_mirror_preserves_version_epochs(self):
        self.assertEqual(windows_builder.source_recipe_url("mingw-w64-spirv-cross", "1:1.4.357.0-1"),
                         "https://mirror.msys2.org/mingw/sources/mingw-w64-spirv-cross-1~1.4.357.0-1.src.tar.zst")

    def test_source_archive_download_is_streamed_and_bounded(self):
        target = self.root / "source.tar.zst"
        data = b"source data with upstream files"
        response = io.BytesIO(data)
        response.url = "https://mirror.msys2.org/source"
        with patch.object(windows_builder.urllib.request, "urlopen", return_value=response):
            digest = windows_builder.download_source_recipe(response.url, target, max_bytes=len(data))
        self.assertEqual(target.read_bytes(), data)
        self.assertEqual(digest, hashlib.sha256(data).hexdigest())
        response = io.BytesIO(data)
        response.url = "https://mirror.msys2.org/source"
        with patch.object(windows_builder.urllib.request, "urlopen", return_value=response):
            with self.assertRaisesRegex(RuntimeError, "archive exceeds"):
                windows_builder.download_source_recipe(response.url, target, max_bytes=len(data) - 1)
        self.assertFalse(target.exists())

    def test_msys_tool_resolution_passes_an_absolute_executable(self):
        executable = self.root / "msys64/usr/bin/bash.exe"
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b"MSYS Bash")
        windows_builder.msys_executable.cache_clear()
        self.addCleanup(windows_builder.msys_executable.cache_clear)
        with patch.object(windows_builder.shutil, "which", return_value="/explicit/msys/cygpath.exe"), patch.object(
                windows_builder.subprocess, "check_output", return_value=str(executable) + "\n") as cygpath:
            actual = windows_builder.msys_executable("bash")
        self.assertEqual(actual, str(executable))
        self.assertTrue(Path(actual).is_absolute())
        cygpath.assert_called_once_with(["/explicit/msys/cygpath.exe", "-w", "/usr/bin/bash.exe"], text=True)

    def test_missing_msys_tool_cannot_fall_back_to_wsl(self):
        windows_builder.msys_executable.cache_clear()
        self.addCleanup(windows_builder.msys_executable.cache_clear)
        with patch.object(windows_builder.shutil, "which", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "MSYS2 cygpath"):
                windows_builder.msys_executable("bash")


if __name__ == "__main__":
    unittest.main()
