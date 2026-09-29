#!/usr/bin/env python3
"""Measure RSS and CPU-time deltas for an owned process tree.

100% CPU means one logical core. Very short-lived processes may escape sampling.
Optional newly launched VideoToolbox services use temporal attribution, which is
not proof of exclusive ownership. Raw commands/provider data are not exported.
Optional per-process RSS uses the same ps sample, not an additional scan.
"""
import argparse
import ctypes
import json
import math
import re
from pathlib import Path
import socket
import subprocess
import sys
import time

FIXTURE_PREFIX = b"library fixture quiescent: "
FIXTURE_LIMIT = 256 * 1024
FIXTURE_LINE_LIMIT = 4096
FIXTURE_NUMBERS = ("elapsed_ms", "generation", "model_rows", "near_viewport_first",
           "near_viewport_end", "near_viewport_ready",
           "viewport_intersecting_ready_thumbnails", "pending", "inflight", "ready",
           "remote_started", "started", "decoded", "failed", "published",
           "published_bytes", "catalog_changes", "catalog_resets", "group_child_changes")


class RusageInfoV0(ctypes.Structure):
    # Exact RUSAGE_INFO_V0 layout in macOS SDK sys/resource.h. Do not use the
    # moving RUSAGE_INFO_CURRENT flavor with a smaller buffer.
    _fields_ = [("ri_uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64) for name in (
            "ri_user_time", "ri_system_time", "ri_pkg_idle_wkups", "ri_interrupt_wkups",
            "ri_pageins", "ri_wired_size", "ri_resident_size", "ri_phys_footprint",
            "ri_proc_start_abstime", "ri_proc_exit_abstime")]


class MacosFootprint:
    """Read the OS physical-footprint ledger; this is distinct from RSS/GPU bytes."""
    def __init__(self):
        if sys.platform != "darwin":
            raise RuntimeError("macOS physical footprint is unavailable on this platform")
        self.library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.query = self.library.proc_pid_rusage
        self.query.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.query.restype = ctypes.c_int

    def read(self, pid):
        result = RusageInfoV0()
        ctypes.set_errno(0)
        if self.query(pid, 0, ctypes.byref(result)) != 0:
            return {"available": False, "errno": ctypes.get_errno()}
        if result.ri_proc_start_abstime == 0 or result.ri_proc_exit_abstime != 0:
            return {"available": False, "reason": "missing_live_process_identity"}
        return {"available": True, "physical_footprint_mib": result.ri_phys_footprint / 1048576,
                "process_start_abstime": result.ri_proc_start_abstime}


def footprint_audit(root, current, reader, limit=64):
    """Use only PIDs from the already attributed ps snapshot; no discovery scan.

    Missing/access-denied/exited processes are explicit; their cost is never zero.
    The summed ledger is not unique physical memory or an independently additive
    GPU allocation. ps and rusage observations are sequential, not atomic.
    """
    if limit < 1:
        raise ValueError("footprint audit limit must be positive")
    descendants = set(snapshot(root, False, set(), current))
    ordered = sorted(current, key=lambda pid: (pid != root, pid))
    totals = {"root": 0, "descendant": 0, "temporal_vt_service": 0}
    entries = []
    for pid in ordered[:limit]:
        category = "root" if pid == root else "descendant" if pid in descendants else "temporal_vt_service"
        entry = {"pid": pid, "attribution": category, **reader.read(pid)}
        if entry["available"]:
            totals[category] += entry["physical_footprint_mib"]
        entries.append(entry)
    complete = bool(current) and root in current and len(ordered) <= limit and all(row["available"] for row in entries)
    return {"complete": complete, "processes": entries, "omitted_count": max(0, len(ordered) - limit),
            "observed_totals_mib": totals,
            "aggregate_physical_footprint_mib": sum(totals.values()) if complete else None}


def parse_marker(line):
    if not line.startswith(FIXTURE_PREFIX):
        return None
    if len(line) > FIXTURE_LINE_LIMIT:
        raise ValueError("fixture readiness line exceeds bound")
    try:
        value = line.decode("ascii")
    except UnicodeError:
        raise ValueError("invalid fixture readiness encoding") from None
    fields = re.findall(r"(?:^|\s)([a-z_]+)=([^\s]+)", value)
    keys = [key for key, _ in fields]
    if len(keys) != len(set(keys)):
        raise ValueError("duplicate fixture readiness field")
    fields = dict(fields)
    result = {}
    for key in FIXTURE_NUMBERS:
        raw = fields.get(key, "")
        if not re.fullmatch(r"[0-9]{1,20}", raw):
            raise ValueError("missing or invalid fixture readiness counter")
        result[key] = int(raw)
    if fields.get("hidden") not in ("true", "false") or fields.get("compositor_visibility") != "not_measured":
        raise ValueError("invalid fixture readiness visibility declaration")
    result["hidden"] = fields["hidden"] == "true"
    first, end = result["near_viewport_first"], result["near_viewport_end"]
    if not (result["generation"] > 0 and 0 <= first < end <= result["model_rows"] <= 100
            and end - first == result["near_viewport_ready"] <= 40
            and 0 < result["viewport_intersecting_ready_thumbnails"] <= end - first
            and all(result[key] == 0 for key in ("pending", "inflight", "ready", "remote_started", "failed"))):
        raise ValueError("fixture readiness does not establish a drained bounded viewport")
    result["scope"] = "application queues and ready model images; compositor visibility and GPU completion unmeasured"
    return result


def wait_for_fixture(path, process, timeout=30):
    """Read only new log bytes before warm-up, with bounded time and storage.

    No raw log data, command line, or private path is returned. The caller must
    retain the original log and separately validate stable workload/visibility.
    """
    if not 0 < timeout <= 30:
        raise ValueError("fixture readiness timeout must be within thirty seconds")
    started = time.monotonic()
    consumed = 0
    pending = b""
    with open(path, "rb") as log:
        while time.monotonic() - started < timeout:
            if process.poll() is not None:
                raise RuntimeError("application exited before fixture readiness")
            data = log.read(min(65536, FIXTURE_LIMIT + 1 - consumed))
            consumed += len(data)
            if consumed > FIXTURE_LIMIT:
                raise ValueError("fixture readiness log exceeds bound")
            pending += data
            lines = pending.split(b"\n")
            pending = lines.pop()
            for line in lines:
                if len(line) > FIXTURE_LINE_LIMIT:
                    raise ValueError("pre-readiness log line exceeds bound")
                marker = parse_marker(line)
                if marker is not None:
                    if process.poll() is not None:
                        raise RuntimeError("application exited at fixture readiness")
                    elapsed = time.monotonic() - started
                    if elapsed >= timeout:
                        raise TimeoutError("fixture readiness arrived after deadline")
                    marker["sampler_wait_seconds"] = elapsed
                    return marker
            if len(pending) > FIXTURE_LINE_LIMIT:
                raise ValueError("unterminated pre-readiness log line exceeds bound")
            if not data:
                time.sleep(min(0.25, max(0, timeout - (time.monotonic() - started))))
    raise TimeoutError("fixture readiness was not observed before deadline")


def cpu_seconds(value):
    days = 0
    if "-" in value:
        day, value = value.split("-")
        days = int(day)
    parts = [float(part) for part in value.split(":")]
    return days * 86400 + sum(part * 60**index for index, part in enumerate(reversed(parts)))


def process_table():
    output = subprocess.check_output(["ps", "-axo", "pid=,ppid=,rss=,time=,comm="], text=True)
    table = {}
    for line in output.splitlines():
        pid, parent, rss, cpu, name = line.split(None, 4)
        table[int(pid)] = (int(parent), int(rss), cpu_seconds(cpu), name)
    return table


def snapshot(root, include_services, baseline, table=None):
    if table is None:
        table = process_table()
    owned = {root}
    while True:
        expanded = owned | {pid for pid, (parent, _, _, _) in table.items() if parent in owned}
        if expanded == owned:
            break
        owned = expanded
    if include_services:
        owned |= {
            pid for pid, (_, _, _, name) in table.items()
            if pid not in baseline and "VTDecoderXPCService" in name
        }
    return {pid: table[pid] for pid in owned if pid in table}


def cpu_delta(current, previous):
    """Only counters present in both samples with the same executable are comparable."""
    return sum(max(0, row[2] - previous[pid][2]) for pid, row in current.items()
               if pid in previous and row[3] == previous[pid][3])


def host_other_cpu(current, previous, owned, seconds):
    """Audit unattributed host work from the same ps table; never add it to app totals."""
    if seconds <= 0:
        raise ValueError("sample interval must be positive")
    names = {}
    for pid, row in current.items():
        if pid in owned or pid not in previous or row[3] != previous[pid][3]:
            continue
        cpu = 100 * max(0, row[2] - previous[pid][2]) / seconds
        if cpu <= 0:
            continue
        # ps comm has executable names, never argv. Export only a bounded basename,
        # including when executable directories contain spaces or private names.
        name = "".join(c for c in Path(row[3]).name if c.isprintable())[:80] or "unknown"
        names[name] = names.get(name, 0) + cpu
    ordered = sorted(names.items(), key=lambda item: (-item[1], item[0]))
    return {
        "host_other_cpu_one_core_percent": sum(names.values()),
        "host_other_top_cpu": [{"executable": name, "cpu_one_core_percent": cpu}
                               for name, cpu in ordered[:8]],
    }


def process_rss_audit(root, current, limit=64):
    """Separate sampled root/descendant RSS from temporal decoder attribution.

    `current` is the already attributed snapshot. PIDs are sample identities,
    not proven lifetimes: same-executable PID reuse remains undetectable.
    """
    if limit < 1:
        raise ValueError("process audit limit must be positive")
    descendants = set(snapshot(root, False, set(), current))
    rows = []
    totals = {"root": 0, "descendant": 0, "temporal_vt_service": 0}
    for pid, (_, rss, _, executable) in current.items():
        attribution = ("root" if pid == root else "descendant" if pid in descendants
                       else "temporal_vt_service")
        totals[attribution] += rss
        name = "".join(c for c in Path(executable).name if c.isprintable())[:80] or "unknown"
        rows.append({"pid": pid, "executable": name, "attribution": attribution,
                     "rss_mib": rss / 1024})
    # Always retain the root; then the largest attributed RSS. Omitted RSS is
    # explicit and category totals include every process, never just this list.
    rows.sort(key=lambda row: (row["attribution"] != "root", -row["rss_mib"], row["pid"]))
    return {"process_rss": rows[:limit],
            "process_rss_totals_mib": {key: value / 1024 for key, value in totals.items()},
            "process_rss_omitted_count": max(0, len(rows) - limit),
            "process_rss_omitted_mib": sum(row["rss_mib"] for row in rows[limit:])}


def statistics(values):
    ordered = sorted(values)
    return {
        "mean": sum(ordered) / len(ordered),
        "p95": ordered[min(len(ordered) - 1, math.ceil(0.95 * len(ordered)) - 1)],
        "peak": max(ordered),
    }


def media_snapshot(socket_path):
    if not socket_path:
        return None
    result = {}
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(3)
        client.connect(socket_path)
        stream = client.makefile("rb")
        properties = ["hwdec-current", "frame-drop-count", "decoder-frame-drop-count",
                      "container-fps", "display-fps", "video-codec", "width", "height"]
        for index, prop in enumerate(properties):
            request = {"command": ["get_property", prop], "request_id": index}
            client.sendall((json.dumps(request) + "\n").encode())
            # Ignore unsolicited engine events; retain bounded reads/deadline.
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                response = json.loads(stream.readline(65536))
                if response.get("request_id") == index:
                    result[prop] = response.get("data")
                    break
            else:
                raise TimeoutError("mpv property response deadline")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mpv-socket")
    parser.add_argument("--include-new-vt-services", action="store_true")
    parser.add_argument("--per-process-rss", action="store_true",
                        help="Include bounded per-process RSS from each existing ps scan (basenames only)")
    parser.add_argument("--library-fixture-ready", action="store_true",
                        help="Wait at most 30s for explicit library fixture quiescence before warm-up")
    parser.add_argument("--macos-footprint", action="store_true",
                        help="Read separate physical-footprint ledgers for sampled PIDs (macOS only)")
    parser.add_argument("--warmup", type=int, default=10)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--output", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command and args.command[0] == "--" else args.command
    if not command or args.seconds < 1 or args.warmup < 0:
        parser.error("a command, positive sample duration, and nonnegative warmup are required")
    footprint = MacosFootprint() if args.macos_footprint else None
    baseline_table = process_table()
    baseline = set(baseline_table)
    rows = []
    error = None
    result = {"status": "incomplete", "warmup_seconds": args.warmup,
              "requested_sample_count": args.seconds, "samples": rows,
              "include_new_vt_services": args.include_new_vt_services}
    if footprint is not None:
        result["macos_footprint"] = {
            "enabled": True, "api": "proc_pid_rusage RUSAGE_INFO_V0 ri_phys_footprint",
            "entry_limit_per_sample": 64,
            "scope": "Separate OS physical-footprint ledger for existing attributed PIDs; no additional process discovery. "
                     "Unavailable processes make the aggregate null, never zero. Totals are not unique physical "
                     "memory and must not be added to RSS or GPU estimates. Sequential ps/rusage observations "
                     "are not atomic; start-abstime records permit later lifetime comparison but do not validate "
                     "the preceding ps identity. Temporal VT attribution and excluded baseline-service limits still apply."}
    if args.per_process_rss:
        baseline_vt = [
            {"pid": pid,
             "executable": "".join(c for c in Path(row[3]).name if c.isprintable())[:80] or "unknown"}
            for pid, row in sorted(baseline_table.items()) if "VTDecoderXPCService" in row[3]
        ]
        result["per_process_rss"] = {
            "enabled": True, "entry_limit_per_sample": 64,
            "baseline_vt_service_count": len(baseline_vt),
            "baseline_vt_services": baseline_vt[:64],
            "baseline_vt_services_omitted": max(0, len(baseline_vt) - 64),
            "scope": "Same ps scan; root and descendants are distinct from temporally attributed new VT services. "
                     "Preexisting VT services are excluded even if reused by this app; absent new-service RSS "
                     "is not evidence of absent or unchanged decoder memory. "
                     "PIDs are sample identities, not birth-time verified lifetimes. Shared RSS pages may be "
                     "double-counted; no physical-footprint, private-allocation, or GPU-memory attribution. "
                     "Short-lived or reparented descendants may be missed. No argv or executable directories."}
    del baseline_table
    with open(args.output + ".log", "w") as log:
        process = subprocess.Popen(command, stdout=log, stderr=log)
        forced_termination = False
        try:
            if args.library_fixture_ready:
                result["library_fixture_readiness"] = wait_for_fixture(args.output + ".log", process)
            time.sleep(args.warmup)
            media_start = media_snapshot(args.mpv_socket)
            previous_table = process_table()
            previous = snapshot(process.pid, args.include_new_vt_services, baseline, previous_table)
            measurement_start_utc = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            last = start = time.monotonic()
            for index in range(args.seconds):
                time.sleep(max(0, start + index + 1 - time.monotonic()))
                now = time.monotonic()
                current_table = process_table()
                current = snapshot(process.pid, args.include_new_vt_services, baseline, current_table)
                if process.poll() is not None:
                    raise RuntimeError("application exited before measurement completed")
                delta = cpu_delta(current, previous)
                rows.append({"rss_mib": sum(item[1] for item in current.values()) / 1024,
                             "cpu_one_core_percent": 100 * delta / (now - last),
                             "processes": len(current),
                             "elapsed_after_warmup_seconds": now - start,
                             **host_other_cpu(current_table, previous_table, set(current), now - last)})
                if args.per_process_rss:
                    rows[-1].update(process_rss_audit(process.pid, current))
                if footprint is not None:
                    rows[-1]["macos_footprint"] = footprint_audit(process.pid, current, footprint)
                previous, previous_table, last = current, current_table, now
            result.update({
                "media_start": media_start, "media_end": media_snapshot(args.mpv_socket),
                "warmup_seconds": args.warmup, "sample_count": len(rows), "interval_seconds": 1,
                "measurement_start_utc": measurement_start_utc,
                "host_other_cpu_one_core_percent": statistics(
                    [row["host_other_cpu_one_core_percent"] for row in rows]),
                "host_audit_top_limit": 8,
                "rss_mib": statistics([row["rss_mib"] for row in rows]),
                "cpu_one_core_percent": statistics([row["cpu_one_core_percent"] for row in rows]),
                "process_count_peak": max(row["processes"] for row in rows), "samples": rows,
                "include_new_vt_services": args.include_new_vt_services,
                "notes": "New VideoToolbox service attribution is temporal, not proven exclusive ownership. "
                         "Unrelated services may overcount. Aggregate RSS may double-count shared pages. "
                         "Unified GPU memory overlaps RSS and is not added. Short-lived descendants may be missed. "
                         "Host-other CPU is separate, not charged to the app; it includes system work and the "
                         "measurement harness. Names are basenames only, no argv. New/exited processes without "
                         "two comparable samples are missed. PID reuse under the same executable is not detectable.",
                "status": "completed_not_automatically_qualified",
            })
            if footprint is not None:
                values = [row["macos_footprint"]["aggregate_physical_footprint_mib"] for row in rows
                          if row["macos_footprint"]["complete"]]
                result["macos_footprint"].update({
                    "complete_sample_count": len(values), "incomplete_sample_count": len(rows) - len(values),
                    "all_samples_complete": len(values) == len(rows),
                    "physical_footprint_mib": statistics(values) if len(values) == len(rows) else None})
        except BaseException as exc:
            error = exc
            # Exception messages may contain socket paths or supplied commands.
            # Preserve the failure class and partial observations, never argv.
            result["failure_type"] = type(exc).__name__
        finally:
            # Prefer the application's --quit-after for graceful GL teardown.
            try:
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    forced_termination = True
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
            except BaseException as exc:
                result["cleanup_failure_type"] = type(exc).__name__
                if error is None:
                    error = exc
        result["exit_code"] = process.returncode
        result["forced_termination"] = forced_termination
        result["sample_count"] = len(rows)
        if error is not None or process.returncode != 0 or forced_termination:
            result["status"] = "incomplete"
        with open(args.output, "w") as output:
            json.dump(result, output, indent=2)
        print(json.dumps({key: value for key, value in result.items() if key != "samples"}, indent=2))
        if error is not None:
            raise error
        if process.returncode != 0 or forced_termination:
            raise RuntimeError("application did not complete measurement with a clean exit")


if __name__ == "__main__":
    main()
