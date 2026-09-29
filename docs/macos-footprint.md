# macOS physical-footprint measurement

SPEC section 11 requires RSS and an available platform-specific memory metric.
The optional `scripts/measure.py --macos-footprint` implementation passes its
deterministic tests and native API self-check on this host. A first application
sample on frozen d7c58acd completed: settled mean147.606MiB (peak147.923),
minimized mean147.731MiB (peak148.048), each with all60 ledger-query sets complete.
The workload was the10,000-item offline raster library, with six intersecting
thumbnails and no media load. This is not the30-visible acceptance case or a
playback/decoder-service footprint result. Exact raw observations are retained in
`artifacts/native-child-v1/idle-resources` and the
[public resource export](evidence/2026-09-29-raster-idle-export.json).
Earlier RSS observations are not retroactively converted.

The sampler calls the system `libproc` function `proc_pid_rusage` using the fixed
`RUSAGE_INFO_V0` flavor. Its `ri_phys_footprint` ledger is reported in MiB,
separately from sampled RSS. The declaration was inspected in the installed
macOS SDK's `sys/resource.h` and `libproc.h`: a sixteen-byte UUID followed by ten
64-bit fields, with physical footprint at byte 72, process start absolute time
at byte 80 and exit absolute time at byte 88. The implementation uses a 96-byte
buffer and flavor zero, never the moving `RUSAGE_INFO_CURRENT` definition. The
C11 probe compiled with strict warnings and passed its live self-query; the
Python ctypes reader also returned a live nonzero footprint for itself.
The explicit probe source is `scripts/check_macos_footprint.c`; compile with
`cc -std=c11 -Wall -Wextra -Werror scripts/check_macos_footprint.c -lproc -o /tmp/serein-footprint-check`
and run that temporary executable outside any measurement window. The Python
reader self-query used `MacosFootprint.read(os.getpid())`.
Neither self-query measures the application or proves decoder-service access.

Only PIDs already attributed by the ordinary process snapshot are queried. Each
sample retains root, descendant and temporally attributed VideoToolbox categories,
with at most 64 process records. Access denial, process exit and omitted entries
are explicit. An incomplete sample has a null aggregate, and any incomplete
sample suppresses whole-run footprint statistics. Partial category totals remain
clearly labeled observed totals. No command arguments, paths or process UUIDs are
exported; start absolute times allow later comparison of observed lifetimes.

The `ps` snapshot and ledger queries are sequential, so they do not prove an
atomic process identity. Previously existing VideoToolbox services remain excluded
under the sampler's existing attribution rule, even if reused by the application.
Short-lived/reparented helpers can still escape sampling. A complete ledger query
set therefore does not establish complete application memory attribution.

Do not add this metric to RSS, texture estimates or unified GPU allocations.
The aggregate sums OS process charges; it is not a claim to unique physical pages
or exact decoder/driver/compositor ownership. Separate GPU allocation evidence is
still required. Synthetic tests cover ABI layout, permission errors, missing
identities, category sums, entry bounds and incomplete-summary behavior. They
pass within the 156-test Python suite; the native checks are separate evidence.
