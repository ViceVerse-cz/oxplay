"""Offline release contracts; no graphics, native compiler, package install or network."""
import hashlib
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package as linux

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/native-media"))
import platform_build
import windows as windows_builder


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
                         "build/install-build-deps-linux.sh"} <= members)

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


if __name__ == "__main__":
    unittest.main()
