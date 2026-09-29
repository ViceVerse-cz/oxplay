# Local soak memory attribution

The completed local mixed-use run in the
[retained analysis](evidence/2026-09-29-soak-local-analysis.json) used
source `ee59eaa47289382b6784547e8bece1b38e5e60a5` and executable SHA-256
`464fdcf1432f1425698cf2892f2fd7f3e942c4e132dfb176c5c45bcb5582b0a8`.
This review concerns that frozen build, before the subsequent seek, mute,
keyboard and offline-library changes.

The 3,600 samples each contained exactly two attributed processes. Aggregate RSS
averaged 232.690 MiB and peaked at 257.313 MiB. Minute medians rose monotonically:
217.727 MiB at minute 11, 234.609 MiB at minute 30, and 256.047 MiB at minute 60.
The last thirty-minute increase was about 0.715 MiB/minute. Four of twenty
approximately phase-matched comparisons exceeded 10% growth. The observed
functional completion, 12 loads, and no surviving audited descendants do not
establish absence of a leak. Aggregate RSS cannot identify which of the two
processes grew or distinguish retained useful data, allocator retention, shared
pages and unreachable allocation.

## Source and counter review

The frozen presenter owns `displayed`, `next`, and a retirement list. Resizing
replaces only `next`; every `AfterRendering` drains that list and deletes its
framebuffer/texture. Final counters record 302 target allocations but only
7,936,128 bytes of live tracked targets. The allocation counter is cumulative;
it is not the number of retained targets. Driver-deferred deletion, mpv-owned
GPU surfaces and compositor allocations are outside that counter. No per-phase
target inventory was captured, so this does not prove their bounded residency.

The frozen player caps its native entry-to-load map at eight, removes active
entries on native end-of-file, coalesces progress queries and absolute seek
requests, and retains one player/presenter across controls and loads. The soak
did not request remote streams, online captions, or account work. No concrete
unbounded application collection was found in these reviewed paths.

At pinned Slint `cf3b07d4917e6759a63b0c03913a2594ec653414`, FemtoVG's
`images.rs::TextureCache::drain` removes image wrappers no longer referenced by
items; `lib.rs::render` calls it after submitting rendering commands, before
`AfterRendering`. New borrowed-image wrappers are created on publication, but
that fact alone does not demonstrate accumulation. Allocator/driver retention
from repeated wrapper and texture turnover remains an attribution candidate,
along with media decoder surfaces and unrelated temporal decoder-service work.

## Optional sampler breakdown

`scripts/measure.py --per-process-rss` adds a bounded breakdown from the **same**
existing `ps` sample. It performs no additional scan. Each sample includes:

- Root, observed descendants, and newly observed VideoToolbox services as
  separate RSS totals; temporal service attribution remains explicitly weaker
  than process-tree ownership.
- Up to 64 individual rows with PID, bounded executable basename, attribution
  category and RSS. The root is retained first, followed by the largest rows.
- Omitted row count and RSS if the bound is reached; category totals still
  include all attributed processes.

The opt-in report also records the original sampler baseline's count and bounded
PID/basename list of already-running VideoToolbox services. Those services stay
excluded from new-service attribution even if the app subsequently reuses one.
A root-only sample therefore cannot establish that decoder memory is absent or
unchanged, and cannot be treated as equivalent to the earlier two-process total.
No shared service may be killed merely to force a fresh attributable instance.

The option is off by default and adds no fields to default output. PIDs are
sample identities without birth-time verification. Same-executable PID reuse,
short-lived processes and unobserved reparenting remain limitations. Summed RSS
may double-count shared pages and is not physical footprint, private heap size
or separately additive GPU memory. No argv or executable directory is emitted.

Nine synthetic sampler tests passed before the repeat, covering accounting,
duplicate basenames, bounded rows, omitted-byte totals, privacy, opt-in
compatibility, and unchanged scan count.

## Completed attribution repeat — 2026-09-29

The repeat sampled from 09:08:16 UTC for 3,600 one-second observations and exited
normally after 3,607.897 seconds of external harness time. It used the **same
frozen executable and 119-input source inventory above**, the same local fixture,
and the same sixty-cycle workload. The only measurement change was the opt-in
per-process RSS breakdown. Newer source edits made during the run were not in its
binary. No allocation profiler, screenshot, VM sampler or second media workload
was introduced. Bounded external display assertions kept this diagnostic awake.

The strict post-run functional analyzer accepted all 120 ordered playing
checkpoints, 60 completed cycles, 12 loads, zero unrelated catalog notifications,
and normal terminal cleanup. The anchored process group was absent after reap;
no audited ordinary descendant remained. These are local functional observations,
not completion of the full mixed-use SPEC soak. The final generic shutdown
snapshot contains `stop_pending=true` because main submits a second stop after
the driver has already confirmed its own terminal cleanup; it is not evidence
that the driver's stop timed out.

Aggregate RSS averaged **234.158 MiB**, with p95 254.469 MiB and peak 256.813 MiB.
Mixed-use CPU averaged 33.669% of one logical core; this includes pause, hidden,
resize and reload phases and must not be substituted for steady playback or idle
qualification. Host-other CPU averaged 43.644%, with p95 120.992% and peak
261.731%; it remains a separate host observation, not application-owned CPU.

Every sample contained the same application PID and one newly observed
`VTDecoderXPCService` PID, with no omitted RSS rows or ordinary descendants. The
sampler excluded two decoder services that existed before launch. No shared
service was stopped to force this attribution. Stable temporal membership does
not prove the new service's exclusive ownership, or account for any app work
performed by the two excluded services.

Whole-minute comparisons use minutes 11–30 against 41–60: thirty minutes preserves
the five-minute reload, odd/even fullscreen, and three-minute seek phases. App and
sampler clocks have different origins, so these remain approximate phase matches.

| RSS category | Median matched increase | Median growth | Growth range | Pairs above 10% |
| --- | ---: | ---: | ---: | ---: |
| Aggregate | 21.313 MiB | 9.306% | 9.078–10.947% | 2 / 20 |
| Application root | 18.551 MiB | 8.882% | 8.710–10.640% | 2 / 20 |
| Temporal decoder service | 2.773 MiB | 13.609% | 12.769–14.604% | 20 / 20 |

Application minute medians increased throughout all sixty minutes: 199.281 MiB
at minute 11, 213.492 MiB at minute 30 and 232.125 MiB at minute 60. Decoder-service
medians were 19.703, 21.047 and 23.734 MiB respectively, with small intervening
decreases. Most absolute matched growth is therefore in application RSS. This
narrows process-level attribution; it does not distinguish reachable allocations,
allocator retention, driver mappings, shared pages or a leak. Category medians
need not add exactly to the median of the aggregate. **Memory acceptance and the
full SPEC soak gate remain open.** The repeated increasing trend does not support
a leak-free claim, even though most aggregate pairs fall below 10%.

All sixty minute medians were independently recomputed from the raw 3,600 samples
and matched the retained analysis exactly. Per-sample process sums, identity and
omitted-row checks also passed. The last observation at 3,600.150 seconds falls
outside the half-open minute-60 bin; it remains in the full sample evidence and
whole-run resource summaries.

## Evidence and next attribution step

The [export manifest](evidence/2026-09-29-soak-attribution-export.json) records each
original and published SHA-256, sizes, privacy checks and exact transformations.
Its SHA-256 is
`e618c7b63f305e742a8f1a10f011c879e997b59bfa2135254298ed90addf50c5`.
It links the deterministic gzip containing all original resource bytes, raw
scalar native log, original source inventory, cleanup summary, process-level
analysis and separate strict functional analysis. No private profile paths,
URIs, credentials, argv or executable directories are published. PID identities
and bounded executable basenames remain for attribution. The preserved sampler
and harness hashes are in the summary; their raw files remain ignored.

This repeat completes the proposed process-breakdown experiment. Both
`current_build_qualification` and `full_spec_soak_gate` remain false. RSS is not
unique physical footprint and does not measure separately additive GPU memory.
The first-hour result and this repeat both warrant further allocation attribution
before any resource-ownership fix or acceptance claim.

For a separate diagnostic run, take only a few `vmmap -summary` observations of
the known app PID at the same visible-playing phase after warm-up and later in
the run, plus after confirmed terminal stop before process exit if the driver
explicitly provides that interval. Do not mix those intrusive observations into
resource qualification. Keep raw process maps local; publish only category
totals. If app growth dominates, compare malloc/VM/IOSurface category deltas and
then select a focused allocation instrument. If the temporal service dominates,
first establish whether that service is exclusively associated with this app;
aggregate membership alone cannot establish that. No causal attribution or
leak-free qualification follows from the current evidence.
