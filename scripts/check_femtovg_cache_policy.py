#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run GPU-free production-cache tests and an uncached viewport negative control.

Creates an isolated temporary package; never modifies the vendored source.
Requires Rust/Cargo. Pass --target-dir to reuse an existing test build directory.
"""

import argparse
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


# Unmodified Cargo.lock from the femtovg-0.27.0.crate release archive recorded
# in vendor/femtovg/OXPLAY-PROVENANCE.md. The upstream .gitignore ignores this
# file, so retaining it explicitly is necessary for clean-checkout --locked tests.
RELEASE_LOCK_SHA256 = "69bcc6664c572852d3d760546ed7264d8baba89c88dabafe16716b86dd9f1c14"


def prepare_package(vendored, package):
    lock = vendored / "Cargo.lock"
    if not lock.is_file():
        raise ValueError("Missing retained vendor/femtovg/Cargo.lock; the cache gate requires the original release lock")
    if hashlib.sha256(lock.read_bytes()).hexdigest() != RELEASE_LOCK_SHA256:
        raise ValueError("The retained FemtoVG lock does not match the original release archive")
    shutil.copytree(vendored, package)
    copied = package / "src/renderer/wgpu.rs"
    # copytree preserves old mtimes. A shared target directory may contain
    # the previous negative-control binary, newer than these copied files.
    # Force Cargo to check the copied production source before its test.
    copied.touch()
    manifest = package / "Cargo.toml"
    with manifest.open("a") as output:
        output.write("\n[workspace]\n")
    return manifest, copied


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    vendored = root / "vendor/femtovg"
    source = vendored / "src/renderer/wgpu.rs"
    transcript = [
        "Production source SHA-256: " + hashlib.sha256(source.read_bytes()).hexdigest(),
        "The positive suite and negative control create no WGPU device or adapter.",
    ]

    with tempfile.TemporaryDirectory(prefix="oxplay-femtovg-policy-") as temporary:
        package = Path(temporary) / "femtovg"
        try:
            manifest, copied = prepare_package(vendored, package)
        except ValueError as error:
            parser.error(str(error))
        base = [
            "cargo", "test", "--manifest-path", str(manifest), "--lib",
            "--no-default-features", "--features", "wgpu", "--locked", "-j2",
        ]
        if args.target_dir:
            base += ["--target-dir", str(args.target_dir.resolve())]

        def run(label, test_filter):
            command = base + [test_filter]
            transcript.append(label + " command: " + " ".join(command))
            result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            transcript.append(result.stdout)
            transcript.append(label + " exit status: " + str(result.returncode))
            return result

        positive = run("Positive production policy", "cache_tests")
        summary = re.search(r"test result: ok\. (\d+) passed;", positive.stdout)
        if positive.returncode or not summary or int(summary[1]) < 8:
            success = False
        else:
            original = copied.read_text()
            signature = "    fn get_or_create(&mut self, viewport: [f32; 2], create: impl FnOnce() -> T) -> T {"
            start = original.index(signature)
            end = original.index("\n    fn end_flush", start)
            uncached = (
                "    fn get_or_create(&mut self, _viewport: [f32; 2], create: impl FnOnce() -> T) -> T {\n"
                "        create()\n"
                "    }\n"
            )
            try:
                copied.write_text(original[:start] + uncached + original[end:])
                negative = run("Negative uncached viewport policy", "separate_clear_and_scene_flushes_reuse_screen_and_layer_uniforms")
                # The old construction pattern makes four groups per frame
                # across 60 clear/scene pairs. Ensure failure is the asserted
                # regression, rather than compilation or another test failure.
                success = (
                    negative.returncode != 0
                    and "left: 240" in negative.stdout
                    and "right: 2" in negative.stdout
                    and "1 failed" in negative.stdout
                )
            finally:
                copied.write_text(original)

    transcript.append("Validation " + ("PASS" if success else "FAIL"))
    report = "\n".join(transcript) + "\n"
    print(report, end="")
    if args.output:
        args.output.write_text(report)
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
