#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Audit a completed 60-minute local diagnostic; never grant the full SPEC gate.

Run after measurement, not concurrently with it. Reads the four bounded evidence
files from the soak harness and prints a path-free report. Raw evidence remains
authoritative. RSS is sampled process RSS, not unique physical or GPU memory.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import statistics

CHECKPOINT = re.compile(
    r"^local soak checkpoint cycle=(\d+) phase=(\d+) elapsed_ms=(\d+) "
    r"loads=(\d+) load_id=(\d+) position=([\d.]+) codec=(.*?) "
    r"decoder=(\S+) size=(\d+)x(\d+) fps=([\d.]+) vo_drops=(\d+) "
    r"decoder_drops=(\d+) events=(\d+) wakes=(\d+) media_notifications=(\d+) "
    r"clock_ticks=(\d+) ui_draws=(\d+) redraw_requests=(\d+) ui_assignments=(\d+) "
    r"model_changes_delta=(\d+) model_resets_delta=(\d+)$", re.MULTILINE)
FINISH = re.compile(
    r"^local soak finished: completed_cycles=(\d+) elapsed_ms=(\d+) "
    r"outcome=([\w-]+) helper_leak_check=external_not_measured$", re.MULTILINE)


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def analyze(resource, summary, source, log):
    if not all(isinstance(value, dict) for value in (resource, summary, source)) or not isinstance(log, str):
        raise ValueError("invalid evidence structure")
    failures = []

    def require(condition, reason):
        if not condition:
            failures.append(reason)

    require(resource.get("status") == "completed_not_automatically_qualified", "resource sampling incomplete")
    require(resource.get("exit_code") == 0 and resource.get("forced_termination") is False, "application did not exit cleanly")
    require(summary.get("exit_code") == 0 and summary.get("forced_termination") is False
            and "failure_type" not in summary, "outer harness did not finish cleanly")
    require(summary.get("observed_owned_processes_remaining") == [], "observed helper survivors or missing audit")
    require(summary.get("full_spec_soak_gate") is False, "local scope was not declared")
    require(summary.get("app_sha256") == source.get("app_sha256")
            and bool(re.fullmatch(r"[a-f0-9]{64}", source.get("app_sha256", ""))), "binary evidence identity mismatch")
    require(summary.get("source_revision") == source.get("source_revision")
            and bool(re.fullmatch(r"[a-f0-9]{40}", source.get("source_revision", ""))), "source evidence identity mismatch")
    require(resource.get("warmup_seconds") == 0, "unexpected sampler warmup")
    samples = resource.get("samples", [])
    require(isinstance(samples, list) and len(samples) == 3600
            and resource.get("sample_count") == 3600 and summary.get("sample_count") == 3600,
            "not exactly 3600 resource observations")
    rows_valid = isinstance(samples, list) and all(
        isinstance(row, dict) and all(finite(row.get(key)) and row[key] >= 0
            for key in ("rss_mib", "cpu_one_core_percent", "elapsed_after_warmup_seconds")) for row in samples)
    require(rows_valid, "invalid numeric resource observation")
    if rows_valid:
        require(all(row["rss_mib"] > 0 for row in samples), "missing resident-memory observations")
        require(all(a["elapsed_after_warmup_seconds"] < b["elapsed_after_warmup_seconds"]
                    for a, b in zip(samples, samples[1:])), "resource times are not increasing")
        require(bool(samples) and 3599 <= samples[-1]["elapsed_after_warmup_seconds"] <= 3605,
                "resource duration differs from one hour")
        require(all(abs(row["elapsed_after_warmup_seconds"] - index) <= 2
                    for index, row in enumerate(samples, 1)), "resource sampling cadence exceeded two-second allowance")

    points = list(CHECKPOINT.finditer(log))
    require([(int(p[1]), int(p[2])) for p in points] == [(cycle, phase) for cycle in range(1, 61) for phase in (5, 57)],
            "missing, duplicate or unordered playing checkpoints")
    require(len(re.findall(r"^local soak checkpoint ", log, re.MULTILINE)) == len(points), "unparsed checkpoint")
    for point in points:
        cycle, phase, elapsed, loads = map(int, point.group(1, 2, 3, 4))
        expected_ms = ((cycle - 1) * 60 + phase) * 1000
        require(abs(elapsed - expected_ms) <= 5000, "checkpoint timing exceeded diagnostic allowance")
        require(loads == 1 + (cycle - 1) // 5, "unexpected fixture reload count")
        require((int(point[21]), int(point[22])) == (0, 0), "unrelated catalog changes or resets")
        try:
            position, fps = float(point[6]), float(point[11])
        except ValueError:
            position = fps = float("nan")
        require(finite(position) and position >= 0 and finite(fps) and fps > 0
                and all(int(point[index]) > 0 for index in (5, 9, 10)), "invalid active-video checkpoint")
    require(all(int(a[3]) < int(b[3]) for a, b in zip(points, points[1:])), "checkpoint times are not increasing")
    finishes = list(FINISH.finditer(log))
    require(len(re.findall(r"^local soak finished:", log, re.MULTILINE)) == len(finishes), "unparsed terminal outcome")
    require(not re.search(r"^local soak .*?(?:FAILED|failed)", log, re.MULTILINE), "recorded local soak failure")
    require(len(finishes) == 1 and int(finishes[0][1]) == 60
            and 3603000 <= int(finishes[0][2]) <= 3620000
            and finishes[0][3] == "local-functional-pass", "full-duration terminal cleanup did not pass")

    # The sampler and app have separate clock origins. Whole-minute medians are
    # an approximate repeated-workload trend, never exact phase-57 observations.
    # Compare minutes 11–30 with 41–60, after ten-minute warmup. A thirty-minute
    # offset preserves five-minute reload, odd/even fullscreen AND three-minute
    # paused-seek target phases. Use every pair rather than favorable endpoints.
    minutes = []
    if rows_valid:
        for minute in range(1, 61):
            selected = [row for row in samples if (minute - 1) * 60 < row["elapsed_after_warmup_seconds"] <= minute * 60]
            minutes.append({"minute": minute, "sample_count": len(selected),
                            "rss_median_mib": statistics.median(row["rss_mib"] for row in selected) if selected else None})
        require(all(58 <= minute["sample_count"] <= 62 for minute in minutes), "incomplete whole-minute resource coverage")
    comparisons = []
    if len(minutes) == 60:
        for early, late in zip(minutes[10:30], minutes[40:60]):
            a, b = early["rss_median_mib"], late["rss_median_mib"]
            if a and b is not None:
                comparisons.append({"earlier_minute": early["minute"], "later_minute": late["minute"],
                                    "growth_percent": 100 * (b / a - 1)})
    growth = [row["growth_percent"] for row in comparisons]
    require(len(comparisons) == 20, "missing matched-minute RSS comparisons")
    mixed_metrics = {}
    if rows_valid and samples:
        for key in ("rss_mib", "cpu_one_core_percent"):
            values = sorted(row[key] for row in samples)
            mixed_metrics[key] = {"mean": statistics.mean(values),
                                  "p95": values[math.ceil(len(values) * 0.95) - 1], "peak": values[-1]}
    return {
        "local_functional_evidence_complete": not failures,
        "evidence_failures": sorted(set(failures)),
        "full_spec_soak_gate": False,
        "performance_acceptance": "not_qualified_by_mixed_use_soak",
        "mixed_use_resource_observations": mixed_metrics,
        "source_revision": source.get("source_revision"), "app_sha256": source.get("app_sha256"),
        "source_scope": "identity-label consistency only; captured build-input hashes require a separate comparison with Git",
        "checkpoint_count": len(points),
        "observed_decoders": sorted({p[8] for p in points}),
        "observed_video": sorted({f"{p[9]}x{p[10]}@{p[11]} {p[7]}" for p in points}),
        "maximum_decoder_drop_counter": max((int(p[13]) for p in points), default=None),
        "maximum_vo_drop_counter": max((int(p[12]) for p in points), default=None),
        "drop_counter_scope": "transition checkpoints; counters reset on seeks/loads, not a cumulative or steady frame-pacing result",
        "rss_minute_medians": minutes,
        "rss_matched_minute_comparisons": comparisons,
        "rss_growth_median_percent": statistics.median(growth) if growth else None,
        "rss_pairs_above_ten_percent": sum(value > 10 for value in growth),
        "rss_trend_scope": "approximate whole-minute alignment; distinct app/sampler clocks; investigate growth, do not certify absence of leaks",
        "memory_scope": resource.get("notes"),
        "helper_audit_scope": summary.get("helper_audit_scope"),
        "remaining_scope": ["search/cancellation and extractor helpers", "authorized account operations and expiry",
                            "forced decoder fallback", "large raster library", "steady playback and idle budgets",
                            "exact GPU allocation/leak accounting", "other target platforms"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    inputs = {}
    hashes = {}
    for name in ("resource.json", "summary.json", "source.json", "resource.json.log"):
        try:
            fd = os.open(args.directory / name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            with os.fdopen(fd, "rb") as file:
                if not stat.S_ISREG(os.fstat(file.fileno()).st_mode):
                    parser.error("evidence input is not a regular file")
                data = file.read(16 * 1024 * 1024 + 1)
        except OSError:
            parser.error("evidence input is unavailable")
        if len(data) > 16 * 1024 * 1024:
            parser.error("evidence input exceeds the analysis bound")
        hashes[name] = hashlib.sha256(data).hexdigest()
        try:
            inputs[name] = data.decode("utf-8") if name.endswith(".log") else json.loads(data)
        except (UnicodeError, ValueError):
            parser.error("evidence input is not valid UTF-8 or JSON")
    try:
        report = analyze(inputs["resource.json"], inputs["summary.json"], inputs["source.json"], inputs["resource.json.log"])
    except (ValueError, TypeError, AttributeError, OverflowError):
        parser.error("evidence has an invalid structure")
    report["evidence_sha256"] = hashes
    print(json.dumps(report, indent=2))
    raise SystemExit(0 if report["local_functional_evidence_complete"] else 1)


if __name__ == "__main__":
    main()
