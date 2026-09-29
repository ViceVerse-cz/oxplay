# Video integration — experimental macOS path

The latest shell revision moves description/comments into the outer watch-page
scroll and retains the persistent video host. Custom chrome and optional alpha/
blur reuse the same window and presenter; main content/video stay opaque. These
appearance changes need separate native composition/lifecycle qualification.

The latest [control styling](picture-in-picture.md) changes only shared UI and
a cached total-duration label. It preserves the player/presenter lifetime and
existing seek/volume commands. Native subtitle overlap at the new bottom-edge
transport position has not been revalidated; earlier native evidence applies
to its recorded source, not this visual revision.

The in-app [local media picker](local-media.md) now routes explicit selections
through the existing stop barrier before loading the same player. Entry-scoped
[local demux restrictions](local-media-policy.md) supplement worker header checks.
[Chapter navigation](video-chapters.md) and [Jump to time](time-navigation.md) use
exact native-load ownership and existing asynchronous seeks. These additions
have not yet received native interaction or resource qualification.

The [picture-in-picture mode](picture-in-picture.md) reuses this window's
existing presenter and decoder. Its nine-stage local lifecycle test preserved
native window, media-load and presenter identities across compact resize and
restoration; no cross-window borrowed-texture sharing was introduced. macOS
floating-layer and subtitle observations are recorded separately from resource
qualification, which remains paused and incomplete. The normal presenter now
composes shared transport controls inside the video rectangle; borderless PiP
uses the same context and restores the original native frame on exit. The
experimental native child keeps its reserved diagnostic control strip.

The current working tree embeds libmpv inside the compiled Slint window. This is
a functional spike, not a passed optimized/release gate. There is one player,
one Slint window, one rendering context, and two persistent RGBA8 textures. No
application screenshot loop, CPU pixel conversion, SharedPixelBuffer video path,
or external player window is used.

The default borrowed-texture path described here remains the product presenter.
A separate, explicitly restricted macOS native-child diagnostic owns its own
OpenGL context and renders only the verified local test fixture; it retains the
same Slint controls. On d78332c its16-stage lifecycle passed, including readable
message suppression/reveal, paused fullscreen transitions and teardown. This is
not arbitrary-media, subtitle, tooltip or A/V qualification. See the
[native-child experiment](experiments/native-video-child.md) and
[current source-associated results](progress.md); historical results below
retain their original binary and workload scope.

## Current macOS scheduling candidate

A functioning same-engine SDL/OpenGL baseline now isolates the pacing failure
more narrowly than the earlier environment hypothesis. Native CGL swap interval 1
reproduces severe warm frame drops, while native interval 0 with CVDisplayLink
synchronization delivers 60 fps. Disabling native swap waiting without the display
clock also fails. The nonblocking bounded-clock diagnostic had zero warm drops
between 10 and 25 seconds. Its source, license provenance, commands and differential
results are in [the experiment record](experiments/native-swap.md).

The current candidate in `crates/media/src/macos.rs` uses a native display clock
during observed Playing and restores the original CGL swap interval at
teardown. It starts on an engine notification and preserves phase through short
gaps; it stops directly on pause, hidden/buffering/ended state. Empty display
ticks never call mpv or wake/redraw the UI. The earlier two-empty-refresh stop
heuristic was removed after a late-run instability. The revised policy passes
the 20-second native lifecycle, including hidden clock stop; release throughput
and resource requalification remain pending. A paused final frame retains one-shot readiness while
its clock is stopped, including when the hidden window cannot draw. The callback only sets
atomic flags and coalesces a UI wake; GL remains on the owning Slint thread.
The macOS timing lead is now the upstream 50 ms default; other unqualified
platforms retain the initial behavior. Old zero-lead/ring/clock experiments below
are historical comparisons, not current configuration claims.

Seven media tests pass, including empty display ticks without UI wakeups,
callback coalescing, and file-scoped companion-track isolation;
strict Clippy passes. The first full-application 30-second run now observed zero additional warm
VO drops (3 startup drops unchanged from 5 seconds through exit), active
VideoToolbox, 900 notifications between 10 and 25 seconds, and clean teardown.
Related rows were empty in that run. A subsequent 60-second sample retained zero
warm drops but averaged 52.79% of one logical CPU, above the 50% release ceiling;
RSS averaged 185.77 MiB. The awake fixture-list lifecycle passes paused seek and
hide/restore with the hidden clock stopped. Display-sleep setup failure was
isolated using public CoreGraphics state queries and a bounded diagnostic wake.
A scoped idle-display-sleep assertion now follows observed Playing state and
is released on pause/hidden/ended/teardown. Native pmset and snapshot checks
observed it active during Playing and absent while hidden/paused and after
teardown. A controlled initially-asleep launch subsequently recovered on a native
wake event without restarting, as recorded below.
Mean render-start lateness 27.8 ms still requires A/V validation. No optimized-gate or A/V qualification is inferred from the standalone
harness's frame count, and its display callbacks are not actual scanout feedback.
CVDisplayLink is deprecated in the macOS 15 SDK; the isolated adapter records
that dependency rather than assuming future OS support.

## Observed configuration and scope

Bootstrap date: 2026-09-29. Initial application revision: `e945b3e283e9eea79b6fba136645cb9191147dd2`;
subsequent scheduling changes are tracked separately in ADR 001 and performance evidence. Slint runtime/compiler:
`cf3b07d4917e6759a63b0c03913a2594ec653414`; locked FemtoVG 0.27.0; glow 0.18.
Available host: Apple M1, 16 GiB, macOS 27.0 (26A428). Runtime graphics diagnostic:
`Apple | Apple M1 | 4.1 Metal - 91.7` (exact line retained by the run harness).
The selected window backend is Winit; the selected renderer is FemtoVG/OpenGL.
A NativeOpenGL rendering notifier and GL vendor/renderer/version checks confirm
the graphics API independently of Cargo feature selection.

Homebrew mpv 0.41.0_10, libmpv client API 2.5, FFmpeg 9.0.2, libplacebo 7.360.1.
The media adapter uses libmpv's OpenGL render API; the linked media package also
contains other backends. That does not select those backends in this application.
Homebrew dependencies are development prerequisites, not a redistributed bundle.
Display dimensions, scale/refresh, thermals, and power mode must accompany each
resource run; the initial functional observation alone is not a benchmark.

The local deterministic generated clip is H.264, 1920×1080, 60 fps, yuv420p, with
AAC audio and an explicit SRT fixture. The native lifecycle run observed
`hwdec-current=videotoolbox`, codec H.264, width 1920, height 1080, and fps 60.
Hardware use is observed, not inferred from `hwdec=auto-safe`. Decoded video and
playback position advance inside the native UI. Pause, seek, resize, fullscreen,
subtitle cycling, and teardown are exercised by the harness. Auditory A/V sync,
subtitle legibility/color accuracy, minimize/restore, and a full soak required
separate validation at that stage; automated command submission alone is not proof.
Later local minimize/restore and the complete mixed-use hour are recorded in
[soak-local.md](soak-local.md). That run passed its observed lifecycle assertions,
but ongoing RSS growth and untested remote/account/fallback workloads keep the
full soak gate open.

Initial debug lifecycle runs showed substantial startup/seek dropped frames
(including 87 at an early checkpoint), so they do not pass the dropped-frame or
resource gate. Full texture-unit state queries were subsequently reduced using
the reviewed FemtoVG/libmpv state contract. Release measurements must qualify the
result; no efficiency improvement is claimed without comparing repeated runs.

## Source-inspected transfer path

```text
H.264 packets
 -> FFmpeg / VideoToolbox decoder
 -> CVPixelBuffer backed by IOSurface
 -> CGLTexImageIOSurface2D imports each plane as GL_TEXTURE_RECTANGLE
 -> libmpv GPU color conversion/scaling/subtitle composition
 -> application GL_RGBA8 GL_TEXTURE_2D framebuffer (alternating persistent pair)
 -> Slint borrowed texture / FemtoVG native-texture sampling
 -> Winit OpenGL surface swap -> OS composition -> display
```

| Boundary | Classification and evidence |
|---|---|
| Compressed packet parsing to decoder | CPU packet/demux work; actual decoder property reports videotoolbox |
| Decoder to CVPixelBuffer / IOSurface | Hardware decoded surface; internal VideoToolbox/driver allocation/copy behavior is not traced |
| IOSurface planes to GL | GPU-resident import in mpv 0.41.0 `hwdec_mac_gl.c`; retains CVPixelBuffer, gets IOSurface, imports with CGLTexImageIOSurface2D |
| YUV planes to RGBA target | GPU sampling/conversion/scaling; writes a separate RGBA render target, therefore not an end-to-end zero-copy claim |
| RGBA target to Slint | Borrowed native GL texture in the same context, sampled during UI composition; no application CPU readback/re-upload |
| Surface/compositor/display | GPU presentation expected, internal driver/compositor copies and energy cost unmeasured |

Relevant exact-version upstream sources:
[VideoToolbox adapter](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/hwdec/hwdec_vt.c),
[macOS CGL import](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/hwdec/hwdec_mac_gl.c),
[OpenGL render backend](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/libmpv_gl.c),
[GL resource/state use](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/ra_gl.c),
[render contract](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/render.h),
[GL contract](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/render_gl.h),
[Slint borrowed image](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/core/graphics/image.rs),
[FemtoVG rendering lifecycle](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs).

This source trace plus the active decoder property supports investigating a
GPU-resident macOS path. It does not measure GPU-to-GPU conversion cost, prove
absence of opaque driver copies, qualify color or power, or pass the optimized
gate. No Windows, Linux/X11, or native Wayland runtime has been exercised.

For the tested H.264 clips, the active-decoder observation has an additional
codec-specific source check: [FFmpeg 9.0.2's decoder](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/videotoolbox.c#L836-L854)
requires hardware acceleration when creating the VideoToolbox session. The
installed libmpv links Homebrew FFmpeg's libavcodec.63; its unpatched 9.0.2 build
enables VideoToolbox. Together with produced H.264 frames and the active method,
this supports hardware decoding for these clips. It does **not** generalize to
HEVC/ProRes: those branches only enable acceleration and permit an internal
software fallback. This application has not directly queried the private VT
session's UsingHardwareAcceleratedVideoDecoder property. Copy behavior remains
a separate question from decoder selection.

## Ownership, timing, and synchronization

`crates/media` owns all mpv/GL handles and unsafe FFI. `Player` uses an Rc owner,
so the presenter retains the engine until its render context is freed. Both are
UI-thread-only. All post-initialization player commands are asynchronous;
properties are observed or requested asynchronously. No synchronous property
getter runs alongside rendering. The render context is created before loading
media: the first attempt loading before RenderingSetup produced audio without
video and was corrected.

Engine callbacks only set atomics and request coalesced scheduling. They never
render or mutate Slint. On macOS, an active display clock supplies the frame-ready
wake, avoiding a redundant UI wake for each decoded frame; a stopped clock is
armed by the next engine wake. `drain_events()` calls `mpv_wait_event(...,0)` to
exhaust the engine queue and services native clock start/stop. BeforeRendering
calls `mpv_render_context_update` and renders only when a frame is pending or
the physical target size changes. No permanent frame polling timer is used.
Progress reads are separately requested by the visible/playing UI at 4 Hz;
the elapsed string changes only at displayed-second boundaries. Absolute seeks
allow one in-flight command and one replaceable latest position. Other commands
are bounded to 64 pending replies. libmpv's own event queue is bounded and an
overflow becomes a visible failure.

The macOS candidate uses `video-timing-offset=0.05`, audio synchronization,
native display-clock gating, and `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME=0`.
No wait for a future video deadline runs on the UI thread, and the legacy Slint
deadline timer is bypassed while this native clock is enabled. Other unqualified
platform paths retain `video-timing-offset=0`. Rendering can still consume CPU/GPU
submission time. Advanced control is deliberately disabled: this version does
not promise to service GL update tasks while Slint has an unavailable/hidden
surface. The app's declared visibility policy must pause playback and stop
progress timers when occluded/minimized. This needs platform-specific testing.

Slint's AfterRendering occurs **before** swap/present in this exact revision.
The adapter therefore does not call `mpv_render_context_report_swap` with a
fictional presentation timestamp. Precise compositor cadence and frame timing
remain measurement tasks.

Each texture is allocated only when size changes. Two targets alternate, so
Slint receives a changed borrowed-image identity without full-frame CPU data.
During resize, an old inactive target can be retained until AfterRendering:
Slint has flushed draw commands and drained its texture cache by then. GL's
ordered same-context commands and deferred object deletion protect submitted
GPU work; there is no second GL context or cross-thread texture access requiring
a custom fence protocol. Texture origin is top-left (mpv FLIP_Y=0 and Slint's
default origin). SDR RGBA8 is the only intended target; HDR is not advertised.
Targets larger than 4096 in either dimension are rejected with an explicit
error. Target byte counters include the transient old target before retirement.

The GL guard saves/restores framebuffer, program, VAO/buffer, texture, viewport,
scissor, blend, color mask, pixel-unpack and enable state. The exact locked
FemtoVG renderer uses texture units 0 and 1. The app uses unit 0; mpv unbinds its
own inputs in `ra_gl.c:disable_binding`. Other texture units consequently remain
at defaults and need not incur hundreds of GL queries every frame. This
source-specific invariant must be audited when either dependency changes; no
other custom GL renderer may share this context without expanding the guard.

RenderingTeardown must run even if a weak Slint component no longer upgrades.
The app clears its borrowed Image when available, then destroys the presenter
with the owning context current. The mpv update callback is removed, the mpv
render context is freed, and texture/FBO objects are deleted. Only then can the
engine owner terminate libmpv. Dropping the presenter after the GL context has
been destroyed is forbidden. A former early weak-upgrade return skipped this
teardown and hung shutdown; the corrected notifier handles teardown first.

## Media/network boundary and remaining gates

Inherited mpv configuration, scripts, OSD/OSC, implicit yt-dlp, cookie loading,
external-reference access, subtitle auto-discovery and audio auto-discovery are
disabled. A process sample revealed that `load-scripts=no` alone still starts
mpv 0.41's bundled Lua stats, console, selection, positioning, commands and
context-menu threads. The adapter now separately disables `load-stats-overlay`,
`load-console`, `load-select`, `load-positioning`, `load-commands`,
`load-context-menu`, and `load-auto-profiles`; these names were checked against
the installed `mpv --no-config --list-options`. Earlier footprint observations
include the bundled scripts; reruns of the final configuration must be labeled. Forward/back demux caches are capped at 32/8 MiB. Explicit local paths
and subtitles originate from user choice. Anonymous network loads accept only
HTTPS `googlevideo.com` hosts; the resolver is responsible for stream selection.
Account cookies are never passed to this player. Media redirect/manifest/DNS and
whole-process egress policy still require validation before any comprehensive
proxy or account playback guarantee. Separate audio is added on file-loaded,
not while the new video file is still opening.

Reproduce the local functional slice with:

```sh
scripts/generate-fixture.sh
cargo run --locked -p serein -- --local artifacts/local-1080p60.mp4 --subtitle artifacts/local.srt --smoke-test
cargo test --locked -p serein-media
```

Use the actual binary/package name in Cargo.toml if it changes. Live URL/search
success, guest/authenticated ad behavior, protected account playback, 60-second
release resource samples, standalone mpv/browser baselines, color/orientation,
A/V sync, decoder fallback, seek/resize soak, accessibility, and the additional
platforms are distinct tests; this document does not mark them passed.


## Release timing investigation

Subsequent release runs exposed a real unresolved delivery defect: ordinary
Slint `Window::request_redraw()` produced only a handful of draws despite
hundreds of pending media notifications. The adopted macOS backend routes that
request through an NSView CADisplayLink. The application now requests a Winit
redraw directly on a coalesced media notification on macOS. This remains an
event-driven request, with no permanent polling timer. It restores movement but
does **not** establish correct 60 fps pacing.

A 75-second release run recorded 1,897 UI draw callbacks and 2,589 reported VO
drops, with VideoToolbox active. A separate 25-second profile run recorded 633
draws and 851 drops. These fail the 1080p60 gate regardless of acceptable CPU or
memory counters. Earlier debug observations near 60 fps do not supersede these
release failures. The sample's main thread was mostly asleep, and the mpv VO
thread waited for ordinary VO work in 2,246 of 2,255 stack samples, versus eight
samples waiting for the render callback; GL-state-query overhead is consequently
not established as the dominant cause. Decode contention, event/display timing,
and system scheduling need controlled isolation.

`SEREIN_MEDIA_TIMING=1` enables bounded aggregate wall-time counters for GL-state
save, `mpv_render_context_update`, mpv drawing, and GL-state restoration, plus
maximum mpv draw time. There are no per-frame logs or pixel reads. Totals and
sample count appear in the existing `RenderStats` output at teardown. Timing is
off by default. Snapshots additionally separate raw render notifications,
coalesced wakeups, VO dropped frames, and decoder dropped frames; callback counts
must not be described as actual display presentations.

The installed standalone mpv has no native OpenGL window backend (its
`--gpu-api=help`/context lists expose Vulkan choices). A gpu-next baseline is a
different presentation stack and, on this session, reported inability to start
DisplayLink / `noScreen` and stopped responding to bounded IPC reads. Its
application-owned process was terminated; it produced no valid comparison.
Together with Slint's stalled display-link path this is a material environment
or native-display integration blocker. A working native window and a readable
OpenGL vendor string do not validate a display timing path. No framework switch
or performance-supported macOS claim follows from this evidence.

VideoToolbox also launches a system-managed `VTDecoderXPCService` whose parent
is launchd rather than the application. A PID-descendant-only sampler can miss
that attributable decoder work. Record it separately where attribution is
possible; do not treat the root process RSS/CPU as the complete cost. Unrelated
`mediaanalysisd` activity was observed and must not be killed to manufacture a
favorable measurement.

Further instrumentation narrows the low frame-rate failure: an 85-second run
produced 2,119 render notifications, 2,118 video draws and 2,961 VO drops, with
zero decoder drops. The UI therefore serviced essentially every delivered media
frame; this result does not support blaming Winit throughput for those drops.
The separate DisplayLink failure remains real but is insufficient to explain
this stage of frame loss.

In mpv 0.41.0 `vo.c:vo_is_ready_for_frame`, `video-timing-offset` determines how
early the core may queue a frame. `vo.c:render_frame` drops a late frame before
calling `draw_frame`, hence before the application gets its render notification.
The initial zero-offset choice leaves no scheduling headroom. A controlled
`SEREIN_VIDEO_LEAD_MS=50` diagnostic (bounded 0–100 ms; default was zero in that experiment)
re-enables headroom while retaining nonblocking rendering. The first A/B version
presented without deadline scheduling and could show video early; those results
must not be treated as an A/V synchronization pass. With
`SEREIN_MEDIA_TIMING=1`, next-frame deadline offsets are recorded as aggregate
and maximum early/late microseconds.

Timestamp warning: the installed mpv 0.41.0 `render.h` describes
NEXT_FRAME_INFO.target_time as microseconds, but its `vo_libmpv.c` implementation
assigns `frame->pts`, constructed in nanoseconds by `player/video.c`. The
instrumentation compares against the verified client API 2.5
`mpv_get_time_ns()`. The build now explicitly requires API 2.5 or later. The
scheduler validates plausible deadline bounds. Re-audit this source/header
discrepancy when updating libmpv; blindly treating this value as microseconds
would schedule against the wrong timescale.


The initial 15-second A/B measurements (including startup, not acceptance
samples) produced:

| Render lead | Engine notifications | Video draws | VO drops | Decoder drops | Total GL save time | Total mpv draw time |
|---|---:|---:|---:|---:|---:|---:|
| 0 ms | 543 | 542 | 339 | 0 | 11.2 ms | 1,404 ms |
| 50 ms, then unscheduled | 783 | 782 | 101 | 0 | 14.2 ms | 1,797 ms |

This identifies a material effect of engine scheduling headroom. State saving
averaged about 20 microseconds per frame, so removing required GL-state
preservation would not address the observed lost-frame rate. The 50 ms result
still misses the full-run dropped-frame gate, and startup must be separated
from steady playback before qualifying it.

The current positive-lead diagnostic path now uses proper one-shot deadline
scheduling. On a real engine notification, BeforeRendering queries and retains
NEXT_FRAME_INFO. If the frame is early, the old texture remains visible and one
Slint SingleShot timer signals a distinct `due` atomic when the deadline arrives.
The callback only schedules a UI wake; it never accesses GL. Intervening resize
continues displaying the prior texture until the frame is due. New real media
updates re-query the authoritative pending frame, and canceled frames stop the
timer. Redraw/repeat frames and target-time zero are handled immediately.

Timer delays round up to Slint's millisecond resolution, avoiding a zero-delay
rescheduling loop. Deadlines more than five seconds from the engine clock, or
future deadlines beyond the configured lead plus 250 ms, return a useful error
rather than waiting on an invalid timescale. There is at most one pending frame
and one timer; teardown stops the timer and clears its due flag. The absence of
a new media frame does not restart a recurring timer. This path can require an
additional UI draw to inspect an early frame before the later presentation draw;
that overhead must be measured. Default lead remains zero until the positive
lead path passes its functional/pacing validation.

Four media tests, including deadline rounding, clock-unit rejection and actual
libmpv initialization/async controls, passed with
`cargo test --locked -p serein-media`. Media clippy passed with
`cargo clippy --locked -p serein-media --all-targets -- -D warnings` after the
scheduled path was added. These unit checks do not substitute for the pending
native A/V, pause/seek/minimize and warm steady-state measurements.

The first scheduled 50 ms run is **not** a fix for the pacing gate. Between its
10- and 25-second checkpoints VO drops increased from 190 to 454: 264 drops
among 900 nominal source frames (29.3%). Across 30 seconds, 1,240 notifications
produced 1,239 video draws. Actual render starts averaged 6.43 ms late (maximum
17.13 ms late, no early starts). Total state-save work was 31.1 ms, versus
3,029.5 ms inside mpv rendering; these totals reinforce that the GL guard is not
the dominant pacing problem.

The mpv source renders the GPU work **before** its optional wait for target
presentation time. Waiting until the target to begin all GL work therefore
adds predictable latency. `SEREIN_VIDEO_PREPARE_MS=8` is a subsequent opt-in
diagnostic allowance to start up to 8 ms before the true target; allowed values
are 0–16 ms and no greater than `SEREIN_VIDEO_LEAD_MS`. The true target remains
the reference for early/late counters. Both defaults were zero in that experiment, and this
allowance does not establish precise presentation or A/V synchronization.
A more exact adapter can prepare a bounded set of persistent GPU targets early
and publish completed borrowed images according to deadlines, provided it proves
target ownership, bounded buffering and actual surface presentation timing.

Local media/subtitle command methods now perform no filesystem I/O on the UI
thread. They require absolute, explicitly selected paths validated by the host
before window creation or on a worker. Nonexistent/unreadable media then also
receives asynchronous engine errors. The media boundary rejects relative paths;
provider URLs cannot enter this explicit local-path operation.

## Latest native lifecycle observations

Commit `3846416` passed the expanded 20-second fixture harness, including an
assertion that the paused seek reached 20.0 seconds before resuming. Native
`Occluded(true)` / `Occluded(false)` events were observed during minimize/restore,
with the engine changing Paused → Playing under the declared pause-when-hidden
policy. Fullscreen, resize, subtitle cycling and control hiding were exercised.
Catalog row changes stayed zero and the reset count stayed at one (explicit
fixture initialization). These observations do not replace perceptual subtitle,
A/V sync, or screen-reader checks.

A three-second copy-trimmed fixture reached `Ended`, paused, at 2.967 seconds of
a 3.008-second file. Draw callbacks stayed at 216 between the 5-second checkpoint
and the 8-second exit, and event counters also stayed unchanged: EOF did not
leave a self-sustaining redraw/progress loop in this run. Clean teardown occurred
in both cases. Selected raw logs are retained under docs/evidence.

## Rejected render-ahead experiment and next timing measurement

The bounded render-ahead pool was implemented, tested and removed: it did not
reduce warm frame loss. Its [preserved experiment](experiments/render-ahead.md)
explains mpv's per-frame VO wait and why a 50 ms admission offset did not create
a 50 ms preparation pipeline. The active implementation retains its persistent
pair and then-zero-lead default. The later macOS clock candidate is described above. No experimental ring runs by default or is compiled
from the evidence patch.

`SEREIN_MEDIA_TIMING=1` additionally records the newest engine render callback's
monotonic timestamp and its arrival at BeforeRendering. Coalesced callbacks make
this a lower bound on the age of pending work. The callback performs only an
optional clock read, atomic stores, and the existing coalesced wake; it never
renders or accesses mpv. Clock reads are absent when diagnostics are off. These
measurements distinguish UI handoff delay from frame-deadline lateness without
claiming actual swap/scanout timestamps.

An observed `hwdec-current=no` is labeled as software decoding in the shared UI.
The reviewed `d3d11va`, `vulkan`, `dxva2`, `nvdec`, `vaapi`, `vdpau`, `drm`,
`mediacodec`, and `videotoolbox` **`-copy` variants** receive a persistent
CPU-accessible-frame fallback warning. Unknown/empty methods on active video
receive an unverified-decoder warning; audio-only/inactive loads do not.
The silent-audio diagnostic warning retains priority. Direct `videotoolbox`
does not receive a fallback warning, but absence of a warning is no performance
or end-to-end zero-copy certification.

This classification follows mpv 0.41's
[decoder implementation](https://github.com/mpv-player/mpv/blob/v0.41.0/video/decode/vd_lavc.c):
`hwdec_autoprobe_info` includes these copy methods in automatic selection;
`add_hwdec_item` appends `-copy` for methods marked `copying`. Such methods output
software frames or download hardware frames through `mp_image_hw_download`.
The [property implementation](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c)
returns the active method from `VDCTRL_GET_HWDEC`, returns `no` for software, and
reports unavailable before an observation exists. The label does not classify
every downstream import, conversion, copy, or allocation. Unreviewed names are
unknown, not classified solely by a suffix. Focused classifier tests were
authored; actual forced-fallback native validation remains pending after the
exclusive soak window.

Rendering failures stop the current load and leave a useful error, so a bounded
queue/target failure cannot become a repeated failing redraw loop.

### Initially sleeping display: native recovery observation

A controlled debug run launched `--local artifacts/local-1080p60.mp4 --diagnostics
--quit-after 22` while `CGDisplayIsAsleep(CGMainDisplayID())` returned true.
Presentation setup reported CoreVideo error -6661; the five-second checkpoint
remained Idle, with no video targets or polling loop. At six seconds an explicit
five-second `caffeinate -u -t 5` diagnostic woke the display (the same CoreGraphics
query then returned false). Winit delivered `Occluded(false)` at 6,318 ms. The
app retried presenter creation in BeforeRendering, with the owning OpenGL context
current, and consumed its pending startup media exactly once.

The ten-second checkpoint observed Playing, VideoToolbox H.264 1920×1080@60,
AAC/AVFoundation, position 3.2 s and an active playback display-sleep assertion.
The process exited 0 after 22 seconds. Two persistent targets (8,614,016 bytes)
were allocated; catalog notifications/resets remained zero. The local log is
`artifacts/display-recovery-controlled.log`. This validates this initial sleep/
wake case, not arbitrary GPU device loss, another monitor/GPU, perceptual A/V sync
or a performance budget. No global power preferences were changed.


## File-scoped audio, start position and initial subtitles

The former `pending_audio`/`pending_subtitle` slots were consumed by any queued
FILE_LOADED event. Rapid replacement could therefore attach the newest track to
an older file. They have been removed. A bounded `mpv_command_node_async` command
now supplies the loadfile fourth argument as a NODE_MAP with string values:
`start`, optional `audio-files-append`, and optional `sub-files-append`. The
required third insertion-index argument is `-1`. mpv 0.41 command.c:cmd_loadfile
copies these parameters to the exact new playlist entry; they are restored after
that entry ends. m_option.c:separate_input_param uses no list separator for the
append operation, so the complete URL/path remains one value. No global cookies
or HTTP headers are added by this change.

The fixed argument/option count and 64 KiB per-string bound limit command storage.
CStrings, pointer vectors, node values and node lists remain owned until the
async submission returns; client.h explicitly documents that writing NODE data
is not modified and is copied when needed. No borrowed pointer survives that
call. Two command slots are reserved for load plus the existing following pause
command, within the 64-pending-command limit. The UI-thread caller performs no
filesystem or network IO here.

`load_local_at` and `load_https_at` retain their public signatures. Initial local
subtitles use `load_local_with_subtitle_at` in the same submission; the obsolete
queue-based add_subtitle entry point is removed. Caption cycling remains. The
shared application requires an explicit local file for the CLI subtitle option
until a real remote-caption selection path is implemented.

A native headless regression test (null video and audio outputs) loads silent WAV
and SRT fixtures with commas, equals signs and UTF-8 in their filenames, verifies
three tracks, rapidly replaces two entries and verifies the final unrelated file
has only its own single track. It passes against installed libmpv 0.41. Seven
media tests and strict Clippy pass. This demonstrates option escaping, ownership
and restoration. The latest 20-second native local lifecycle also passed initial
subtitles, paused seek, shared fullscreen controls, hide/restore and teardown,
with zero unrelated catalog changes and one fixture initialization reset. Public
quality-switch requalification follows separately. Raw signed URLs are never logged.

## TLS and compressed-stream callback boundary

The direct media path now sets tls-verify=yes explicitly: mpv0.41 stream_lavf.c
defaults this option to false. `new_with_ca_file` accepts a host-prevalidated
absolute CA resource without performing UI-thread file I/O; bundle CA selection
and validation happen before window creation. A native property test confirms
verification is enabled. This does not claim complete media egress confinement.

The experimental `load_streams_at` API receives policy-owned stream factories
with no URLs/headers in media, as detailed in ADR002. A real native headless test
demuxes WAV bytes through the custom callback protocol and confirms every cookie
closes before teardown returns. Live HTTP playback/performance remain separate.

Post-load caption attachment accepts a prevalidated local path plus an Arc lease.
The host checks session/playback generation and keeps its own file owner for the
whole playback lifetime; media retains an extra lease until asynchronous sub-add
reply or engine destruction. Lease Drop queues cleanup, never performs UI-thread
I/O. Cached flag selection reuses an existing track for the same path. Four adds
may be pending, metadata lengths are bounded, and replaced-playback failures are
suppressed using media generation. Off is re-applied after a pending add completes
to preserve the user's latest intent without polling. Observed sid is normalized
to a positive track ID; no caption text/path enters Snapshot diagnostics.

Fourteen media tests cover these contracts, including TLS, callback cancellation,
stream ownership, and caption lease/stale-reply behavior. The explicit loopback
TLS fixture is ignored by default and passes separately via its harness. Caption
confirmation now also matches the engine's external subtitle filename privately
against at most eight registered local files; no raw path enters Snapshot. A
native null-output regression changes two captions, races Off against an add and
reselects cached files, with one FILE_LOADED throughout. This proves exact track
selection/ownership without recreating playback, not visible composition or A/V
perception. Actual FILE_LOADED and subtitle-update counters support UI lifetime
and stale-selection handling.


## Local VTT glyph composition check (2026-09-29)

An 18-second native debug run with the generated local 1080p60 H.264/AAC clip
and an explicitly selected synthetic VTT cue exited successfully. Binary SHA-256:
`739d448427073c7b39d128a5a7bb31a6b36e8e2b7d85699becf1d19c9a84d700`.
The [actual captured window](evidence/2026-09-29-local-subtitle-glyphs.png)
shows two readable subtitle lines inside the video, including `Café`, Czech
accents, an em dash and digits. The selected subtitle ID was 1; the
[log](evidence/2026-09-29-local-subtitle-glyphs.log) observed VideoToolbox,
AVFoundation audio, zero unrelated catalog changes/resets and two persistent
render targets. The [VTT fixture](evidence/2026-09-29-local-subtitle-glyphs.vtt)
is labeled synthetic and is never normal catalog content.

The one-shot diagnostic screenshot at 15 seconds deliberately reads pixels back;
it is not the playback transfer path or performance instrumentation. This debug
run recorded 64 VO drops, so it is not a smoothness/resource pass. Visual review
supports legibility of these Latin glyphs and upright composition on this host;
it does not qualify color accuracy, all writing systems, spoken A/V sync,
automatic YouTube captions or the other platforms.

```sh
target/debug/serein --local /ABSOLUTE/local-1080p60.mp4 --subtitle /ABSOLUTE/fixture.vtt --ui-size 1100x760 --ui-theme dark --snapshot /ABSOLUTE/glyphs.png --diagnostics --quit-after 18 --data-root /ABSOLUTE/ISOLATED/ROOT
```

## Cross-file display admission and terminal stop (2026-09-29)

A matching `load_request_id` and native playlist-entry ID proves which file is
active, not which pixels libmpv would return. In mpv 0.41,
[FILE_LOADED precedes the play loop](https://github.com/mpv-player/mpv/blob/v0.41.0/player/loadfile.c#L1869).
[Video-chain teardown retains the VO](https://github.com/mpv-player/mpv/blob/v0.41.0/player/video.c#L155),
and [VOCTRL_RESET retains its current frame](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/vo_libmpv.c#L606).
A render without a next frame can therefore
[redraw the previous current frame](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/vo_libmpv.c#L386),
including during resize. FrameInfo has no playback-entry identifier.

The presenter now invalidates its pending FrameInfo/deadline when an accepted
load ID changes. It continues consuming startup frames into its private GPU
targets but returns no Slint image until the exact active entry has emitted
PLAYBACK_RESTART and a request-correlated asynchronous `video-dec-params/w`
query confirms decoded video. Unlike `width`, this property has no fallback to
container dimensions and is unavailable without a video decoder/VO chain.
Late replies from another load or entry cannot admit publication. The first
accepted publication forces an actual render even for a paused file with no new
pending frame; it never simply returns an old target. This uses one native query
and a coalesced one-shot redraw, not a polling timer. The media layer additionally
pins `video-latency-hacks=no`: the ordinary startup path queues the first video
frame and [waits for the VO before restart](https://github.com/mpv-player/mpv/blob/v0.41.0/player/video.c#L1275).

**This ordinary presentation gate is not an unconditional privacy proof.**
PLAYBACK_RESTART also accepts video EOF, and decoded parameters alone do not prove
that a frame reached the VO: end/frame limits can discard a decoded image.
Account-boundary transitions must clear the shared UI image immediately and wait
for the stronger terminal-stop boundary below before accepting the next load.
They must not substitute FILE_LOADED, a generic stop reply, or the ordinary
`current_load_frame_ready()` predicate for that boundary.

`Player::stop()` invalidates registered streams immediately and has one reserved
asynchronous command slot independent of the ordinary 64-command limit.
Repeated requests coalesce. New loads are refused while `Snapshot.stop_pending`
is true. The client API permits asynchronous reordering, so when older loads are
outstanding, an immediate stop is followed by one final stop only after every
older load and the immediate stop report completion. This is grounded in
[cmd_loadfile inserting its playlist entry before returning](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c#L6116)
and [cmd_stop clearing the playlist](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c#L6280).

After those replies, a fresh asynchronous `current-vo` query must return
`MPV_ERROR_PROPERTY_UNAVAILABLE` before stop admission reopens. `force-window=no`
is pinned. The native [idle loop calls handle_force_window before notifying idle](https://github.com/mpv-player/mpv/blob/v0.41.0/player/playloop.c#L1336);
[that call destroys an unused VO](https://github.com/mpv-player/mpv/blob/v0.41.0/player/playloop.c#L1021).
The [current-vo property reports that actual VO pointer](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c#L3002),
and [libmpv VO uninitialization clears its current frame](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/vo_libmpv.c#L685).
If a query still sees a VO, native idle notifications trigger another fresh
query; only one query is outstanding, and a notification racing that query is
remembered. There is no retry loop or timer. Unexpected native errors keep
admission closed and are reported; explicit stop retry is possible. The pinned
0.41 deprecated IDLE event is an explicitly enabled wake hint, never the proof
itself. Upstream updates must re-audit that dependency. Caption file leases still
release at their exact END_FILE event after native demuxer teardown.

Position and duration reset on accepted load, stop and START_FILE. This removes
stale timing metadata from the immediate snapshot; host rendering also needs its
load-identity admission check. Account transitions additionally invalidate their
asynchronous UI operations and private metadata.

The focused media suite passes **25 tests**, with the separate synthetic TLS
fixture ignored by default. Native null-output regressions cover a saturated
64-command queue, 1,000 coalesced stop requests, queued replacements, the final
empty playlist, paused synthetic video admission, an old property reply, failed
replacement and audio-only rejection. Strict all-target Clippy passes. These
headless engine tests do not prove Slint/GL cross-file presentation: the separate
30-second local video→audio→paused-video→stop→paused-video window diagnostic
remains pending at this source checkpoint. No authenticated human test or privacy
certification is claimed.

### Follow-up: private target ownership and unavailable metadata

The first 30-second GL handoff retry passed on release SHA-256
`aedd865c006c234798402d1de7569d034591e61db40915dc08384153f7f63322`
with a bounded external display-awake assertion. It exercised four loads,
audio-only blanking through resize, paused-video publication, terminal stop and
clock shutdown, with zero unrelated catalog notifications. An earlier attempt
failed after a logged native occlusion while paused; it is retained as a failed
functional attempt. Neither run is a resource qualification.

Subsequent review found a texture-ownership defect in that binary: private
startup rendering swapped `displayed` and `next` despite returning no image.
Slint could still borrow the previous displayed target, so a later private render
or resize could overwrite or retire it. The corrected presenter keeps private
work exclusively in `next`; it creates a borrowed image and swaps slots only
when publication is allowed. The host passes an explicit `allow_publication`
flag and must assign every returned image immediately and unconditionally.
This also prevents the host from silently discarding a returned replacement
while retaining a borrow of the now-reusable target. Target-byte accounting
includes private targets. A focused ownership regression exercises repeated
private writes, private resize/retirement and subsequent publication while an
old host borrow remains live; it makes no GL driver claim.

The same handoff exposed stale diagnostic metadata: unavailable property events
previously left the prior H264/VideoToolbox dimensions in an audio-only snapshot.
`MPV_FORMAT_NONE`, null property data and null string values now clear only the
corresponding duration/codec/decoder/dimension/fps/audio field. No decoder or
playback option changed. A synthetic property matrix verifies localized clearing,
and a native null-output paused Y4M→WAV transition verifies that video metadata
disappears while valid audio metadata remains. An inactive hardware-decoder
property may legitimately be `no`; the host must require active video before
labeling that as software-video fallback.

After these two fixes the media suite passes **27 tests**, with the explicit TLS
fixture ignored. The previous native GL pass predates the ownership and metadata
fixes: the rebuilt GL handoff, quality replacement and caption regression runs
are still required before claiming those fixes validated in the real presenter.

### Finite audio diagnostics after replacement

A subsequent scoped-stream refresh run observed a cached decoder sample rate of
zero after replacement despite selected Opus metadata and an AVFoundation output
name. Zero here means unavailable/unknown, not measured zero-Hz audio. The pinned
[client API warns that property observations can miss updates](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/client.h#L1174),
and the installed 0.41 manual distinguishes decoder `audio-params` from output
`audio-out-params`. Its default weak gapless mode may retain an output object
across files. Neither the codec label nor the output name proves audio delivery.

`request_audio_probe()` now offers an explicit finite diagnostic: three async
GET_PROPERTY operations read `audio-params/samplerate`,
`audio-out-params/samplerate` and `audio-pts`. One probe may be outstanding; it has
no timer, automatic retry or effect on ordinary playback. The combined reply
carries a nonsecret token and load request ID and is published only after all
three replies match the captured playback generation and native playlist entry.
Unavailable/nonpositive rates and nonfinite timestamps remain `None`; finite
negative audio timestamps are valid driver-delay readings. Cached observer
fields are deliberately not overwritten, allowing the diagnostic to distinguish
stale observation from current native unavailability.

Load, seek, pause, stop and native entry changes invalidate probe results.
Token-scoped cancellation and partial submission failure retain admission for
already-submitted native replies until they drain; repeated cancellation cannot
create an unbounded native queue. No provider URL, header, credential or audio
sample enters these structures.

The media suite passes **35 tests**, with the explicit TLS fixture ignored.
Five accounting tests cover out-of-order/duplicate replies, stale contexts,
partial submission failure, cancellation lifetime and invalid values/token
bounds. A real null-output paused-WAV test obtains decoder and output sample rates
of 48,000 Hz and a finite audio timestamp, and verifies replacement invalidation.
Strict all-target Clippy passes. The scheduled two-probe live refresh diagnostic
still needs to run on the rebuilt executable. Advancing native audio timestamps
would strengthen engine-level audio evidence; it would not qualify audible
output, perceptual A/V synchronization or resource usage.


### Rebuilt functional regressions at `785ed516`

Release SHA-256
`785ed516f5d703031b1756161ea7a9676197ca963665fc2a6ffe2db3a090222d`
passed the rebuilt 30-second handoff, 70-second scoped guest refresh and
85-second guest caption/clear-local diagnostics on macOS/Apple M1. This binary
came from the evolving working tree; no exact source commit or frozen build
manifest is asserted. [Run metadata, log hashes, source limits and retained
failures](evidence/2026-09-29-account-integration-summary.json) preserve that
qualification boundary. All three used fresh isolated profiles and a bounded
external display-awake assertion, and exited normally without harness timeout.
No real account, credentials or resource measurement was involved.

The [handoff log](evidence/2026-09-29-account-integration-handoff.log) records
four loads: local H.264/VideoToolbox video, audio-only WAV blanking through resize,
paused-video publication, terminal stop blanking, and another paused-video
publication. Texture width was zero at the audio-only/stop checks and 1,672 at
both accepted paused-video checks. The audio-only checkpoint had empty video
codec/decoder and zero video dimensions. Catalog changes/resets remained zero.
These observations validate the finite presenter lifecycle after the private
GPU target fix; they do not prove every codec, adversarial handoff or account
privacy scenario. The earlier occluded failure and pre-fix pass remain retained.

The [scoped refresh log](evidence/2026-09-29-account-integration-refresh.log)
records two loads, a fresh 49.933-second resume position and no second automatic
refresh. VideoToolbox H.264 1080p60 and Opus/AVFoundation were observed. The old
source recorded **23 unclassified read errors**; these cannot be relabeled as
expected cancellation. The replacement's cached audio sample-rate field was zero,
meaning unavailable/unknown in this observation, not proven silent output or a
zero-rate decoder. Error classification and a finite fresh-property audio probe
were added afterward; the separate repeat is recorded below. See [stream-refresh.md](stream-refresh.md).

The [85-second clear log](evidence/2026-09-29-account-integration-clear.log)
used the caption-capable public fixture `wsQiKKfKxug` on the ordinary guest media
path. It exercised selection, Off, cached reselection, paused quality replacement
and coordinated clear. At 70 seconds the player was idle with stop admission
complete; by 80 seconds the clear barrier had finished. There were two loads,
zero catalog row changes and one explicit clear-induced reset. Read-only inspection
of the isolated profile afterward found zero playlist, playlist-item,
subscription and history rows; one preferences row with all optional privacy
features off; SQLite quick-check `ok`; and no VTT or other caption payload files.
Only the app's caption registry lock remained. Profile paths and row contents
were not exported. The earlier wrong-fixture assertion is retained as a harness
choice failure, not evidence that the supported caption path failed.


### Fresh-property/audio and classified-network repeat

The subsequent source checkpoint `287a7c0` passed 222 Rust tests and strict
workspace Clippy before its locked release was run. Release
`9a48e90f8a7f75adba352ae11d3a2f3df3f0f28d98cf3a71f184a2b1b776a799`
then passed the 70-second scoped guest refresh diagnostic. Two finite,
load-correlated audio queries returned 48,000 Hz for both decoder and output,
with audio PTS advancing from 79.037692 to 84.038267. This distinguishes unknown
cached notification state from the actual current audio properties. It does not
substitute for listening or A/V synchronization testing. All 28 recorded HTTP
ranges completed with zero categorized errors; the previous 23 unclassified
errors remain historical and unexplained, rather than relabeled as cancellation.

All 108 hashes in the post-build source manifest match the subsequently committed
checkpoint. See [source manifest](evidence/2026-09-29-account-integration-source-9a48e9.json),
[log](evidence/2026-09-29-account-integration-refresh-9a48e9.log), and
[stream-refresh.md](stream-refresh.md#classified-transport-and-fresh-audio-repeat-at-287a7c0)
for exact scope. This new repeat does not replace the earlier clear/handoff
binary evidence or qualify native authenticated playback/performance.

### Corrected keyboard, mute and guest replacement regression

Source `ca21de92a94abf209b76576c8b584751d3d20b4f` passed the local native lifecycle
in debug and release; the release took 21.549 seconds. The corrected fullscreen
Escape test focuses by clicking the visible video, preserving normal-window
scroll for the later restoration assertion. Editor Escape invoked zero playback
fullscreen callbacks. Native mute/unmute retained volume 100 and the same single
file load; paused seeking, resize, minimize/restore and finite moving-pixel
readback checks also passed. These are state/composition checks, not a listening
or performance result. See [keyboard-controls.md](keyboard-controls.md).

The same release passed public guest refresh (71.606 seconds) and captions
(71.730 seconds). Refresh performed one real replacement at fresh position
51.650 seconds, retained two loads and returned 48,000 Hz decoder/output rates
with advancing finite audio PTS. Captions passed exact selection, Off, cached
reselection and paused quality reattachment with one cache file and two loads.
VideoToolbox was observed in both runs; no actual account was used. The [complete
evidence summary](evidence/2026-09-29-raster-corrected-summary.json) associates all
six debug/release checks with 125 frozen source inputs and exact binary hashes.
Earlier first-run keyboard/scroll failures remain retained. No new screenshot,
perceptual sync, resource, decoder fallback or cross-platform pass is inferred.

The subsequent PiP restoration fix selects the original still-connected monitor
from current native enumeration before applying saved geometry. It retains the
same player, window and presentation context. Monitor disconnection and movement
across displays still need hardware qualification; pure geometry/selection tests
are not that evidence. See [PiP restoration](picture-in-picture.md#restore-the-original-display).
