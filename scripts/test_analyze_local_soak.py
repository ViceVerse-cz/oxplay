# SPDX-License-Identifier: GPL-3.0-or-later
import copy
import unittest

from analyze_local_soak import analyze


def fixture():
    resource = {
        "status": "completed_not_automatically_qualified", "exit_code": 0,
        "forced_termination": False, "sample_count": 3600, "warmup_seconds": 0,
        "samples": [{"elapsed_after_warmup_seconds": second,
                     "rss_mib": 200.0, "cpu_one_core_percent": 40.0} for second in range(1, 3601)],
    }
    summary = {
        "exit_code": 0, "forced_termination": False,
        "observed_owned_processes_remaining": [], "full_spec_soak_gate": False,
        "app_sha256": "a" * 64, "source_revision": "b" * 40, "sample_count": 3600,
    }
    source = {"app_sha256": "a" * 64, "source_revision": "b" * 40}
    lines = []
    for cycle in range(1, 61):
        for phase in (5, 57):
            lines.append(f"local soak checkpoint cycle={cycle} phase={phase} "
                         f"elapsed_ms={((cycle - 1) * 60 + phase) * 1000} "
                         f"loads={1 + (cycle - 1) // 5} load_id=10 position=5.000 "
                         "codec=H.264 / AVC decoder=videotoolbox size=1920x1080 fps=60.000 "
                         "vo_drops=6 decoder_drops=0 events=1 wakes=1 media_notifications=1 "
                         "clock_ticks=1 ui_draws=1 redraw_requests=1 ui_assignments=1 "
                         "model_changes_delta=0 model_resets_delta=0")
    lines.append("local soak finished: completed_cycles=60 elapsed_ms=3603002 "
                 "outcome=local-functional-pass helper_leak_check=external_not_measured")
    return resource, summary, source, "\n".join(lines)


class LocalSoakAnalysisTests(unittest.TestCase):
    def test_complete_local_run_never_grants_full_or_performance_gate(self):
        report = analyze(*fixture())
        self.assertTrue(report["local_functional_evidence_complete"])
        self.assertFalse(report["full_spec_soak_gate"])
        self.assertEqual(report["checkpoint_count"], 120)
        self.assertEqual(report["observed_decoders"], ["videotoolbox"])
        self.assertEqual(report["maximum_vo_drop_counter"], 6)
        self.assertEqual(report["rss_growth_median_percent"], 0)
        self.assertEqual(len(report["rss_matched_minute_comparisons"]), 20)
        self.assertEqual(report["performance_acceptance"], "not_qualified_by_mixed_use_soak")

    def test_shortened_forced_or_surviving_process_run_is_incomplete(self):
        mutations = [
            (0, "sample_count", 3599), (0, "forced_termination", True),
            (0, "exit_code", 1), (0, "status", "incomplete"), (0, "warmup_seconds", 10),
            (1, "exit_code", 1), (1, "failure_type", "TimeoutError"),
            (1, "observed_owned_processes_remaining", ["serein"]),
            (1, "app_sha256", "c" * 64), (1, "source_revision", "c" * 40),
        ]
        for index, key, value in mutations:
            with self.subTest(index=index, key=key):
                args = list(fixture())
                args[index][key] = value
                self.assertFalse(analyze(*args)["local_functional_evidence_complete"])

    def test_checkpoint_and_cleanup_failures_cannot_pass(self):
        changes = [
            ("cycle=2 phase=5", "cycle=1 phase=5"),
            ("model_changes_delta=0", "model_changes_delta=1"),
            ("model_resets_delta=0", "model_resets_delta=1"),
            ("loads=12", "loads=13"),
            ("outcome=local-functional-pass", "outcome=failed"),
            ("elapsed_ms=3603002", "elapsed_ms=3600000"),
            ("completed_cycles=60", "completed_cycles=59"),
            ("phase=5 elapsed_ms=5000", "phase=5 elapsed_ms=15000"),
            ("position=5.000", "position=nan"),
            ("position=5.000", "position=.."),
            ("fps=60.000", "fps=..."),
            ("load_id=10", "load_id=0"),
            ("size=1920x1080", "size=0x1080"),
        ]
        for old, new in changes:
            with self.subTest(old=old):
                resource, summary, source, log = fixture()
                report = analyze(resource, summary, source, log.replace(old, new, 1))
                self.assertFalse(report["local_functional_evidence_complete"])

    def test_numeric_samples_and_order_are_checked(self):
        for bad in (float("nan"), float("inf"), -1, None, True):
            resource, summary, source, log = fixture()
            resource["samples"][0]["rss_mib"] = bad
            self.assertFalse(analyze(resource, summary, source, log)["local_functional_evidence_complete"])
        resource, summary, source, log = fixture()
        resource["samples"][1]["elapsed_after_warmup_seconds"] = 1
        self.assertFalse(analyze(resource, summary, source, log)["local_functional_evidence_complete"])
        resource, summary, source, log = fixture()
        for index, row in enumerate(resource["samples"]):
            row["elapsed_after_warmup_seconds"] = 3599 + index / 3600
        self.assertFalse(analyze(resource, summary, source, log)["local_functional_evidence_complete"])
        resource, summary, source, log = fixture()
        for row in resource["samples"]:
            row["rss_mib"] = 0
        self.assertFalse(analyze(resource, summary, source, log)["local_functional_evidence_complete"])

    def test_extra_failures_or_malformed_terminal_lines_are_not_ignored(self):
        for extra in ("local soak finished: failed malformed terminal outcome",
                      "local soak FAILED cycle=10 phase=57: fixture failure"):
            resource, summary, source, log = fixture()
            self.assertFalse(analyze(resource, summary, source, log + "\n" + extra)["local_functional_evidence_complete"])
        resource, summary, source, log = fixture()
        # Both are within their individual five-second tolerances, but overlap.
        log = log.replace("elapsed_ms=57000", "elapsed_ms=62000").replace("elapsed_ms=65000", "elapsed_ms=61000")
        self.assertFalse(analyze(resource, summary, source, log)["local_functional_evidence_complete"])

    def test_growth_reports_every_matched_minute_without_cherry_picking(self):
        resource, summary, source, log = fixture()
        original = copy.deepcopy(resource)
        for row in resource["samples"]:
            if row["elapsed_after_warmup_seconds"] > 2400:
                row["rss_mib"] = 240
        report = analyze(resource, summary, source, log)
        self.assertAlmostEqual(report["rss_growth_median_percent"], 20)
        self.assertEqual(report["rss_pairs_above_ten_percent"], 20)
        self.assertEqual([p["earlier_minute"] for p in report["rss_matched_minute_comparisons"]], list(range(11, 31)))
        self.assertEqual([p["later_minute"] for p in report["rss_matched_minute_comparisons"]], list(range(41, 61)))
        self.assertEqual(original["samples"][0], resource["samples"][0])
        self.assertFalse(report["full_spec_soak_gate"])


if __name__ == "__main__":
    unittest.main()
