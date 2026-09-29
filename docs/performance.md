# Performance qualification

Numbers in SPEC remain contract targets/ceilings, not results. Reference host:
Apple M1 (7 GPU cores), 16 GiB, macOS 27.0, built-in 2560×1600 Retina, AC power,
low-power mode off. Release build, no debug logging. Sampling script:
`scripts/measure.py`, one-second process CPU-time deltas (100%=one logical CPU),
aggregate process-tree RSS. Shared pages can be counted twice; macOS footprint
is recorded separately in newer runs below; unified GPU allocations remain
unmeasured. Script can miss
very short-lived children and is unsuitable alone for transient-helper peak gates.

The sampler now reuses each existing whole-process `ps` snapshot to record a
separate host-other CPU aggregate and the eight busiest executable basenames.
That host-other audit emits no foreign PID, arguments or directory paths and does not charge that
activity to the application. Per-sample elapsed offsets and a UTC measurement
start make overlap auditable. This includes system services and the harness;
it is context, not proof that a process caused a playback stall. Processes born
or exited between samples, counter resets, and unobservable PID reuse limit the
audit. `python3 scripts/test_measure.py` passes nine attribution, privacy,
bounded-output and CPU-counter regression tests. Earlier samples have no
per-second foreign-work audit and cannot be retroactively qualified by this change.
An optional `--per-process-rss` now records bounded root/descendant/temporal-VT
breakdowns from the same scan; it was not enabled in historical samples. See
[soak-attribution.md](soak-attribution.md) for its identity and shared-memory limits.

An opt-in `--library-fixture-ready` sampler change admits the explicit
offline raster fixture before starting warm-up. It validates the app's bounded
queue-drain record with a thirty-second deadline, then stops reading readiness
events during the measured phase. Deterministic tests and the native readiness runs recorded below pass. This does not retroactively qualify earlier idle measurements or
prove compositor visibility; see [fixture admission](library-resource-fixture.md).
An optional macOS physical-footprint query passes unit tests and native self-query; it reports
separate OS ledger values and explicit missing coverage. Its ABI, attribution and
remaining validation are recorded in [macOS footprint](macos-footprint.md).

Required baseline: generated H.264 1920×1080@60 SDR yuv420p, AAC 48 kHz, 90 seconds.
`ffmpeg` testsrc2/sine fixture; current local sample encoded with h264_videotoolbox
at 8 Mbit/s; generation script uses libx264 for host independence. Compare the exact
same resulting file between players; do not compare independently generated files.

Qualification remains incomplete. Shell idle with no thumbnails cannot pass the required
settled 30-thumbnail library case. A native run with an obscured/minimized window
cannot count as visible playback. Warm-up 10 seconds then 60 one-second samples is the
initial method. Mean/p95/peak RAM and CPU will be recorded here with the code revision.

Initial debug lifecycle showed startup/seek drops. Release lifecycle command is
not a performance test (fullscreen transitions, synthetic pointer events, concurrent
compilation in an early run). It verifies observed state/counters only. GPU draw time,
actual swap/present cadence, energy, latency percentiles, memory footprint, browser
baseline and 10k-item UI virtualization were not measured in that initial slice.
Later baseline results are recorded below. The completed local hour-long soak
is [recorded separately](soak-local.md): functional completion passed, but RSS
growth and limited workload leave the full soak gate open.

## Initial display-clock candidate: throughput corrected, CPU ceiling failed

A controlled native-swap differential isolated the severe frame loss: native
CGL interval 1 reproduces it, while interval 0 with a bounded, nonblocking
CVDisplayLink gate delivers 60 fps. Interval 0 alone fails. This is a small
macOS media adapter inside the same shared Slint UI, not another application
frontend. Details/source provenance are in
[the experiment record](experiments/native-swap.md). The current macOS default
lead is upstream's 50 ms; speculative autosync remains off. Historical failures
below describe the earlier zero-lead path and must not be read as current results.

The application and diagnostic-only standalone libmpv/SDL harness used the exact
same fixture, H264 1080p60 VideoToolbox decoding, AAC 48 kHz AVFoundation output,
1384×778 physical video target, nonblocking render, 50 ms lead and advanced
control 0. Both used ten seconds warm-up and sixty one-second process samples.
The current display mode reported by SDL is 1440×900 at 60 Hz with Retina scaling;
System Information reports the built-in panel as 2560×1600. The application has
a larger window containing its UI; the standalone window contains only video.
A five-second diagnostic wake established an awake display before standalone
measurement; SDL then held its normal idle-display-sleep assertion.

| Run / metric | Mean | p95 | Peak |
|---|---:|---:|---:|
| Slint application RSS, MiB |185.767|186.453|186.516|
| Slint application CPU, % of one logical core |52.786|64.822|65.767|
| Standalone same-engine RSS, MiB |163.458|164.156|164.266|
| Standalone same-engine CPU, % of one logical core |44.585|53.170|54.011|

Both runs include the application/harness and one newly launched VideoToolbox
service (temporal attribution, not proof of exclusive ownership). The application
had one startup VO drop unchanged at 10/25/70 seconds and exit; the standalone
had three unchanged at 10/25 seconds and exit. Decoder drops were zero. Thus
steady throughput improved without reducing input quality, but the application
**fails the 50% CPU release ceiling and the 25% target**. Its RSS meets playback
budgets. The difference of independent means is about 22.31 MiB and 8.20 CPU
percentage points; this is an overhead estimate, not measured per-stage cost.

Application binary SHA256
`a36b84a0f86d0ef133ddf671996b6fc220a427d60a7dfc9be3d2d30dca9f4761`
was built from a dirty working tree based on `319fa36251de8d1b97cab964f8906796db03f817`.
Exact media/harness source hashes, standalone binary hash, sanitized full sample
arrays and attribution notes are retained in [evidence/README.md](evidence/README.md).
Optional per-frame timing instrumentation was off in these resource samples.
Two application textures occupied 8,614,016 bytes; this excludes engine/Slint/
driver/compositor allocations and overlaps unified memory rather than adding to
RSS as independent physical memory. There were 5,079 video draws, 5,421 UI draws,
and zero catalog changes/resets across 85 seconds. This demonstrates model
locality, not partial GPU redraw. Related rows were empty in this resource run;
the realistic fixture-list matrix remains separate.

An awake 20-second fixture lifecycle passed paused seek, resize/fullscreen,
subtitle controls, minimize/restore and teardown; the hidden clock stopped.
Initial sleep-state failure was isolated: CoreGraphics reported active=0,
asleep=1 and clock creation failed -6661; a bounded wake produced active=1,
asleep=0 and the identical lifecycle passed. A newly implemented assertion is
scoped to observed Playing and releases on pause/hidden/ended/teardown. Native
pmset checks confirmed its presence during Playing and absence while hidden/
paused and after teardown. A subsequent controlled initially-asleep launch recovered
on a native wake event and began playback without restarting (video-integration.md). Nothing changes
system-wide sleep settings or wakes screens automatically during playback.

Remaining qualification includes profiling the CPU failure, actual A/V sync and
scanout (the timing-enabled 30-second run averaged 27.8 ms render-start deadline
lateness), frame-time/input percentiles, GPU work/energy, same-format browser
baseline, realistic library scenarios, repeated runs and the hour-long soak.
Zero VO drops alone does not pass the optimized gate.

## Header/sidebar cache comparison — cached run rejected

A source-profiled candidate caches only the header and sidebar. Same binary and
commit in both runs: `c77ebe91e8a268ec0a4d26150a76c3ace93f47b7`, SHA256
`6b3c97f2a2a610aab267d0bb6dce4b806b714b195efcc4c92e8627cd30637ede`.
Both used `--demo-related` (30 explicitly labeled synthetic text rows with empty
thumbnails), the same local clip, ten-second warm-up and sixty one-second samples.
The first used `--no-ui-cache`; the second used the checkpoint's default cache.
This is not the settled 30-decoded-thumbnail library test.

| Metric | Cache off | Cache on |
|---|---:|---:|
| RSS mean / p95 / peak, MiB |186.972 / 187.688 / 187.766|193.579 / 194.703 / 194.813|
| CPU mean / p95 / peak, % of one logical core |47.725 / 62.000 / 64.176|38.773 / 52.363 / 66.038|
| VO drops at 10 / 25 / 70 / 85 seconds |1 /1 /2 /2|2 /2 /299 /592|
| Decoder drops |0|0|
| Display-clock starts /stops at exit |1 /0|95 /94|

The noncached control lost one of 3,600 expected warm frames (0.028%) and met the
mean CPU ceiling in this particular run, while missing the 25% target. The cached
run's lower aggregate CPU is **not a passing result or a qualified improvement**:
it lost 297 warm frames by 70 seconds and 592 total, despite its first 25 seconds
looking healthy. Both had zero catalog row changes and one fixture initialization
reset, hardware VideoToolbox decoding, AVFoundation output and identical media
target dimensions. Full sample arrays are retained in `docs/evidence/`.

A separate eight-second lazy-render diagnostic reported zero new cached layers
on the reported steady frames, but also suffered frame loss. Cache geometry
estimates at 1320×860 logical/scale 2 are 3.95 MiB RGBA plus 0.99 MiB stencil before
padding, not a measured GPU residency counter. The measured RSS difference 6.61 MiB
includes all allocations and cannot be equated with those texture bytes.

No application-owned competing build/player/helper ran. A later system snapshot
showed syspolicyd, parsec-fbf, Spotlight/Biome and XProtect doing significant work;
this is a possible confound, not a proven cause, and no OS process was stopped.
Repeated clock restarts identify a concrete fragility: the earlier adapter stopped
after two empty display ticks (~33ms), losing phase after brief engine/driver
jitter. The new source keeps the clock's phase only while the engine reports
Playing and stops directly on pause/hidden/buffering/ended. Empty display ticks
never call mpv, wake the UI or redraw. Seven media tests and strict Clippy pass.
The new 20-second native lifecycle passed file-scoped initial subtitle loading,
paused seek, shared fullscreen controls, hidden clock stop and teardown, with
zero unrelated catalog changes and one fixture initialization reset. Its debug
transition frame losses are not a performance result; release resource and
throughput requalification of this revision remain pending. Caching is now off by default in
source, with `--ui-cache` explicitly experimental. The checkpoint binary flags
above are historical and must not be confused with the new default.

## Continuous-clock follow-up — uncontrolled host workload

Release SHA256 `3b24ba9e770e8b6525bc280473c214caae44734ff64547627f0822d8eb9cda30`
(dirty source based on c77ebe91; precise file hashes in the evidence JSON) was
sampled twice with the same clip, 30 labeled related text rows and 10+60-second
method. Default caching was off; the second run explicitly added `--ui-cache`.
The application and one new decoder service exited cleanly in both runs.

| Metric | Cache off | Cache on |
|---|---:|---:|
| RSS mean / p95 / peak, MiB |188.094 /188.984 /189.125|194.704 /196.031 /196.094|
| CPU mean / p95 / peak, one-core % |46.404 /59.222 /60.813|45.996 /62.705 /63.003|
| VO drops at 10 /25 /70 /85 seconds |2 /83 /354 /386|2 /2 /259 /481|
| Display clock starts /stops |1 /0|1 /0|

**Neither run qualifies playback or provides a controlled cache comparison.**
The uncached run lost 352/3,600 warm frames (9.78%); cached lost 257/3,600 (7.14%).
Decoder drops remained zero. Both retained the 1384×778 target, two textures
(8,614,016 bytes), and zero catalog changes/one fixture initialization reset.
Continuous clock phase removed restarts but did not prevent frame loss under
these observed conditions. Lower CPU numbers cannot establish efficiency.

No agent working on this repository launched a competing build/helper/player.
However, an immediate post-run process inspection found clippy drivers from a
separate local project. This foreign work was not stopped.
The runs occupied 01:43:05–01:44:31 and 01:44:46–01:46:11 UTC on 2026-09-29;
raw process/start-time records are in `artifacts/continuous-clock-host-work.json`.
The exact overlap cannot be reconstructed for every process from a post-run
snapshot. Therefore both remain uncontrolled observations, not a causal test.
Repeat with host-workload auditing before evaluating the revision's resource or
throughput gate. Full sample arrays/provenance remain in `docs/evidence/`.

## Empty-shell measurement (completed)

Release binary SHA256 `992f684ca0dd31b08c83101f1e67c5a16e819928001440c3d517eca8364cf37c`.
Command: `python3 scripts/measure.py --include-new-vt-services --output artifacts/idle.json -- ./target/release/serein --quit-after 85`.
Ten-second warm-up, 60 samples at one-second intervals, one process and no newly
launched decoder service. Source/evidence provenance is in evidence/README.md.

| Metric | Mean | p95 | Peak |
|---|---:|---:|---:|
| Aggregate RSS, MiB |106.418|106.563|106.563|
| CPU, percent of one logical core |0.0338|0|2.0299|

Application lifetime counters: 3 draw callbacks, 0 media-driven redraw requests,
0 catalog changes/resets. No recurring progress timer ran. This is **not** the
required settled 30-thumbnail library measurement. It provides initial empty-shell
idle evidence only; no full resource gate is marked passed. Subsequent code avoids
allocating media targets before the first actual video frame; this run predates
that change and its ~8.6 MB of empty targets must not be silently subtracted.

## Historical playback failures and baseline limitations

Before the direct Winit media-wake workaround, a release run had 810 redraw requests
but only 2 draw callbacks in 15 seconds. After the workaround, a 75-second run had 1,897 draw
callbacks and 2,589 dropped frames. That run's initial sampler accumulated subprocess
sampling overhead and exited before all 60 samples completed; no RSS/CPU summary is
reported from it. The sampler now uses absolute interval deadlines.

A five-second macOS `sample` capture found the main thread largely waiting;
a GL-call bottleneck was not established. Its one-shot physical footprint was
269.8 MB, peak 280.6 MB; these are **not**60-second RSS statistics. Source-confirmed
IOSurface import was visible in stacks. A concurrent snapshot showed unrelated
system mediaanalysisd at ~217% CPU. This activity was not stopped. VideoToolbox
also launched a decoder XPC process outside the child tree; resource sampling
now optionally includes newly launched decoder services, with temporal-attribution
limits clearly marked in the JSON.

Standalone `mpv --gpu-api=opengl --gpu-context=cocoa` was rejected: this installed
Homebrew build only exposes Vulkan window contexts despite its libmpv OpenGL
render API. A second `--vo=gpu-next --hwdec=auto-safe --geometry=928x280` attempt
reported actual VideoToolbox decode but `Couldn't start DisplayLink, no Screen or
DisplayLink available`. Its IPC requests timed out and it failed SIGTERM shutdown;
the known owned process was forcibly reaped. No baseline performance result is
claimed. Presentation dimensions/backend cannot be asserted equivalent. The newer SDL/libmpv same-engine baseline above now supplies a functioning
OpenGL comparison; it does not change these rejected packaged-mpv results.
Browser/energy baselines remain open.

## Reproducible failing playback sample

Application commit `463e67748e99ac288d8f765a2dfa3affad5da1ea`; release binary SHA256
`982a0050b1fd2259a289c70467475599aa447b44300cf4c85a30d2fb0a7d0cd2`.
Input fixture SHA256 `d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0`.
Command: `python3 scripts/measure.py --include-new-vt-services --output artifacts/playback.json -- ./target/release/serein --local artifacts/local-1080p60.mp4 --quit-after 85`.

| Metric | Mean | p95 | Peak |
|---|---:|---:|---:|
| RSS, MiB (app + newly launched decoder service) |178.815|179.266|179.438|
| CPU, percent of one logical core |33.261|39.660|41.833|

Ten-second warm-up, 60 samples, 2 processes. Attribution caveats apply. Native decoder
was `videotoolbox`; AAC 48 kHz audio observed. Across 85 seconds: 2,119 render notifications,
2,118 video draws, 2,961 VO dropped frames, 0 decoder dropped frames. Two persistent media
targets totaled 8,342,400 bytes (~7.96 MiB), excluding engine/driver/compositor allocations.
Catalog changes/resets remained 0. This **fails playback/frame pacing**: resource
numbers at ~25 presenter draws/second cannot pass a 1080p60 throughput gate. CPU also
misses the 25% target. No quality reduction or budget increase is used to hide this.
The missing frames precede the render callback. Investigation covers engine
frame timing, the audio clock, and render scheduling; UI throughput alone does
not explain this result.

## Timing-lead investigation

A controlled 15-second release A/B with `SEREIN_MEDIA_TIMING=1`:

| Requested lead | Render notifications / video draws | VO drops | Decoder drops | Aggregate mpv render submission time |
|---|---:|---:|---:|---:|
|0 ms|543/542|339|0|1.405s|
|50 ms (diagnostic, unscheduled)|783/782|101|0|1.797s|

GL-state save overhead was 11.2 ms and 14.2 ms **in total**, respectively. This does
not establish a GPU-time metric. The 50 ms lead improves notification cadence but
would present early without a deadline scheduler; it is not adopted as a passed
A/V path. Source review found `vo_is_ready_for_frame` subtracts video-timing-offset
before waking the core and `render_frame` drops overdue frames before notifying
the render client. Subsequent one-shot deadline trials also failed:

| Scheduled lead / preparation allowance | VO drops at 10 s / 25 s | Warm-window drops per 900 nominal frames | End notifications / video draws |
|---|---:|---:|---:|
| 50 ms / 0 ms | 190 / 454 | 264 (29.3%) | 1,240 / 1,239 |
| 50 ms / 8 ms | 200 / 513 | 313 (34.8%) | 1,155 / 1,154 |

Each run lasted 30 seconds. Both had zero decoder drops. These diagnostic trials
are too short for resource qualification. The first started rendering on average
6.43 ms late; allowing 8 ms of preparation did not fix loss. At that stage default lead and preparation remained zero. These rejected
one-shot timer trials did not justify changing defaults; the later native
display-clock differential is recorded separately above.

The standalone fixture run also reported CoreAudio rejecting its mono channel
layout and falling back to avfoundation. Since mpv derives video deadlines from
the audio clock, `SEREIN_FORCE_STEREO=1` is a separate diagnostic that requests
stereo output and records the observed `current-ao` property. This does not alter
video resolution, frame rate, codec, or decoder. The stereo experiment did not switch the observed driver: `avfoundation`
remained active, with 343 and 890 drops at the 10- and 25-second checkpoints
(547/900 nominal frames). The ineffective stereo option was removed.
A subsequent `SEREIN_DIAGNOSTIC_NULL_AUDIO=1` path selects mpv's timed null
audio output to isolate the audio clock. It carries a persistent visible warning,
produces no audible audio, and cannot pass the functional or optimized gate.

The null-audio experiment at commit `3846416` also failed: 153 → 390 VO drops
between 10 and 25 seconds, or 237/900 (26.3%) nominal frames. It observed
`current-ao=null`, active VideoToolbox, and zero decoder drops. This changes
behavior relative to avfoundation but does not establish an isolated audio-driver
cause. The persistent diagnostic banner changes layout slightly; no resource
comparison is drawn from these short trials. The retained log and binary hash
are in evidence/README.md.

The render-ahead experiment also failed (277/900 warm nominal frames lost) and
was removed. Its bounded ownership implementation, measurements and upstream
VO timing analysis are preserved in [the experiment record](experiments/render-ahead.md).
The active presenter remains a persistent target pair, with optional one-shot
deadline diagnostics and no permanent rendering loop.

An additional idle run requested minimization and recorded 111.579 MiB mean RSS
and no CPU-time increase at the sampler's precision. Its minimized state was
not verified, so it is excluded from the minimized acceptance case. The harness
now reapplies minimization after startup activation and checks Winit's observed
window state before sampling begins.

## Callback delivery measurement

Commit `7e4e81b`, binary SHA256
`d3253b95619de9b015699c3312ea6f24ec9a153bac656fe323eb336e67ff7200`.
Command: `SEREIN_MEDIA_TIMING=1 ./target/release/serein --local artifacts/local-1080p60.mp4 --diagnostics --quit-after 30`.
The newest callback-to-BeforeRendering delay averaged 0.863 ms across 728
samples, with a 50.5 ms maximum; coalescing makes this a lower bound on queue
age. Render starts averaged 14.3 ms after their frame deadlines. Warm VO drops
rose from 349 to 876, with no decoder drops. This suggests engine/audio timing
is a stronger next lead than average UI dispatch cost, without establishing a
complete cause or excluding occasional UI stalls. The raw log is retained.
A source-backed `autosync=30` diagnostic is described in experiments/autosync.md;
defaults remain unchanged pending its result and A/V qualification.

The audible autosync trial also failed: 303/900 warm frames were lost (33.7%).
No speculative timing setting was made the default. See the retained autosync
experiment for its rationale, exact settings, and result. These trials preceded the native display-clock correction above; they did
not resolve pacing or justify retaining autosync as a production default.

A minimized-state check initially ran after the native event loop had returned,
when window state had already changed. That run exited with an explicit failure
and is excluded from acceptance. Verification now runs in the final timer before
requesting event-loop exit, as well as at startup; a short native check observed
`Some(true)` at both points and exited cleanly. The sampler now records the exit
code/forced termination and rejects a failed or forcibly stopped application.

## Verified minimized idle sample

Application commit `9bada8a`; release binary SHA256
`7b1e68569a66e881910a0966126ed2d65293e46987fb41c3d9fd63223469bbf5`.
Command: `python3 scripts/measure.py --include-new-vt-services --output artifacts/minimized-qualified.json -- ./target/release/serein --minimized --quit-after 85`.
Ten-second warm-up followed by 60 one-second samples. Winit reported minimized
at five seconds and immediately before quit at 85.096 seconds, with no intervening
unoccluded event. Exit code was zero and no forced termination occurred.

| Metric | Mean | p95 | Peak |
|---|---:|---:|---:|
| Aggregate RSS, MiB |111.610|111.594|111.922|
| CPU, percent of one logical core |0.0154|0|0.9260|

One process, no new decoder service. CPU time has the system sampler's finite
precision; zero samples do not mean literally no CPU work. Lifetime counters:
four UI draw callbacks, zero media redraw requests, zero catalog changes/resets,
and zero media targets allocated. Slint/driver GPU allocations are not zero and
remain unmeasured. This run meets the minimized mean-CPU target and showed no
self-sustaining redraw loop; it does not qualify the full library or playback
resource gates. Earlier minimized attempts had missing/misplaced state checks
and are not substituted for this verified sample. Broader repeated-run/platform
qualification remains open. Raw JSON and the window-state log are in evidence/.

## GPU execution diagnostic implementation

`SEREIN_GPU_TIMING=1` now enables bounded asynchronous timestamp sampling around
media rendering and subsequent Slint composition. See [gpu-timing.md](gpu-timing.md)
for exact interval boundaries, unsupported/disjoint reporting, fixed query limits
and why these intervals are not whole-frame scanout or GPU utilization. Default
resource runs allocate no query objects. Native timing qualification is pending;
arithmetic tests and strict Clippy pass.


Measurement failures now retain an `incomplete` JSON result, partial sample rows,
exit/forced-termination state and exception class. Cleanup failures are recorded
separately. Exception messages and command arguments are not copied into that
evidence. A completed sampler run is labeled `completed_not_automatically_qualified`: visibility, frame pacing, decoder and host contention still require review.

## Repeated whole-chrome cache comparison at bc2e53f

Four sequential runs used the same release SHA256
`6791c634802d1649b5d4ff26a7c5cfd6d3e1c334527a527760fe26233edcf874`,
in uncached/cached/cached/uncached order. Each used the generated 90-second
H.264 1920×1080 SDR60/AAC48kHz clip, actual VideoToolbox/AVFoundation, requested
1320×860 logical window, 1384×778 physical video target, visible controls and30
explicitly labeled related rows with empty thumbnails. No screenshot, stack
profile, GPU queries or transport timing ran. Each 85-second application lifetime
contained a ten-second warm-up and60 one-second process-tree samples. All local
builds, downloads and other test windows were held; host-other work was audited
from the same process snapshots.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB | Warm VO drops |
|---|---:|---:|---:|
| a1-off | 52.005 / 62.731 / 64.000 | 189.220 / 189.953 / 190.016 | 0 |
| b1-on | 51.926 / 64.001 / 65.000 | 195.080 / 195.859 / 196.062 | 0 |
| b2-on | 52.548 / 63.001 / 64.000 | 194.906 / 195.594 / 195.625 | 0 |
| a2-off | 52.678 / 62.999 / 65.999 | 188.568 / 189.234 / 189.359 | 0 |

Every run exceeded the50% mean CPU release ceiling, despite zero additional
VO/decoder drops during the warm interval. Whole-chrome caching showed no
repeatable CPU benefit and added roughly6MiB RSS; it remains off by default.
All four exited cleanly without forced termination or observed occlusion.
Two video targets used8,614,016bytes in each run. Catalog changes stayed zero
and resets stayed at the single explicit fixture initialization; no video-driven
model reset occurred. This is useful repeated pacing evidence for this local clip,
but it does not qualify performance-supported release,30-thumbnail library use,
other codecs, perceptual A/V synchronization, energy or the60-minute soak.

The host audit is dominated by WindowServer (about42–43% of one core in the
first two runs) and coreaudiod (about6–7%); these shared services are not added to
the owned application tree or presented as exclusively attributable. Finite
sampling misses very short-lived work. Aggregate RSS may double-count shared
pages and is not macOS footprint. Media target bytes are separate allocations,
not an additional amount blindly added to unified-memory RSS.

[Summary and provenance](evidence/2026-09-29-abba-summary.json) link the exact
binary/fixture/sampler hashes, order, post-run power settings and host summaries.
The four `2026-09-29-abba-*.json` files retain full samples and accompanying logs
retain decoder, lifecycle and model counters. Later UI/GPU-diagnostic changes are
not covered by this frozen-binary series. The next smaller optimization candidate
is documented in [search-cache.md](search-cache.md).


The later search-only cache comparison at `e1725df` also produced no repeatable
CPU improvement; all four warm frame-loss deltas were zero but mean CPU remained
above the release ceiling. Full mean/p95/peak and per-second observations are in
[search-cache.md](search-cache.md). The updated phase-continuous standalone
reference measured39.42% mean CPU and163.46MiB mean RSS, with no additional warm
drops; see [standalone-baseline.md](standalone-baseline.md). The minimal headed
browser measured23.45% and1,029.08MiB respectively, with zero quality-counter
drops; see [browser-baseline.md](browser-baseline.md). Different surrounding UI,
shared/temporal service attribution, browser PID auditing limits and independent
run conditions prevent treating differences as exact per-stage costs. None of
these observations qualifies superiority, energy use or actual scanout/A-V sync.


## Related-card cache comparison at 5094c39

The later off/related/related/off comparison retained zero additional warm
VO/decoder drops in all four runs, with actual VideoToolbox decoding and no
observed occlusion. Mean whole-tree CPU was **53.987/48.010/53.281/51.202% of one
logical core**; mean aggregate RSS was **188.939/193.285/193.080/189.059 MiB**.
One cached mean passed the 50% ceiling and the other failed; all exceeded the
25% target. Caching remains off because the gain and ceiling pass were not
repeatable. It increased pair-mean RSS by approximately 4.18 MiB, without an
independent measurement of Slint layer allocations.

This frozen release used the same 30 labeled text rows with empty thumbnails,
a local 1080p60 clip and 60 samples after ten seconds of warm-up. Host-other
activity varied, and bounded external display-awake fixtures were present;
this is neither a power result nor validation of application sleep prevention.
An earlier occluded/paused attempt is retained and excluded from playback
results. Full p95/peak, counters, provenance and limitations are in
[related-cache.md](related-cache.md) and its
[hashed evidence summary](evidence/2026-09-29-related-cache-summary.json).
The optimized release gate remains open.

## Initial cosmetic-clock scheduling comparison — 2026-09-29

The frozen `7a4c9bf` off/on/on/off comparison measured mean CPU
47.062/53.733/52.498/51.912% of one logical core and mean aggregate RSS
189.441/189.483/189.523/189.554 MiB. Each run sampled 60 seconds after ten seconds
of warm-up. All warm intervals added zero VO/decoder drops; no occlusion was
observed. Three CPU means failed the 50% ceiling and all missed the 25% target.
The initial experiment did not establish a CPU benefit and remains off.
[Full values, frozen hashes and raw evidence](clock-staging.md) include separate
UI draw/media notification counts, host activity and diagnostic limitations.
The new scrollable watch layout changes video target dimensions, preventing an
exact shell-overhead subtraction from the older standalone/browser geometry.
The position-cache correction in `9c70e8e` was measured separately below;
its results must not be substituted for these initial measurements.

### Corrected position-cache comparison

Source `9c70e8e`, release `541f4cef58a9efb3e7f366a1b1b44938df9ea1cbcb05d4b7ee2ac0449575f84b`,
repeated off/on/on/off with other experiments disabled. Mean CPU was
53.106/45.250/51.548/53.132% of one core and RSS
189.391/189.301/189.215/189.159 MiB. Both staged warm intervals had 3720 UI
draws, compared with 3949/3946 baseline draws; all had 3600 media notifications
and zero additional warm VO/decoder drops. The correction reduces those UI draws,
but one staged mean still fails the 50% CPU ceiling. It stays off. Complete
p95/peak values, sample scope and host variation are in
[clock-staging.md](clock-staging.md). This does not establish partial GPU redraw
or a power benefit. The separate stable-target functional tests are not this
CPU comparison and are not resource qualification.


## Stable admitted-target comparison — frozen 83a07b4/C2

Release SHA256 `c2ff7d1c0f93145713e2de039e9aa6b574051afcba5d07ed331650c025568398`
used 115 source inputs verified against `83a07b4b493a7d411fc21fdbc55daf529596be7e`.
Central validation reported 243 Rust tests passing, three ignored, 63 Python
tests, strict all-target workspace Clippy, formatting and a locked release build.
The newer working-tree settings/soak implementation is not measured by this series.

With clock staging and all UI caches off, the off/on/on/off stable-target series
measured mean CPU **54.385/44.431/54.146/53.724% of one logical core** and mean
RSS **189.306/189.365/189.311/189.352 MiB**. Each run sampled 60 intervals after
ten seconds of warm-up on the same local 1080p60 H.264/AAC clip, 1320×860 logical
geometry, dark theme and 30 labeled related rows. Warm UI draw counts were
3952/3945/3937/3958; media notification deltas were 3600 each. Warm VO and decoder
drops were zero, with unchanged startup/final VO counts 3/3/2/3. All exited 0
without forced termination or occlusion. No screenshot/readback, profiler or GPU
query was enabled.

Stable mode reduced image publications from 5075 in each A run to one in each
B run. Every run still allocated two persistent targets totaling 7,936,128 bytes.
The B1 CPU improvement did not repeat in B2: three means fail the 50% ceiling,
and all exceed the 25% target. RSS alone passes the playback memory target but
cannot pass the combined optimized gate. Stable mode stays off. Separate mean
host-other CPU was 60.776/84.895/57.777/60.339%; this is context, not a causal
explanation for B1. The sampler's temporal VideoToolbox attribution, short-lived
process visibility and shared-RSS accounting limits remain unchanged. Unified
GPU memory is not added to RSS. External bounded display-awake fixtures establish
neither application power management nor energy performance.

[Complete table, raw samples, hashes and limitations](stable-video-target.md)
retain this comparison separately from the 9c70e8e corrected-clock series. The C2
motion snapshots, guest refresh, caption/clear and compact-library checks are
functional evidence, not additional performance samples or real-account tests.

## Raster library idle: retained failures and neutral startup focus

The `ca21de92` release first measured the actual 10,000-item local database and
30 raster files at a requested 1320×860 logical window. Only **six thumbnail
rectangles intersected the viewport**, with nine decoded near-viewport images
and 100 bounded model rows. After ten seconds of warm-up, the settled run's
60 samples measured 1.248202% mean CPU, **failing the 1% release ceiling**, even
for this smaller workload. Its 130.656 MiB RSS does not turn that into a pass.
It ended with 162 UI draws, nine thumbnail publications and one catalog reset.

The accompanying old minimized run is **invalid**, not a minimized-idle result.
The full log records restoration at 60.345 seconds, final native minimized=false
at 85.001 seconds, three catalog resets, 45 image starts/publications and exit 1.
All 60 collected resource samples are retained, but the workload changed during
measurement. The cause of the restoration/extra work was not established.

The later ordinary browsing release `d7c58acd0096c7b767377f29f423e1ed79668c2a`
uses neutral startup focus rather than automatically editing Search; explicit
Search still has its normal caret, and slash/Tab navigation remains available.
Executable SHA-256 is
`6d982bb8c297d80637327e22f7aed4354a7981a0d559982106157df607ebef52`.
Its 133 selected build inputs matched that commit and were identical before and
after the locked release build. The new sampler first awaited the explicit
drained-fixture marker, then warmed for ten seconds and collected 60 samples.
This was **ordinary Slint browsing, without the native-child presenter flag**;
the local artifact directory's name does not change that scope.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB | Separate footprint mean / p95 / peak, MiB |
| --- | ---: | ---: | ---: |
| Old settled — CPU FAIL | 1.248202 / 2.000083 / 2.999811 | 130.656 / 130.656 / 130.656 | Not sampled |
| Old minimized — INVALID | 1.816920 / 5.000358 / 29.864481 | 130.853 / 135.250 / 137.656 | Not sampled |
| Neutral-focus settled | 0.016667 / 0 / 1.000029 | 129.933 / 129.922 / 130.250 | 147.606 / 147.595 / 147.923 |
| Neutral-focus minimized | 0 / 0 / 0 | 130.073 / 130.063 / 130.391 | 147.731 / 147.720 / 148.048 |

Whole-log inspection verifies that both new runs retained generation 3, 100
model rows, nine near-viewport images, nine publications, nine catalog changes
and one reset from readiness through final reporting. No subsequent page/image
work was logged; remote thumbnail starts and media file loads were zero. Each
sample contained one application process. Both sessions recorded only six UI
draws in total and zero video draws. The minimized run became occluded at
1.657 seconds, had no later expose/restore event, and confirmed native
minimized=true at 85.075 seconds immediately before quit. Both applications
exited zero, and their supervisor groups were confirmed absent.

The new values meet the numeric CPU/RSS targets **for this six-thumbnail
workload**, but do not close the approximately thirty-visible-thumbnail gate.
The minimized intersection count describes retained layout geometry, not images
physically visible while hidden. Zero sampled CPU means zero observed deltas in
the sampler's quantized process counters, not zero actual work. Both new runs
have all 60 physical-footprint readings available, with one consistent native
process-start identity. This separate OS ledger is neither unique physical
memory nor a GPU allocation; never add it to RSS or inferred texture bytes.

These are single runs on different builds with additional readiness/footprint
instrumentation, not a controlled single-variable A/B or a power measurement.
The reduced redraw count is consistent with removing unsolicited search-caret
activity; no exact causal CPU subtraction, energy benefit, GPU-texture inventory,
repeatability, larger-display or other-platform qualification follows.

[All four full sample payloads, native/sampler logs, frozen source and fixture
provenance, and original/export hashes](evidence/2026-09-29-raster-idle-export.json)
are public. Only private workspace roots in harness/traceback records were
normalized; samples and counters were not changed. Manifest SHA-256:
`562b9825759a60b571b285895fc2f15c645a32e406a49e454c4afbfb210b5ffd`.


## Native-child comparison — corrected a45ff676

A single frozen release from `a45ff67692e0f25e22c188dc1583c3490d94a542`
(SHA256 `0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`)
completed default/native/native/default, following successful local and two
14-stage native lifecycle checks. All135 committed build inputs were identical
before/after build. The installed libmpv and exact synthetic90-second H.264
1080p60/AAC48k fixture remained unchanged and hashed before/after every run.
No build, screenshot, profiler or competing application test ran during sampling.

Every run warmed10seconds and retained60 one-second process-tree samples.
Actual video geometry matched:2640×1528 window backing pixels at scale2,
664×373.5 logical video (1328×747 backing); native geometry did not change in
its sampled interval. Playback stayed visible, unmuted at volume100 and speed1,
using observed `videotoolbox` and AVFoundation; exactly one load and zero
unrelated catalog notifications were observed. All four exited0 without forced
termination, with owned process groups confirmed absent. The native candidate
still has its diagnostic warning and30 labeled empty-thumbnail related rows;
this is neither pixel-identical chrome nor the raster-library acceptance case.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB | Separate footprint mean / p95 / peak, MiB | Slint draws,10–70s |
| --- | ---: | ---: | ---: | ---: |
| A1 default | 50.658 / 63.002 / 64.405 | 189.313 / 189.938 / 189.969 | 318.788 / 319.596 / 319.627 | 3838 |
| B1 native | 46.437 / 54.210 / 56.001 | 189.863 / 190.766 / 190.813 | 312.799 / 313.674 / 316.627 | 240 |
| B2 native | 43.568 / 55.001 / 56.000 | 189.656 / 190.703 / 190.828 | 327.914 / 328.877 / 329.080 | 239 |
| A2 default | 48.784 / 61.393 / 61.932 | 189.265 / 189.938 / 189.984 | 318.448 / 319.377 / 319.455 | 3839 |

All warm VO/decoder drop deltas were0; startup VO drops2/3/3/2 are retained.
Native publication counts advanced3600/3599 during the approximate10–70second
checkpoint interval. These are engine/publication counters, not proof of each
physical display presentation or perceived A/V synchronization. The checkpoint
and external sampler clocks are not atomically aligned.

The native surface substantially reduces Slint draws, but both native runs
**miss the25% playback CPU target**. Their means fall below the50% ceiling on
this workload; A1 default exceeds it. These four observations do not qualify
production playback or establish an energy benefit. Host-other mean CPU was
50.38/51.25/60.19/49.19% of one core (see exact raw samples for variability);
no causal attribution to individual background processes follows. WindowServer
appeared in all60 top-eight samples per run and averaged26.714/30.998/30.298/
27.258%; this shared compositor observation prevents treating reduced app CPU
as an equal system-wide reduction. Both native
footprint means remain below400MiB but differ by15.1MiB despite near-identical
RSS; do not infer an allocation reduction from one pair.

Footprint readings were available for all240 samples. Each sample attributed
the app and one temporally associated new VT service; preexisting services were
excluded even if reused. RSS can double-count shared pages; physical footprint
is a separate OS ledger, not unique memory or GPU usage, and is never added to
RSS or the two-target7,936,128-byte estimate. WindowServer cost, total graphics
allocations and energy remain unmeasured. The comparison's linkage preflight
checks installed-library identity; it is not a loaded-image inventory.

The candidate stays opt-in and exact-local-fixture-only. Tooltips and all shared
overlays, subtitles, hit testing, DPI changes, stale-frame/account exclusion and
perceptual A/V still need qualification before broader admission. See
[native presenter](experiments/native-video-child.md). Raw evidence is retained
under `artifacts/native-child-v2/abba`; [the public export](evidence/2026-09-29-native-child-abba-export.json)
contains all240 samples, exact logs and source/provenance with original/export
hashes. Private harness paths and host identifiers are explicitly sanitized.

## Native-child matched mpv pass-timer experiment

The private timer ON/OFF libraries were applied to separate copies of the same
a45ff676 native-child application, preserving the original app and installed
library. Before measurement both copies passed14 native lifecycle stages, exact
loaded-library/closure checks and byte-identical paused-frame captures. A quiet
ON/OFF/OFF/ON comparison then retained all240 samples after10-second warm-ups;
all four runs exited cleanly with owned process groups absent.

| Run | CPU mean / p95 / peak, one-core % | RSS mean, MiB | Separate footprint mean, MiB |
| --- | ---: | ---: | ---: |
| A1 timers ON | 44.759 / 55.001 / 57.001 | 189.747 | 319.574 |
| B1 timers OFF | 36.665 / 45.000 / 46.009 | 188.136 | 317.865 |
| B2 timers OFF | 36.026 / 45.000 / 46.999 | 187.706 | 310.105 |
| A2 timers ON | 44.541 / 54.999 / 55.811 | 189.525 | 327.855 |

Warm VO and decoder-drop increments were zero throughout. OFF's mode mean was
36.345% versus44.650% ON, an observed8.304 percentage-point reduction. Both OFF
repeats remained above the25% target. Shared WindowServer CPU was higher in OFF,
so this does not establish an equal system-wide or energy reduction. The fixture
still had30 empty-thumbnail related rows and only the restricted local media
path. No production patch was adopted. Existing process attribution, shared RSS,
separate footprint, unmeasured GPU allocation and A/V limitations apply.

See [full measurements and exact identities](experiments/mpv-pass-timers.md#native-child-matched-onoffoffon-resource-result)
and the [34-payload export](evidence/2026-09-29-native-timer-abba-export.json).
