#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Prerequisite probes and isolated C frame policy; no GPU/dependency builds."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("native_media_macos", Path(__file__).parent / "native-media/macos.py")
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class NativePrerequisiteTests(unittest.TestCase):
    def test_disabled_vulkan_stubs_require_bounded_header_probe(self):
        env = {"CFLAGS": '-I"/private/synthetic include" -DMY_FLAG=1'}
        with patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "", "")) as run:
            evidence = native.check_vulkan_headers(env)
        self.assertTrue(evidence["available"])
        self.assertFalse(evidence["vulkan_backend_enabled"])
        args = run.call_args.args[0]
        self.assertIn("-I/private/synthetic include", args)
        self.assertIn("-fsyntax-only", args)
        self.assertNotIn("-o", args)
        self.assertEqual(run.call_args.kwargs["input"], "#include <vulkan/vulkan.h>\n")
        self.assertEqual(run.call_args.kwargs["timeout"], 10)

    def test_missing_headers_report_prerequisite_without_compiler_details(self):
        with patch.object(native.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "private-token")):
            with self.assertRaisesRegex(RuntimeError, "install vulkan-headers") as raised:
                native.check_vulkan_headers({})
        self.assertNotIn("private-token", str(raised.exception))

    def test_header_probe_timeout_is_finite(self):
        with patch.object(native.subprocess, "run", side_effect=subprocess.TimeoutExpired("synthetic", 10)):
            with self.assertRaisesRegex(RuntimeError, "10-second limit"):
                native.check_vulkan_headers({})


class NativeFrameCachePolicyTests(unittest.TestCase):
    """Execute the patched C policy, without duplicating it in Python."""

    @classmethod
    def setUpClass(cls):
        compiler = shutil.which("cc")
        if not compiler:
            raise unittest.SkipTest("C compiler unavailable for isolated policy check")
        patch_text = (Path(__file__).parent /
                      "native-media/patches/common-mpv-gpu-next.patch").read_text()
        start = patch_text.index("+    bool will_redraw = frame &&")
        end = patch_text.index("-    if (!pl_render_image_mix", start)
        policy = "\n".join(line[1:] for line in patch_text[start:end].splitlines())
        cls.temporary = tempfile.TemporaryDirectory(prefix="oxplay-frame-cache-test-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.binary = Path(cls.temporary.name) / "policy"
        # Stand-ins cover only the fields used by this exact patched block.
        # A sentinel ensures all other rendering settings remain untouched.
        source = """#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
struct vo_frame { bool display_synced, still, redraw, repeat; int num_vsyncs; };
struct render_params { bool skip_caching_single_frame; const void *frame_mixer; int scaler; };
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int flags = atoi(argv[1]);
    struct vo_frame input = {
        .display_synced = flags & 1, .still = flags & 2,
        .redraw = flags & 4, .repeat = flags & 8,
        .num_vsyncs = flags & 16 ? 2 : 1,
    };
    struct vo_frame *frame = flags & 32 ? NULL : &input;
    int mixer;
    struct render_params rparams = { .frame_mixer = &mixer, .scaler = 73 };
""" + policy + """
    printf("%d %d %d\\n", rparams.skip_caching_single_frame,
           rparams.frame_mixer == &mixer, rparams.scaler);
    return 0;
}
"""
        subprocess.run([compiler, "-std=c11", "-Wall", "-Wextra", "-Werror",
                        "-x", "c", "-o", str(cls.binary), "-"],
                       input=source, text=True, check=True, capture_output=True, timeout=10)

    def result(self, flags):
        return tuple(map(int, subprocess.check_output(
            [str(self.binary), str(flags)], text=True, timeout=2).split()))

    def test_fresh_video_bypasses_mixer_cache_and_keeps_scaler_and_mixer(self):
        for flags in (0, 1, 16):
            with self.subTest(flags=flags):
                self.assertEqual(self.result(flags), (1, 1, 73))

    def test_paused_still_frames_cache_and_disable_mixing(self):
        for flags in (2, 2 | 4, 2 | 8, 2 | 1 | 16):
            with self.subTest(flags=flags):
                self.assertEqual(self.result(flags), (0, 0, 73))

    def test_host_redraw_repeat_and_multiple_vsyncs_keep_cache(self):
        for flags in (4, 8, 4 | 8, 1 | 16):
            with self.subTest(flags=flags):
                self.assertEqual(self.result(flags), (0, 1, 73))

    def test_absent_frame_does_not_dereference_or_change_render_options(self):
        self.assertEqual(self.result(32), (1, 1, 73))


if __name__ == "__main__":
    unittest.main()
