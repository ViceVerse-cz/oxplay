#!/usr/bin/env python3
"""Measure RSS and CPU-time deltas for an owned process tree.

100% CPU means one logical core. Very short-lived processes may escape sampling.
Optional newly launched VideoToolbox services use temporal attribution, which is
not proof of exclusive ownership. Raw commands/provider data are not exported.
Optional per-process RSS uses the same ps sample, not an additional scan.
"""
import argparse
import json
import math
from pathlib import Path
import socket
import subprocess
import time


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
    parser.add_argument("--warmup", type=int, default=10)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--output", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command and args.command[0] == "--" else args.command
    if not command or args.seconds < 1 or args.warmup < 0:
        parser.error("a command, positive sample duration, and nonnegative warmup are required")
    baseline_table = process_table()
    baseline = set(baseline_table)
    rows = []
    error = None
    result = {"status": "incomplete", "warmup_seconds": args.warmup,
              "requested_sample_count": args.seconds, "samples": rows,
              "include_new_vt_services": args.include_new_vt_services}
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
