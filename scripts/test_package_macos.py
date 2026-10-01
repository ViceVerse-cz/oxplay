#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""No native execution, signing, account data or network is used by these tests."""
import argparse
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import package_macos as packaging
import package_helpers


class PackagingBoundaryTests(unittest.TestCase):
    def test_bundle_versions_are_numeric_for_nightlies_and_ci_builds(self):
        with patch.dict("os.environ", {}, clear=True):
            self.assertEqual(packaging.bundle_versions("0.1.0"), ("0.1.0", "0.1.0"))
            self.assertEqual(packaging.bundle_versions("0.2.0-nightly.20261001.12"), ("0.2.0", "0.2.0"))
        with patch.dict("os.environ", {"OXPLAY_BUNDLE_VERSION": "57.2"}):
            self.assertEqual(packaging.bundle_versions("0.2.0-nightly.20261001.12"), ("0.2.0", "57.2"))
        for version, build in (("0.2", ""), ("v0.2.0", ""), ("0.2.0", "57-2"), ("0.2.0", "1.2.3.4")):
            with self.subTest(version=version, build=build), \
                    patch.dict("os.environ", {"OXPLAY_BUNDLE_VERSION": build}), \
                    self.assertRaises(packaging.PackagingError):
                packaging.bundle_versions(version)

    def test_source_snapshot_includes_contract_and_standalone_baseline_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            names = ["SPEC.md", "tools/media-baseline/main.c", "tools/media-baseline/README.md",
                     "tools/unrelated/private.txt", "artifacts/local.log", "crates/app/Cargo.toml",
                     "vendor/femtovg/Cargo.toml", "vendor/femtovg/src/lib.rs", "vendor/femtovg/LICENSE-MIT",
                     "rust-toolchain.toml", ".cargo/config.toml"]
            for name in names:
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("synthetic source fixture")
            (root / "tools/media-baseline/linked").symlink_to(root / "artifacts/local.log")
            names.append("tools/media-baseline/linked")
            with patch.object(packaging, "ROOT", root), patch.object(packaging, "run", return_value="\0".join(names)):
                selected = {str(file.relative_to(root)) for file in packaging.source_files()}
            self.assertEqual(selected, {"SPEC.md", "tools/media-baseline/main.c", "tools/media-baseline/README.md", "crates/app/Cargo.toml",
                                        "vendor/femtovg/Cargo.toml", "vendor/femtovg/src/lib.rs", "vendor/femtovg/LICENSE-MIT",
                                        "rust-toolchain.toml", ".cargo/config.toml"})

    def test_locked_build_requires_exactly_both_first_party_binary_artifacts(self):
        def artifact(name, kind="bin"):
            return json.dumps({"reason": "compiler-artifact", "target": {"name": name, "kind": [kind]},
                               "executable": "/synthetic/" + name})
        records = [artifact("oxplay"), artifact("oxplay-dns"), artifact("ignored")]
        with patch.object(packaging, "run", return_value="\n".join(records)) as command:
            binaries = packaging.build_executables()
            self.assertEqual(set(binaries), {"oxplay", "oxplay-dns"})
            self.assertIn("--locked", command.call_args.args)
            self.assertIn("oxplay-network", command.call_args.args)
            self.assertIn("--bins", command.call_args.args)
            self.assertNotIn("--no-default-features", command.call_args.args)
            self.assertIn("native-rendering", command.call_args.args)
            self.assertEqual(command.call_args.kwargs["env"]["OXPLAY_NATIVE_MPV_PREFIX"],
                             str(packaging.NATIVE_PREFIX.resolve()))
        for malformed in ([records[0]], records + [records[1]], [records[0], artifact("oxplay-dns", "example")]):
            with self.subTest(records=malformed), patch.object(packaging, "run", return_value="\n".join(malformed)):
                with self.assertRaises(packaging.PackagingError):
                    packaging.build_executables()

    def native_fixture(self, root):
        prefix = root / "artifacts/native-media/macos/prefix"
        inputs = {"lib/libmpv.2.dylib": b"synthetic mpv; never executed",
                  "lib/libplacebo.365.dylib": b"synthetic libplacebo; never executed",
                  "include/mpv/render_mtl.h": b"#define OXPLAY_NATIVE_RENDER_ABI 1\n",
                  "share/licenses/native-media/mpv/LICENSE": b"synthetic license fixture"}
        for name, data in inputs.items():
            path = prefix / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        archive = prefix.parent / "downloads/mpv-pinned.tar.gz"
        archive.parent.mkdir()
        archive.write_bytes(b"synthetic archive; never extracted")
        pins = {"mpv": {"revision": "pinned", "bytes": archive.stat().st_size,
                        "sha256": packaging.digest(archive)}}
        source = root / "scripts/native-media/macos-sources.json"
        source.parent.mkdir(parents=True)
        packaging.json_write(source, {"sources": pins})
        patch_file = source.parent / "patches/native.patch"
        patch_file.parent.mkdir()
        patch_file.write_text("synthetic patch; never applied")
        packaging.json_write(prefix.parent / "build-result.json", {
            "schema": 1, "status": "compiled_and_installed", "native_render_abi": 1,
            "sources": pins, "patches": {patch_file.name: packaging.digest(patch_file)},
            "installed_files": {name: packaging.digest(prefix / name) for name in inputs}})
        return prefix

    def test_native_media_inventory_binds_libraries_sources_patches_and_licenses(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prefix = self.native_fixture(root)
            with patch.object(packaging, "ROOT", root):
                inputs = packaging.native_media_inputs(prefix)
                self.assertEqual(inputs["backend"], "metal")
                self.assertEqual({x["target"] for x in inputs["evidence"]},
                                 {"build-result.json", "macos-sources.json", "sources/mpv-pinned.tar.gz",
                                  "patches/native.patch", "licenses/mpv/LICENSE"})
                (prefix / "lib/libmpv.2.dylib").write_bytes(b"changed library")
                with self.assertRaisesRegex(packaging.PackagingError, "changed after"):
                    packaging.native_media_inputs(prefix)

    def test_native_media_source_license_and_patch_tampering_fail(self):
        for name in ("downloads/mpv-pinned.tar.gz", "prefix/share/licenses/native-media/mpv/LICENSE",
                     "../../../../scripts/native-media/patches/native.patch"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                prefix = self.native_fixture(root)
                file = prefix.parent / name
                if name.startswith("../"):
                    file = root / "scripts/native-media/patches/native.patch"
                file.write_bytes(b"changed evidence")
                with patch.object(packaging, "ROOT", root), self.assertRaises(packaging.PackagingError):
                    packaging.native_media_inputs(prefix)

    def test_native_media_closure_rejects_stock_and_mixed_mpv(self):
        mpv = Path("/private/prefix/lib/libmpv.2.dylib")
        placebo = Path("/private/prefix/lib/libplacebo.365.dylib")
        inputs = {"installed": [{"source": str(x), "kind": "installed"} for x in (mpv, placebo)]}
        packaging.require_native_media_closure({mpv: {}, placebo: {}}, inputs)
        for graph in ({mpv: {}}, {Path("/opt/homebrew/lib/libmpv.2.dylib"): {}, placebo: {}},
                      {mpv: {}, placebo: {}, Path("/stock/lib/libplacebo.360.dylib"): {}}):
            with self.assertRaises(packaging.PackagingError):
                packaging.require_native_media_closure(graph, inputs)

    def test_macos_release_ceiling_rejects_newer_transitive_dependencies(self):
        graph = {Path("/private/libmpv.dylib"): {"minimum_macos": "12.0"},
                 Path("/homebrew/libavcodec.dylib"): {"minimum_macos": "26.0"}}
        packaging.require_macos_ceiling(graph, "26")
        packaging.require_macos_ceiling(graph, "26.0")
        graph[Path("/homebrew/libavcodec.dylib")]["minimum_macos"] = "27.0"
        with self.assertRaisesRegex(packaging.PackagingError, "libavcodec"):
            packaging.require_macos_ceiling(graph, "26.0")

    def test_dns_offline_probe_uses_empty_stdin_clean_environment_and_timeout(self):
        import subprocess
        result = subprocess.CompletedProcess([], 2, b"", b"")
        with patch.object(packaging.subprocess, "run", return_value=result) as command:
            report = packaging.validate_dns_helper(Path("/synthetic/Oxplay.app/Contents/Helpers/oxplay-dns"))
            self.assertFalse(report["network_requested"])
            self.assertFalse(report["cancellation_and_system_dns_qualified"])
            options = command.call_args.kwargs
            self.assertEqual(options["input"], b"")
            self.assertEqual(options["timeout"], 5)
            self.assertEqual(set(options["env"]), {"PATH", "HOME", "TMPDIR"})
        for code, stdout, stderr in ((0, b"", b""), (2, b"leak", b""), (2, b"", b"leak")):
            with patch.object(packaging.subprocess, "run", return_value=subprocess.CompletedProcess([], code, stdout, stderr)):
                with self.assertRaises(packaging.PackagingError):
                    packaging.validate_dns_helper(Path("/synthetic/oxplay-dns"))

    def test_ca_notices_bind_original_source_and_license_to_exact_public_bundle(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            keg = root / "Cellar/ca-certificates/synthetic-version"
            bundle = keg / "share/ca-certificates/cacert.pem"
            bundle.parent.mkdir(parents=True)
            cache = root / "third_party/ca-certificates/synthetic-version"
            cache.mkdir(parents=True)
            (cache / "certdata.txt").write_text("Synthetic source fixture; not Mozilla source")
            (cache / "LICENSE-MPL-2.0").write_text("Synthetic license fixture; not a license grant")
            source_hash = packaging.digest(cache / "certdata.txt")
            bundle.write_text(f"## SHA256: {source_hash}\n-----BEGIN CERTIFICATE-----\nsynthetic\n")
            evidence = {"schema": 1, "formula": "ca-certificates", "version": keg.name,
                        "declared_license": "MPL-2.0", "bundle_sha256": packaging.digest(bundle),
                        "certdata_sha256": source_hash,
                        "files": [{"path": name, "sha256": packaging.digest(cache / name)}
                                  for name in ("certdata.txt", "LICENSE-MPL-2.0")]}
            packaging.json_write(cache / "manifest.json", evidence)
            with patch.object(packaging, "ROOT", root):
                files, recorded = packaging.ca_notice_evidence(keg, root / "output")
                self.assertEqual(len(files), 3)
                self.assertEqual(recorded, evidence)
                self.assertEqual((root / "output/upstream-certdata.txt").read_bytes(), (cache / "certdata.txt").read_bytes())
                (cache / "LICENSE-MPL-2.0").write_text("tampered")
                with self.assertRaisesRegex(packaging.PackagingError, "failed verification"):
                    packaging.ca_notice_evidence(keg, root / "tampered-output")

    def test_ca_notices_reject_different_bundle_and_source_header(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            keg = root / "Cellar/ca-certificates/synthetic-version"
            bundle = keg / "share/ca-certificates/cacert.pem"
            bundle.parent.mkdir(parents=True)
            bundle.write_text("## SHA256: " + "0" * 64 + "\n")
            cache = root / "third_party/ca-certificates/synthetic-version"
            cache.mkdir(parents=True)
            evidence = {"schema": 1, "formula": "ca-certificates", "version": keg.name,
                        "declared_license": "MPL-2.0", "bundle_sha256": "0" * 64,
                        "certdata_sha256": "1" * 64, "files": []}
            with patch.object(packaging, "ROOT", root):
                self.assertEqual(packaging.ca_notice_evidence(keg, root / "missing"), ([], None))
                packaging.json_write(cache / "manifest.json", evidence)
                with self.assertRaisesRegex(packaging.PackagingError, "exact bundled"):
                    packaging.ca_notice_evidence(keg, root / "mismatch")
                evidence["bundle_sha256"] = packaging.digest(bundle)
                packaging.json_write(cache / "manifest.json", evidence)
                with self.assertRaisesRegex(packaging.PackagingError, "retained Mozilla"):
                    packaging.ca_notice_evidence(keg, root / "bad-header")

    def test_helper_runtime_validation_rejects_host_imports_and_trust_paths(self):
        root = Path('/synthetic/Oxplay.app')
        audit = {'prefix': str(root / 'runtime'), 'certificate_file': str(root / 'ca.pem'),
                 'paths': [str(root / 'packages')], 'modules': {'builtin': None, 'module': str(root / 'module.py')}}
        package_helpers.validate_runtime_paths(audit, root, packaging)
        for key, value in [('certificate_file', '/opt/homebrew/etc/ca-certificates/cert.pem'), ('prefix', '/synthetic/Oxplay.app-evil/runtime')]:
            with self.assertRaises(packaging.PackagingError):
                package_helpers.validate_runtime_paths(audit | {key: value}, root, packaging)
        with self.assertRaises(packaging.PackagingError):
            package_helpers.validate_runtime_paths(audit | {'modules': {'bad': '/usr/local/lib/python/foreign.py'}}, root, packaging)

    def test_helper_plan_excludes_host_site_hooks_and_rejects_changed_package_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            prefix = root / "python"
            (prefix / "bin").mkdir(parents=True)
            executable = prefix / "bin/python3.14"
            executable.write_bytes(b"synthetic interpreter; never executed")
            interpreter = prefix / "Resources/Python.app/Contents/MacOS/Python"
            interpreter.parent.mkdir(parents=True)
            interpreter.write_bytes(b"synthetic actual interpreter; never executed")
            stdlib = prefix / "lib/python3.14"
            (stdlib / "encodings").mkdir(parents=True)
            (stdlib / "encodings/__init__.py").write_text("# synthetic stdlib")
            (stdlib / "site-packages").mkdir()
            (stdlib / "site-packages/host.pth").write_text("raise RuntimeError('must never execute')")
            packages = root / "packages"
            packages.mkdir()
            fixture = packages / "synthetic.py"
            fixture.write_text("# synthetic selected distribution file")
            hook = packages / "untrusted.pth"
            hook.write_text("raise RuntimeError('must never execute')")
            package_records = [{"name": name, "version": "0.0.0", "location": str(packages),
                                "installed_tree_sha256": "synthetic", "installed_files": [
                                    {"path": file.name, "sha256": packaging.digest(file)} for file in (fixture, hook)]}
                               for name in package_helpers.RUNTIME_PACKAGES]
            package_records.append({"name": "unrelated-global", "version": "0.0.0"})
            binary = {"resolved": str(executable), "sha256": packaging.digest(executable)}
            inputs = {"python": binary, "deno": binary, "python_environment": {"packages": package_records}}
            with patch.object(packaging, "keg_for", side_effect=lambda p: root if p.resolve().is_relative_to(root) else None), patch.object(package_helpers, "mozilla_bundle", return_value=(fixture, packaging.digest(fixture))):
                plan = package_helpers.helper_plan(inputs, packaging)
                self.assertFalse(any(".pth" in item["target"] or "site-packages" in item["target"] for item in plan["files"]))
                self.assertEqual(plan["excluded_global_packages"], ["unrelated-global"])
                fixture.write_text("changed after inventory")
                with self.assertRaises(packaging.PackagingError):
                    package_helpers.helper_plan(inputs, packaging)

    def test_helper_plan_rejects_unknown_stdlib_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "bin").mkdir()
            executable = root / "bin/python3.14"
            executable.write_bytes(b"synthetic")
            interpreter = root / "Resources/Python.app/Contents/MacOS/Python"
            interpreter.parent.mkdir(parents=True)
            interpreter.write_bytes(b"synthetic actual interpreter; never executed")
            stdlib = root / "lib/python3.14"
            (stdlib / "encodings").mkdir(parents=True)
            (stdlib / "encodings/__init__.py").write_text("# synthetic")
            (stdlib / "profile-link").symlink_to("/nonexistent/private-profile")
            binary = {"resolved": str(executable), "sha256": packaging.digest(executable)}
            inputs = {"python": binary, "deno": binary, "python_environment": {"packages": []}}
            with patch.object(packaging, "keg_for", return_value=root):
                with self.assertRaisesRegex(packaging.PackagingError, "symlink"):
                    package_helpers.helper_plan(inputs, packaging)

    def test_system_allowlist_rejects_prefix_lookalikes_and_parent_escape(self):
        self.assertTrue(packaging.is_system("/usr/lib/libSystem.B.dylib"))
        self.assertTrue(packaging.is_system("/System/Library/Frameworks/AppKit.framework/AppKit"))
        for load in ("/usr/library/libx.dylib", "/System/LibraryEvil/libx.dylib",
                     "/System/Library/../../opt/homebrew/lib/libx.dylib", "@loader_path/libx.dylib"):
            self.assertFalse(packaging.is_system(load))

    def test_otool_load_parser_preserves_spaces_and_weak_dependencies(self):
        listing = "example:\n\t/opt/vendor/My Library.dylib (compatibility version 1.0.0, current version 2.0.0)\n\t/usr/lib/swift/libswiftCore.dylib (compatibility version 1.0.0, current version 1.0.0, weak)\n"
        self.assertEqual(packaging.parse_otool(listing), ["/opt/vendor/My Library.dylib", "/usr/lib/swift/libswiftCore.dylib"])

    def test_rpath_resolution_uses_real_selected_file_and_rejects_missing_load(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Frameworks").mkdir()
            library = root / "Frameworks/libsynthetic.dylib"
            library.write_bytes(b"synthetic path fixture; never executed")
            executable = root / "oxplay"
            self.assertEqual(packaging.resolve_load("@rpath/libsynthetic.dylib", executable, executable,
                                                    ["@loader_path/Frameworks"]), library.resolve())
            with self.assertRaises(packaging.PackagingError):
                packaging.resolve_load("@rpath/missing.dylib", executable, executable, ["@loader_path/Frameworks"])

    def test_existing_output_is_refused_before_any_build_or_native_tool(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "existing.app"
            output.mkdir()
            marker = output / "preserve"
            marker.write_text("synthetic existing user data")
            with patch.object(packaging.sys, "platform", "darwin"), patch.object(packaging, "run") as command:
                with self.assertRaises(packaging.PackagingError):
                    packaging.package(argparse.Namespace(output=output, command="bundle"))
                command.assert_not_called()
            self.assertEqual(marker.read_text(), "synthetic existing user data")

    def test_receipt_redaction_covers_path_values_and_native_input_keys(self):
        home = str(Path.home())
        original = {home + "/source/app": {"receipt": home + "/Library/Caches/Homebrew"}}
        redacted = packaging.sanitized(original)
        self.assertEqual(redacted, {"${HOME}/source/app": {"receipt": "${HOME}/Library/Caches/Homebrew"}})
        self.assertIn(home + "/source/app", original)

    def test_supplemental_notice_requires_matching_crate_revision_and_untampered_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            crate = root / "crate"
            crate.mkdir()
            (crate / "Cargo.toml").write_text('[package]\nname="synthetic-notice-test"\nversion="0.0.0"\nlicense="MIT"\nrepository="https://github.com/example/synthetic-notice-test"\n')
            vcs = crate / ".cargo_vcs_info.json"
            vcs.write_text(json.dumps({"git": {"sha1": "1" * 40}}))
            package = {"name": "synthetic-notice-test", "version": "0.0.0", "license": "MIT", "manifest_path": str(crate / "Cargo.toml")}
            cache = root / "third_party/notices/synthetic-notice-test-0.0.0"
            cache.mkdir(parents=True)
            source = cache / "000-LICENSE"
            source.write_text("Synthetic notice test fixture; not an actual upstream license.")
            record = {"name": package["name"], "version": package["version"], "declared_license": "MIT", "repository": "https://github.com/example/synthetic-notice-test", "revision": "1" * 40, "cargo_vcs_info_sha256": packaging.digest(vcs), "files": [{"path": source.name, "sha256": packaging.digest(source)}]}
            packaging.json_write(cache.parent / "manifest.json", {"packages": [record]})
            destination = root / "output"
            destination.mkdir()
            with patch.object(packaging, "ROOT", root):
                copied, evidence = packaging.supplemental_notices(package, destination)
                self.assertEqual(len(copied), 1)
                self.assertEqual(evidence["revision"], "1" * 40)
                self.assertEqual((destination / copied[0]).read_bytes(), source.read_bytes())
                source.write_text("tampered synthetic fixture")
                with self.assertRaises(packaging.PackagingError):
                    packaging.supplemental_notices(package, destination)
                vcs.write_text(json.dumps({"git": {"sha1": "2" * 40}}))
                with self.assertRaises(packaging.PackagingError):
                    packaging.supplemental_notices(package, destination)


if __name__ == "__main__":
    unittest.main()
