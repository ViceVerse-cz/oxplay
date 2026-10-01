#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline native CI contracts; capability probes run against synthetic tools."""

import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
CI = ROOT / ".github/workflows/ci.yml"


def native_job():
    source = CI.read_text()
    start = re.search(r"^  native-macos:\s*$", source, re.MULTILINE)
    if not start:
        raise AssertionError("CI must include the native macOS default-feature job")
    remaining = source[start.end():]
    following = re.search(r"^  [\w-]+:\s*$", remaining, re.MULTILINE)
    return remaining[:following.start()] if following else remaining


def release_build_job():
    source = (ROOT / ".github/workflows/release.yml").read_text()
    start = re.search(r"^  build:\s*$", source, re.MULTILINE)
    if not start:
        raise AssertionError("Release must include platform package builds")
    remaining = source[start.end():]
    following = re.search(r"^  [\w-]+:\s*$", remaining, re.MULTILINE)
    return remaining[:following.start()] if following else remaining


def step(job, name):
    marker = "      - name: " + name + "\n"
    if job.count(marker) != 1:
        raise AssertionError("Expected exactly one step: " + name)
    body = job.split(marker, 1)[1]
    following = re.search(r"^      - ", body, re.MULTILINE)
    return body[:following.start()] if following else body


def shell_block(body):
    marker = "        run: |\n"
    lines = body.split(marker, 1)[1].splitlines(keepends=True)
    selected = []
    for line in lines:
        if line.strip() and not line.startswith("          "):
            break
        selected.append(line)
    return textwrap.dedent("".join(selected))


@unittest.skipIf(os.name == "nt", "CI capability shell uses Unix process tools")
class NativeCapability(unittest.TestCase):
    def probe(self, capability, compiler=0):
        with tempfile.TemporaryDirectory(prefix="oxplay-capability-fixture-") as temporary:
            folder = Path(temporary)
            tools = folder / "bin"
            tools.mkdir()
            # Compile the exact workflow shell against a stub compiler. This
            # verifies shell error propagation, not hardware availability.
            clang = tools / "clang"
            clang.write_text(
                "#!/bin/sh\n"
                "test \"$COMPILER_STATUS\" = 0 || exit \"$COMPILER_STATUS\"\n"
                "while test \"$#\" -gt 0; do\n"
                "  if test \"$1\" = -o; then shift; output=$1; fi\n"
                "  shift\n"
                "done\n"
                "printf '#!/bin/sh\\nexit %s\\n' \"$CAPABILITY_STATUS\" > \"$output\"\n"
                "chmod +x \"$output\"\n"
            )
            clang.chmod(0o755)
            output = folder / "github-output"
            summary = folder / "summary"
            output.touch()
            summary.touch()
            environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                               RUNNER_TEMP=str(folder), GITHUB_OUTPUT=str(output),
                               GITHUB_STEP_SUMMARY=str(summary), COMPILER_STATUS=str(compiler),
                               CAPABILITY_STATUS=str(capability))
            script = shell_block(step(native_job(), "Check headless GPU and hardware decoder availability"))
            result = subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
                                    capture_output=True, text=True, env=environment, timeout=10)
            return result, output.read_text(), summary.read_text()

    def test_available_hardware_requires_runtime_smoke(self):
        result, output, summary = self.probe(0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "available=true\n")
        self.assertEqual(summary, "")
        smoke = step(native_job(), "Run headless native capacity, content and teardown smoke")
        self.assertIn("if: steps.hardware.outputs.available == 'true'", smoke)
        self.assertIn('"$RUNNER_TEMP/native-metal-smoke" "$RUNNER_TEMP/native-fixture.mp4"', smoke)
        self.assertNotIn("continue-on-error", smoke)

    def test_only_explicit_hardware_unavailability_skips(self):
        script = shell_block(step(native_job(), "Check headless GPU and hardware decoder availability"))
        self.assertIn("VTIsHardwareDecodeSupported(kCMVideoCodecType_H264)", script)
        self.assertIn("return device && decoder ? 0 : 77;", script)
        result, output, summary = self.probe(77)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, "available=false\n")
        self.assertIn("runner lacks required hardware", summary)
        result, output, summary = self.probe(78)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((output, summary), ("", ""))

    def test_compilation_failure_cannot_be_relabelled_hardware_unavailable(self):
        for status in (1, 77):
            with self.subTest(compiler_status=status):
                result, output, summary = self.probe(0, status)
                self.assertEqual(result.returncode, status)
                self.assertEqual((output, summary), ("", ""))

    def test_native_default_build_and_abi_compile_are_unconditional(self):
        job = native_job()
        for name in ("Build pinned private native media stack", "Check formatting and locked default native features",
                     "Run default native workspace and dependency tests", "Build locked default native release workspace",
                     "Compile native ABI and moving-frame smoke", "Test bounded native renderer caches without a GPU"):
            with self.subTest(step=name):
                body = step(job, name)
                self.assertNotRegex(body, r"(?m)^        if:", "Native source/build/test/ABI checks cannot be hardware gated")
                self.assertNotIn("continue-on-error", body)
                self.assertNotIn("--no-default-features", body)
        self.assertIn("OXPLAY_NATIVE_MPV_PREFIX", step(job, "Build pinned private native media stack"))
        self.assertIn("cargo build --workspace --release --locked", job)
        features = tomllib.loads((ROOT / "crates/app/Cargo.toml").read_text())["features"]
        self.assertIn("native-rendering", features["default"])


@unittest.skipIf(os.name == "nt", "Synthetic MSYS2 shell propagation check uses Unix stubs")
class WindowsNativeRelease(unittest.TestCase):
    def build(self, status):
        with tempfile.TemporaryDirectory(prefix="oxplay-windows-builder-fixture-") as temporary:
            folder = Path(temporary)
            tools = folder / "bin"
            tools.mkdir()
            for name, script in {
                "cygpath": '#!/bin/sh\nprintf "%s\\n" "$2"\n',
                "python": (
                    '#!/bin/sh\ntest "$BUILDER_STATUS" = 0 || exit "$BUILDER_STATUS"\n'
                    'while test "$#" -gt 0; do\n'
                    '  if test "$1" = --prefix; then shift; prefix=$1; fi\n'
                    '  shift\ndone\n'
                    'mkdir -p "$prefix"\n'
                    'touch "$prefix/libmpv-2.dll" "$prefix/native-media-provenance.json"\n'
                ),
            }.items():
                path = tools / name
                path.write_text(script)
                path.chmod(0o755)
            output = folder / "github-env"
            path_output = folder / "github-path"
            output.touch()
            path_output.touch()
            environment = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                               RUNNER_TEMP=str(folder), GITHUB_ENV=str(output),
                               GITHUB_PATH=str(path_output), BUILDER_STATUS=str(status))
            script = shell_block(step(release_build_job(), "Build private D3D11 media libraries and runtime source closure"))
            result = subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
                                    capture_output=True, text=True, env=environment, timeout=10)
            return result, output.read_text(), path_output.read_text(), str(folder / "oxplay-native-windows")

    def test_native_builder_failure_cannot_export_or_fall_back_to_stock(self):
        result, output, path, _ = self.build(17)
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual((output, path), ("", ""))
        result, output, path, prefix = self.build(0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, f"OXPLAY_NATIVE_MPV_PREFIX={prefix}\nMPV_DIR={prefix}\n")
        self.assertEqual(path, prefix + "\n")

    def test_windows_release_requires_private_native_build_and_package_evidence(self):
        job = release_build_job()
        native = step(job, "Build private D3D11 media libraries and runtime source closure")
        for argument in ("--build-ffmpeg", "--collect-runtime-sources"):
            self.assertIn(argument, native)
        self.assertNotIn("continue-on-error", native)
        build = step(job, "Check, test and build locked native Windows release")
        self.assertNotIn("--no-default-features", build)
        for command in ("cargo check --workspace --all-targets --locked",
                        "cargo clippy --workspace --all-targets --locked -- -D warnings",
                        "cargo test --workspace --locked --no-fail-fast", "cargo build --workspace --release --locked"):
            self.assertIn(command, build)
        helpers = step(job, "Fetch pinned Windows helpers")
        self.assertNotIn("mpv-dev-windows", helpers)
        package = step(job, "Stage and archive Windows package")
        self.assertIn("--native-media", package)
        self.assertIn("--uninstall-include dist/windows-uninstall.nsh", package)
        self.assertIn("test_native_windows_packaging.py", step(job, "Native Windows packaging regression (offline)"))


if __name__ == "__main__":
    unittest.main()
