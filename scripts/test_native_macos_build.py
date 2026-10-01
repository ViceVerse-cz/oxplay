#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic prerequisite probes; no compiler, GPU or dependency build runs."""
import importlib.util
from pathlib import Path
import subprocess
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


if __name__ == "__main__":
    unittest.main()
