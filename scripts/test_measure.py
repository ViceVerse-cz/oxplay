#!/usr/bin/env python3
"""Deterministic measurement attribution/privacy regression tests; no live players."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
import io
import ctypes
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measure


def row(parent, cpu, name, rss=1024):
    return parent, rss, cpu, name


def fixture_marker(**changes):
    fields = dict.fromkeys(measure.FIXTURE_NUMBERS, 0)
    fields.update(elapsed_ms=450, generation=3, model_rows=100,
                  near_viewport_end=9, near_viewport_ready=9,
                  viewport_intersecting_ready_thumbnails=6, started=9, decoded=9,
                  published=9, published_bytes=2073600, catalog_changes=9,
                  catalog_resets=1, group_child_changes=9, hidden="false",
                  compositor_visibility="not_measured")
    fields.update(changes)
    return measure.FIXTURE_PREFIX + " ".join(f"{key}={value}" for key, value in fields.items()).encode()


class MeasurementTests(unittest.TestCase):
    def test_footprint_v0_matches_reviewed_sdk_layout_and_checks_kernel_result(self):
        self.assertEqual(ctypes.sizeof(measure.RusageInfoV0), 96)
        self.assertEqual(measure.RusageInfoV0.ri_phys_footprint.offset, 72)
        self.assertEqual(measure.RusageInfoV0.ri_proc_start_abstime.offset, 80)
        self.assertEqual(measure.RusageInfoV0.ri_proc_exit_abstime.offset, 88)
        reader = measure.MacosFootprint.__new__(measure.MacosFootprint)
        def query(pid, flavor, pointer):
            self.assertEqual((pid, flavor), (42, 0))
            result = ctypes.cast(pointer, ctypes.POINTER(measure.RusageInfoV0)).contents
            result.ri_phys_footprint = 125 * 1048576
            result.ri_proc_start_abstime = 1234
            return 0
        reader.query = query
        self.assertEqual(reader.read(42), {"available": True, "physical_footprint_mib": 125,
                                          "process_start_abstime": 1234})
        def denied(*_):
            ctypes.set_errno(13)
            return -1
        reader.query = denied
        self.assertEqual(reader.read(42), {"available": False, "errno": 13})
        reader.query = lambda *_: 0
        self.assertEqual(reader.read(42), {"available": False, "reason": "missing_live_process_identity"})
        with patch.object(measure.sys, "platform", "linux"), self.assertRaises(RuntimeError):
            measure.MacosFootprint()

    def test_footprint_separates_charge_categories_without_changing_rss_or_discovering_processes(self):
        current = {42: row(1, 0, "/private/app", 2048),
                   43: row(42, 0, "/private/worker", 1024),
                   44: row(43, 0, "/private/child", 512),
                   99: row(1, 0, "/System/VTDecoderXPCService", 4096)}
        reader = Mock()
        reader.read.side_effect = lambda pid: {"available": True, "physical_footprint_mib": pid,
                                               "process_start_abstime": pid * 100}
        with patch.object(measure, "process_table", side_effect=AssertionError("extra discovery")):
            result = measure.footprint_audit(42, current, reader)
        self.assertTrue(result["complete"])
        self.assertEqual(result["aggregate_physical_footprint_mib"], 228)
        self.assertEqual(result["observed_totals_mib"], {"root": 42, "descendant": 87, "temporal_vt_service": 99})
        self.assertEqual(sum(entry[1] for entry in current.values()), 7680)
        self.assertNotIn("private", json.dumps(result))
        self.assertEqual([call.args[0] for call in reader.read.call_args_list], [42, 43, 44, 99])

    def test_footprint_denied_or_omitted_process_never_becomes_zero_aggregate(self):
        current = {42: row(1, 0, "app"), 43: row(42, 0, "helper")}
        reader = Mock()
        reader.read.side_effect = lambda pid: ({"available": True, "physical_footprint_mib": 100,
                                                "process_start_abstime": 1} if pid == 42
                                               else {"available": False, "errno": 13})
        for limit in (1, 64):
            result = measure.footprint_audit(42, current, reader, limit)
            self.assertFalse(result["complete"])
            self.assertIsNone(result["aggregate_physical_footprint_mib"])
            self.assertEqual(result["observed_totals_mib"]["root"], 100)
            self.assertEqual(result["omitted_count"], 1 if limit == 1 else 0)
        self.assertFalse(measure.footprint_audit(42, {}, reader)["complete"])
        self.assertFalse(measure.footprint_audit(99, current, reader)["complete"])
        with self.assertRaises(ValueError):
            measure.footprint_audit(42, current, reader, 0)

    def test_optional_footprint_summarizes_only_complete_coverage(self):
        for available in (True, False):
            with self.subTest(available=available), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "sample.json"
                process = Mock(pid=42, returncode=0)
                process.poll.return_value = None
                reader = Mock()
                reader.read.return_value = ({"available": True, "physical_footprint_mib": 80,
                                              "process_start_abstime": 10} if available
                                             else {"available": False, "errno": 13})
                args = ["measure.py", "--macos-footprint", "--warmup", "0", "--seconds", "1",
                        "--output", str(output), "--", "dummy"]
                with patch.object(sys, "argv", args), \
                        patch.object(measure, "MacosFootprint", return_value=reader), \
                        patch.object(measure, "process_table", return_value={42: row(1, 0, "app")}) as scan, \
                        patch.object(measure.subprocess, "Popen", return_value=process), \
                        patch.object(measure.time, "sleep"), patch("builtins.print"):
                    measure.main()
                result = json.loads(output.read_text())
                scan.assert_called()
                self.assertEqual(scan.call_count, 3)
                reader.read.assert_called_once_with(42)
                self.assertEqual(result["rss_mib"]["mean"], 1)
                self.assertEqual(result["macos_footprint"]["all_samples_complete"], available)
                self.assertEqual(result["macos_footprint"]["incomplete_sample_count"], 0 if available else 1)
                if available:
                    self.assertEqual(result["macos_footprint"]["physical_footprint_mib"]["mean"], 80)
                else:
                    self.assertIsNone(result["macos_footprint"]["physical_footprint_mib"])

    def test_fixture_marker_requires_drained_current_bounded_images_without_visibility_claim(self):
        result = measure.parse_marker(fixture_marker())
        self.assertEqual(result["viewport_intersecting_ready_thumbnails"], 6)
        self.assertFalse(result["hidden"])
        self.assertIn("unmeasured", result["scope"])
        self.assertIsNone(measure.parse_marker(b"unrelated private message"))
        for changed in ({"pending": 1}, {"inflight": 1}, {"ready": 1},
                        {"remote_started": 1}, {"failed": 1}, {"generation": 0},
                        {"model_rows": 101}, {"near_viewport_first": 10},
                        {"near_viewport_ready": 8}, {"viewport_intersecting_ready_thumbnails": 0},
                        {"viewport_intersecting_ready_thumbnails": 10},
                        {"hidden": "unknown"}, {"compositor_visibility": "yes"},
                        {"published": -1}, {"decoded": "NaN"}):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                measure.parse_marker(fixture_marker(**changed))
        with self.assertRaises(ValueError):
            measure.parse_marker(fixture_marker() + b" pending=0")
        with self.assertRaises(ValueError):
            measure.parse_marker(fixture_marker().replace(b"generation=3", b"generation-missing=3"))

    def test_fixture_wait_reads_incrementally_and_never_exports_raw_log(self):
        class Log(io.BytesIO):
            def read(self, size=-1):
                return super().read(min(size, 37))
        stream = Log(b"private path /not/exported\n" + fixture_marker() + b"\n")
        process = Mock()
        process.poll.return_value = None
        with patch("builtins.open", return_value=stream), patch.object(measure.time, "sleep") as sleep:
            result = measure.wait_for_fixture("private-path", process)
        self.assertEqual(result["generation"], 3)
        self.assertNotIn("private", json.dumps(result))
        sleep.assert_not_called()

    def test_fixture_wait_rejects_exit_deadline_unterminated_and_oversized_logs(self):
        process = Mock()
        process.poll.return_value = 0
        with patch("builtins.open", return_value=io.BytesIO(fixture_marker() + b"\n")):
            with self.assertRaises(RuntimeError):
                measure.wait_for_fixture("unused", process)
        process.poll.return_value = None
        with patch("builtins.open", return_value=io.BytesIO()), \
                patch.object(measure.time, "monotonic", side_effect=[0, 31]):
            with self.assertRaises(TimeoutError):
                measure.wait_for_fixture("unused", process)
        for content in (b"x" * 4097, b"x" * 4097 + b"\n",
                        b"short\n" * (measure.FIXTURE_LIMIT // 6 + 1)):
            with patch("builtins.open", return_value=io.BytesIO(content)), self.assertRaises(ValueError):
                measure.wait_for_fixture("unused", process)
        for timeout in (0, -1, 31):
            with self.assertRaises(ValueError):
                measure.wait_for_fixture("unused", process, timeout)

    def test_fixture_wait_requires_complete_line_and_live_process_at_admission(self):
        process = Mock()
        process.poll.side_effect = [None, 0]
        with patch("builtins.open", return_value=io.BytesIO(fixture_marker() + b"\n")):
            with self.assertRaises(RuntimeError):
                measure.wait_for_fixture("unused", process)
        process.poll.side_effect = None
        process.poll.return_value = None
        with patch("builtins.open", return_value=io.BytesIO(fixture_marker())), \
                patch.object(measure.time, "monotonic", side_effect=[0, 0, 31]):
            with self.assertRaises(TimeoutError):
                measure.wait_for_fixture("unused", process)
        with patch("builtins.open", return_value=io.BytesIO(fixture_marker() + b"\n")), \
                patch.object(measure.time, "monotonic", side_effect=[0, 0, 31]):
            with self.assertRaises(TimeoutError):
                measure.wait_for_fixture("unused", process)

    def test_fixture_admission_precedes_warmup_and_is_recorded_without_extra_steady_polling(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "sample.json"
            process = Mock(pid=42, returncode=0)
            process.poll.return_value = None
            order = []
            args = ["measure.py", "--library-fixture-ready", "--warmup", "10", "--seconds", "1",
                    "--output", str(output), "--", "fixture-app"]
            marker = measure.parse_marker(fixture_marker())
            def ready(*_):
                order.append("ready")
                return marker
            with patch.object(sys, "argv", args), \
                    patch.object(measure, "process_table", return_value={}), \
                    patch.object(measure.subprocess, "Popen", return_value=process), \
                    patch.object(measure, "wait_for_fixture", side_effect=ready) as wait, \
                    patch.object(measure.time, "sleep", side_effect=lambda value: order.append(value)), \
                    patch("builtins.print"):
                measure.main()
            self.assertEqual(order[:2], ["ready", 10])
            wait.assert_called_once_with(str(output) + ".log", process)
            result = json.loads(output.read_text())
            self.assertEqual(result["library_fixture_readiness"], marker)
            self.assertEqual(result["sample_count"], 1)

    def test_owned_descendants_and_new_decoder_are_separate_from_foreign_compiler(self):
        previous = {10: row(1, 1, "/app/serein"), 11: row(10, 2, "/helper/yt-dlp"),
                    12: row(11, 3, "/helper/deno"), 20: row(1, 9, "/sys/VTDecoderXPCService"),
                    21: row(1, 4, "/sys/VTDecoderXPCService"), 30: row(1, 10, "/other/rustc")}
        current = {pid: (r[0], r[1], r[2] + 1, r[3]) for pid, r in previous.items()}
        with patch.object(measure, "process_table", side_effect=AssertionError("extra ps scan")):
            owned = measure.snapshot(10, True, {10, 20, 30}, current)
        self.assertEqual(set(owned), {10, 11, 12, 21})
        self.assertEqual(measure.cpu_delta(owned, previous), 4)
        other = measure.host_other_cpu(current, previous, owned, 2)
        self.assertEqual(other["host_other_cpu_one_core_percent"], 100)
        self.assertEqual({r["executable"] for r in other["host_other_top_cpu"]},
                         {"VTDecoderXPCService", "rustc"})
        self.assertEqual(set(measure.snapshot(10, False, set(), current)), {10, 11, 12})

    def test_audit_exports_basenames_groups_names_and_bounds_list_without_losing_total(self):
        previous = {pid: row(1, 0, f"/private/profile-{pid}/tool-{pid}") for pid in range(12)}
        current = {pid: row(1, pid + 1, old[3]) for pid, old in previous.items()}
        current[12] = row(1, 2, "/a directory with spaces/tool-11")
        previous[12] = row(1, 0, current[12][3])
        result = measure.host_other_cpu(current, previous, set(), 1)
        self.assertEqual(len(result["host_other_top_cpu"]), 8)
        self.assertEqual(result["host_other_cpu_one_core_percent"], 8000)
        self.assertEqual(result["host_other_top_cpu"][0],
                         {"executable": "tool-11", "cpu_one_core_percent": 1400})
        encoded = json.dumps(result)
        self.assertNotIn("/", encoded)
        self.assertNotIn("profile", encoded)
        self.assertNotIn("directory", encoded)

    def test_missing_reset_or_changed_executable_counters_never_create_false_cpu(self):
        old = {1: row(0, 100, "tool"), 2: row(0, 1, "old"), 4: row(0, 50, "exited")}
        new = {1: row(0, 2, "tool"), 2: row(0, 5, "new"), 3: row(0, 500, "new-process")}
        self.assertEqual(measure.cpu_delta(new, old), 0)
        self.assertEqual(measure.host_other_cpu(new, old, set(), 1),
                         {"host_other_cpu_one_core_percent": 0, "host_other_top_cpu": []})
        with self.assertRaises(ValueError):
            measure.host_other_cpu(new, old, set(), 0)

    def test_cpu_time_days_and_percentile_conventions(self):
        self.assertEqual(measure.cpu_seconds("1-02:03:04.50"), 93784.5)
        self.assertEqual(measure.cpu_seconds("02:03.25"), 123.25)
        self.assertEqual(measure.statistics(list(range(1, 21))),
                         {"mean": 10.5, "p95": 19, "peak": 20})

    def test_process_rss_separates_root_nested_child_and_temporal_service_without_extra_scan(self):
        current = {10: row(1, 0, "/private/root/serein", 2048),
                   11: row(10, 0, "/private/child/worker", 1024),
                   12: row(11, 0, "/another/worker", 512),
                   20: row(1, 0, "/System/VTDecoderXPCService", 4096)}
        with patch.object(measure, "process_table", side_effect=AssertionError("extra ps scan")):
            result = measure.process_rss_audit(10, current)
        self.assertEqual(result["process_rss_totals_mib"],
                         {"root": 2, "descendant": 1.5, "temporal_vt_service": 4})
        by_pid = {r["pid"]: r for r in result["process_rss"]}
        self.assertEqual(by_pid[12]["attribution"], "descendant")
        self.assertEqual(by_pid[20]["attribution"], "temporal_vt_service")
        self.assertEqual(len([r for r in by_pid.values() if r["executable"] == "worker"]), 2)
        self.assertEqual(sum(r["rss_mib"] for r in by_pid.values()), 7.5)
        self.assertNotIn("/", json.dumps(result))
        self.assertEqual(result["process_rss_omitted_count"], 0)

    def test_process_rss_bounds_rows_but_preserves_all_category_and_omitted_bytes(self):
        current = {10: row(1, 0, "/private/app", 1)}
        current.update({pid: row(10, 0, "/private/" + "x" * 100, pid * 1024)
                        for pid in range(11, 100)})
        result = measure.process_rss_audit(10, current)
        self.assertEqual(len(result["process_rss"]), 64)
        self.assertEqual(result["process_rss"][0]["pid"], 10)
        self.assertEqual(result["process_rss_omitted_count"], len(current) - 64)
        self.assertTrue(all(len(row["executable"]) <= 80 for row in result["process_rss"]))
        total = sum(result["process_rss_totals_mib"].values())
        visible = sum(row["rss_mib"] for row in result["process_rss"])
        self.assertEqual(total, visible + result["process_rss_omitted_mib"])
        with self.assertRaises(ValueError):
            measure.process_rss_audit(10, current, 0)

    def test_optional_rss_output_preserves_default_schema_and_uses_existing_sample_table(self):
        for enabled, existing_service in ((False, False), (True, False), (True, True)):
            with self.subTest(enabled=enabled, existing_service=existing_service), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "sample.json"
                process = Mock(pid=42, returncode=0)
                process.poll.return_value = None
                table = {42: row(1, 0, "/private/app", 2048),
                         43: row(1, 0, "/System/VTDecoderXPCService", 1024)}
                baseline = {43: table[43]} if existing_service else {}
                args = ["measure.py", "--warmup", "0", "--seconds", "1",
                        "--include-new-vt-services", "--output", str(output)]
                if enabled:
                    args.append("--per-process-rss")
                args.extend(["--", "dummy-player"])
                with patch.object(sys, "argv", args), \
                        patch.object(measure, "process_table", side_effect=[baseline, table, table]) as scan, \
                        patch.object(measure.subprocess, "Popen", return_value=process), \
                        patch.object(measure.time, "sleep"), \
                        patch("builtins.print"):
                    measure.main()
                result = json.loads(output.read_text())
                self.assertEqual(scan.call_count, 3)  # baseline, warmup, existing sample
                self.assertEqual(result["samples"][0]["rss_mib"], 2 if existing_service else 3)
                self.assertEqual(result["status"], "completed_not_automatically_qualified")
                self.assertEqual("per_process_rss" in result, enabled)
                self.assertEqual("process_rss" in result["samples"][0], enabled)
                if enabled:
                    self.assertEqual(result["samples"][0]["process_rss_totals_mib"],
                                     {"root": 2, "descendant": 0,
                                      "temporal_vt_service": 0 if existing_service else 1})
                    self.assertEqual(result["per_process_rss"]["baseline_vt_service_count"],
                                     int(existing_service))
                    if existing_service:
                        self.assertEqual(result["per_process_rss"]["baseline_vt_services"],
                                         [{"pid": 43, "executable": "VTDecoderXPCService"}])

    def test_failed_measurement_preserves_partial_evidence_without_exception_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "failed.json"
            process = Mock(pid=42, returncode=0)
            args = ["measure.py", "--warmup", "0", "--seconds", "2", "--output", str(output),
                    "--", "dummy-player"]
            with patch.object(sys, "argv", args), \
                    patch.object(measure, "process_table", return_value={}), \
                    patch.object(measure.subprocess, "Popen", return_value=process), \
                    patch.object(measure.time, "sleep"), \
                    patch.object(measure, "media_snapshot",
                                 side_effect=TimeoutError("private socket path")), \
                    patch("builtins.print"):
                with self.assertRaises(TimeoutError):
                    measure.main()
            result = json.loads(output.read_text())
            self.assertEqual(result["status"], "incomplete")
            self.assertEqual(result["failure_type"], "TimeoutError")
            self.assertEqual(result["sample_count"], 0)
            self.assertEqual(result["samples"], [])
            self.assertNotIn("per_process_rss", result)
            self.assertNotIn("private", output.read_text())
            process.wait.assert_called_once_with(timeout=20)

    def test_cleanup_failure_also_writes_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "failed.json"
            process = Mock(pid=42, returncode=None)
            process.wait.side_effect = OSError("private process detail")
            args = ["measure.py", "--warmup", "0", "--output", str(output), "--", "dummy"]
            with patch.object(sys, "argv", args), \
                    patch.object(measure, "process_table", return_value={}), \
                    patch.object(measure.subprocess, "Popen", return_value=process), \
                    patch.object(measure.time, "sleep"), \
                    patch.object(measure, "media_snapshot", side_effect=TimeoutError()), \
                    patch("builtins.print"):
                with self.assertRaises(TimeoutError):
                    measure.main()
            result = json.loads(output.read_text())
            self.assertEqual(result["status"], "incomplete")
            self.assertEqual(result["cleanup_failure_type"], "OSError")
            self.assertNotIn("private", output.read_text())


if __name__ == "__main__":
    unittest.main()
