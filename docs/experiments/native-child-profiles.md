# Matched native-child thread profiles

Two diagnostic `/usr/bin/sample` runs completed on frozen a45ff676 release
`0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`.
Each app ran for28seconds in its own fresh private profile; one five-second
sample requested1ms between observations after12seconds. Default ran first,
then native, never concurrently. Both apps and profilers exited0; owned app,
profiler and bounded display-assertion groups were reaped and confirmed absent.
No screenshot was taken. Other-agent compilation was permitted, so these are
**not resource or cadence benchmarks**.

The [sanitized aggregate export](../evidence/2026-09-29-native-child-profiles-export.json)
retains useful stack counts, complete nonsecret app logs, supervision and
binary/tool/raw-profile hashes. Manifest SHA-256:
`91c71b18d8bab8f8f80e0e927001fe156ca5a73ceb683f6d0a93adff0551fcde`.
Large raw profiles remain private under `artifacts/native-child-v2/thread-profiles`.

Both10- and25-second observations retained direct VideoToolbox H2641080p60,
AAC48kHz/AVFoundation, Playing, unmuted volume100, no occlusion and matching
2640×1528 window geometry at scale2 with664×373.5 logical video. Nevertheless,
VO drops rose21→291 default and49→196 native; decoder drops stayed zero.
These instrumented runs permitted foreign compilation; the individual cause of
the drop increase was not isolated. They cannot replace the separate
unprofiled ABBA measurements.

Both sample Binary Images lists identify `libmpv.2.dylib` UUID
`1869BFE9-CAA0-373D-8FA2-DAC87D0CC5DF`. An independent `dwarfdump --uuid` of
installed arm64 mpv0.41.0_10 matched it; that library's SHA-256 is
`12c400d4ec2740ebf225c3965f4e9963c55de9212094f1337d055b0573812e08`.
This is loaded-image evidence for these two profiles only; it does not silently
add image enumeration to the earlier ABBA.

## What the stacks establish

Main-thread trees contain961 observations default and789 native, despite the
same requested interval. These are wall-stack residence observations, **not CPU
percentages, call counts, or directly comparable execution durations**. Inclusive
subtrees overlap and must not be added. Leaf residence was computed as a node's
count minus its immediate children; totals were checked against each thread root.

| Selected main-thread subtree | Default observations | Native observations |
| --- | ---: | ---: |
| `mpv_render_context_render` (all disjoint occurrences) | 230 | 316 |
| `gl_timer_stop` | 91 | 113 |
| Timer-stop leaves at explicit wait/IPC boundaries | 78 | 98 |
| Native `serein_child_flush` | 0 | 197 |
| Native child-flush semaphore-wait leaves | 0 | 128 |
| `glTexImage2D_Exec` | 13 | 1 |

Both modes show the concrete path
`timer_pool_stop → gl_timer_stop → glEndQuery_Exec → gldGetQueryInfo →
GLDContextRec::flushContext → Metal command submission → IOKit/Mach IPC`.
The native child's flush additionally reaches
`CGLFlushDrawable → glSwap_Exec → gldPresentFramebufferData → flushContext →
semaphore_wait_trap`, and a separate path reaches WindowServer IPC. A kernel
boundary can include actual kernel work as well as waiting; the remaining
leaves are not automatically CPU-active either.

The selected CVDisplayLink thread had960/961 and788/789 observations at explicit
wait/IPC boundaries. The audio converter was961/961 and789/789. Decoder worker,
VO and demux threads were also predominantly at those boundaries. This rules
out interpreting their large inclusive thread-root counts as equivalent busy
CPU time; it does not measure their precise CPU contribution. No decoder
service or shared compositor thread was profiled here.

## Source-backed interpretation and next step

mpv0.41 [timer stopping](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/ra_gl.c)
ends `GL_TIME_ELAPSED` around renderer passes. Reusing its eight-query ring reads
`GL_QUERY_RESULT` without an availability check. The profile contains only two
named result-getter observations default and none native; this neither proves
that read harmless nor identifies it as the dominant cost. The observed driver
submission at `EndQuery` is stronger evidence for the existing
[isolated pass-timer experiment](mpv-pass-timers.md) than another unexplained
application-clock tweak. Removing queries could simply move submission to the
later draw/flush; the earlier default ON/OFF comparison did not pass the CPU
ceiling repeatably.

Native `serein_child_flush` calls `NSOpenGLContext.flushBuffer` on the retained
main-thread context. Its wall-time and semaphore residence do not justify
removing synchronization, changing swap policy, presenting early or moving the
context to another thread. No such source changes were made.

`xcrun --find xctrace` resolved the installed Xcode tool. Installed record/export
help supports a targeted `Time Profiler` attachment, finite5s limit and TOC
export. A bounded owned-app native attempt is prepared, not executed in this
record: attach only the known PID after observed playback, use a new private
trace, avoid all-process recording and privacy-prompt suppression, retain failures,
and kill/reap only anchored owned process groups on timeout. First inspect actual
CPU-aware sample schemas/weights before authorizing a default comparison. Tool
presence does not establish attachment permission or useful trace contents.
Only after that attribution should the existing isolated timer ON/OFF candidate
be retested on the native path; no production patch or expected saving is claimed.

## Subsequent targeted Time Profiler recordings

Installed Xcode27.0 Time Profiler successfully attached to one owned native
process, then a separate default process, using the same frozen app/clip/window.
Each app had a35-second lifetime and a new private profile. The recording command
started after the ten-second Playing checkpoint, requested a5s limit and saved
under a new private output directory. Both apps, recorders and exports exited0;
all anchored owned groups were absent after reap. No prompt was suppressed, no
all-process recording was requested, and no TCC setting was changed. Raw traces
and TOCs contain automatically captured host/environment metadata and remain
private. Only whitelisted aggregates and nonsecret application logs are exported.

The [CPU-profile export](../evidence/2026-09-29-native-child-cpu-export.json)
has SHA-256
`58051c7c8585772c49ffb9050078a2d570d4f7746d0834039d849aaa2feefbac`.
It includes exact raw-XML and private trace-bundle-manifest hashes, tool/harness
hashes and the post-run analyzer. Each bundle manifest binds sorted relative
file records with byte count and SHA-256; its public digest does not expose
private trace contents.

Both10- and25-second observations remained Playing with VideoToolbox1080p60,
AAC48kHz/AVFoundation and the same normal geometry. Cumulative VO drops stayed
3→3 and decoder drops0→0 in each run. These were instrumented diagnostics with
concurrent builds allowed, not additional resource measurements.

The exported `time-profile` schema disables waiting-thread recording; its raw
CPU sampling table specifies1,000µs and `all-thread-states=NO`. Every exported
row resolves to the exact attached application PID and is labeled **Running**
with1ms statistical weight. Kernel call stacks were disabled. Consequently,
user-side kernel-entry symbols do not expose internal syscall work or prove that
their entire weight is CPU-active rather than attribution at a state transition.

| Disjoint stack category | Native statistical weight (ms) | Default statistical weight (ms) |
| --- | ---: | ---: |
| libmpv render subtree | 750 | 136 |
| Native child flush subtree | 179 | 0 |
| Slint renderer, excluding nested media/child flush | 78 | 318 |
| Other main-thread work | 33 | 37 |
| Other threads | 766 | 185 |
| Missing backtrace, retained explicitly | 3 | 0 |
| Total exported Running weight | 1809 | 676 |

These categories are a partition of the exported rows, **not exact measured CPU
time or a performance comparison**. Native main-thread weight totaled1040ms;
its core/audio-IO/CVDisplayLink weights were158/120/2ms. Default main totaled491ms,
with32/7/1ms respectively. Nested timer-stop weights were263ms native and44ms
default; draw weights were298/66ms. These inclusive values overlap their parent
categories and cannot be added. Result-query getters had4/1ms weight. The data
establishes actual query/draw/driver paths within targeted Running samples; it
does not establish removable cost or justify a percentage-saving claim.

The2.7-fold difference in total exported weights conflicts with treating these
short traces as comparable CPU totals. The
[sampling audit](../evidence/2026-09-29-native-child-cpu-sampling-audit.json)
checked exact target identity, unique XML definitions, monotonic timestamps,
no duplicate(time,thread) pairs and uniform weight. Native sample times span
0.281–5.786s in a5.788s recording; default spans0.298–5.748s in5.756s.
Their one-second row buckets were106/527/345/344/325/162 and
104/119/122/123/108/100. Thus the difference persists through the recordings,
rather than simply reflecting a truncated export. Recorder/TOC/CPU-export logs
reported no warning/error/lost/dropped entries, but no loss-status table was
exposed. Absence of such messages is **not proof of no sample loss**. No cause
is assigned to the discrepancy, and no mode ratio or CPU-percentage conclusion
is taken from it.

## Read-only review of the native timer-OFF candidate

The existing diagnostic patch changes only `timer_pool_create` to return null
when its explicit build option is false. mpv0.41's pool start/stop/measure/destroy
functions already accept null. Raster/compute dispatch still occurs, and this
patch does not select a different decoder, shader, scaler, output size, frame
clock or application UI. The native child reaches the same legacy libmpv OpenGL
renderer as the default presenter; the observed query paths confirm that this
candidate applies to it. Driver submission may nevertheless move to the later
flush instead of disappearing. Native `flushBuffer` synchronization must remain
unchanged for this experiment.

The private ON/OFF builds already recorded in
[the timer experiment](mpv-pass-timers.md) are the appropriate matched pair:
`fc873c29f7f7cac23f419c7030c9351ee70a4662f4d257f974f87a7a1bca93c5`
and `895e653209230e86dd46f58ffe8a8ab983c8686308ef4383830ff65f41e4224e`.
A future native experiment must use newly hashed, explicitly derived app copies
linked to each corresponding library, preserve frozen0734 unchanged, recheck
actual loaded images and native functional/pixel behavior, then compare unprofiled
resource runs. OFF versus the separately built Homebrew bottle would conflate
the timer switch with build differences. No new copies, patch, library build or
timer-OFF launch were made in this review. The earlier default-path ON/OFF series
failed repeatable ceiling acceptance; these profiles do not override that result
or make the diagnostic option an upstream production feature.
