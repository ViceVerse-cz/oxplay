# macOS native OpenGL swap timing differential

The SDL-only diagnostic in `tools/media-baseline/` established a functioning
same-engine, same-codec hardware-decoded baseline after the packaged mpv Vulkan
window baseline failed. It is not a shipped frontend. This investigation used
Homebrew mpv 0.41.0_10, libmpv API 2.5.0, sdl2-compat 2.32.72 over SDL3 3.4.16,
and the existing licensed local H264/AAC 1080p60 fixture. The observed decoder
was VideoToolbox and audio output was AVFoundation in every run. The display
reported 1440×900 at 60 Hz. Power diagnostics reported low-power mode off.

The first two 30-second runs used a 928×280 physical drawable and therefore are
functional comparisons, not resource baselines comparable to the application.
Both libmpv blocking render/default 50 ms lead and nonblocking render/zero lead
had zero additional VO drops between 10- and 25-second checkpoints. Their full
local logs are `artifacts/sdl-baseline-block.log` and
`artifacts/sdl-baseline-nonblock.log`.

The matched run requested a Retina drawable (1856×560 physical pixels) and
nonblocking render, zero timing lead, and advanced-control 0, matching the
application's engine settings. It had 62 startup drops and no further drops
between 10 and 25 seconds; there were 904 draws over 15.068 seconds. Its log is
`artifacts/sdl-baseline-matched.log`. It omits Slint composition, so it cannot
qualify total application rendering cost.

Source inspection identified a concrete difference:

- SDL3 release 3.4.16 (resolved SHA
  `fa2c02bb6e21974a89ea9824bc53c9932abe5f9c`) explicitly sets native
  `NSOpenGLCPSwapInterval=0` and handles swap timing separately with a
  CVDisplayLink condition. See
  [`SDL_cocoaopengl.m`](https://github.com/libsdl-org/SDL/blob/fa2c02bb6e21974a89ea9824bc53c9932abe5f9c/src/video/cocoa/SDL_cocoaopengl.m#L380-L383)
  and its
  [`Cocoa_GL_SwapWindow`](https://github.com/libsdl-org/SDL/blob/fa2c02bb6e21974a89ea9824bc53c9932abe5f9c/src/video/cocoa/SDL_cocoaopengl.m#L489-L521).
- Slint's pinned Winit/FemtoVG context requests native swap interval 1 through
  glutin 0.32.3. That backend sets `NSOpenGLCPSwapInterval` directly and calls
  `flushBuffer()` without SDL's separate CVDisplayLink wait.

The next diagnostic disabled SDL's DisplayLink wait and set the native CGL
swap interval to 1, verifying the getter returned 1. All other matched engine
and drawable settings stayed the same. Severe dropping reproduced: 365 drops
at 10 seconds, 785 at 25 seconds, 928 at 30 seconds. The warm difference was
420/900 expected frames (46.7%), with decoder drops 0. This isolates native
swap blocking as a causal difference in this environment; it does not establish
which macOS driver/GL layer implementation is responsible. Log:
`artifacts/sdl-baseline-native-swap.log`.

Disabling both native swap blocking and SDL's extra display-link wait also
failed: 265 drops at 10 seconds, 657 at 25 seconds, a warm 392/900 (43.6%).
Log: `artifacts/sdl-baseline-unblocked.log`. Therefore merely changing native
swap interval to 0 is not a valid fix; the successful comparison specifically
requires display-driven synchronization. No production swap override was
selected from the earlier differential alone. A wait-before-render baseline
can next test whether a nonblocking, vblank-gated UI handoff is sufficient.

A nonblocking, demand-started CVDisplayLink was then tested (mode 4). The callback
only marked a frame ready and posted one coalesced event; the main loop stopped
the display link before rendering. With zero lead, warm drops increased 21→57
(36/900, 4.0%). With the upstream 50 ms lead, drops increased 7→31
(24/900, 2.7%). These are materially better than unpaced native swaps but still
fail the gate. Logs are `artifacts/sdl-baseline-demand-clock.log` and
`artifacts/sdl-baseline-demand-clock-lead.log`. No Rust production adapter has
been selected from this incomplete result. Restarting the display clock for
every frame may introduce phase or startup jitter; that is a hypothesis to test
against continuous display timing during active video, with a bounded stop once
pending work settles.

The continuous display-clock before-render comparison (mode 3) achieved nearly
60 fps: one extra warm VO drop (1→2), with no subsequent increase by 30 seconds.
The nonblocking bounded clock (mode 5) then achieved zero extra warm VO drops:
2 at 10 seconds and 2 at 25 seconds, with 899 additional notifications over 14.973 seconds.
It ended at 1,786 draws/1,788 notifications. The clock started twice and stopped
once during startup, preserving phase throughout steady playback. Its callback
only marks ready or requests stopping after two empty display ticks; it does
not render or perform synchronous UI work. Logs:
`artifacts/sdl-baseline-before-render.log` and
`artifacts/sdl-baseline-bounded-clock.log`.

This result justifies a small macOS presentation-clock adapter, implemented in
`crates/media/src/macos.rs`. It preserves the existing single GL context and
persistent texture pair. It is not proof that the full Slint application now
passes: its frame loss, controls, hidden/paused clock stop, A/V timing, resource
budgets and teardown must be measured using the new application build.

The first full Slint candidate then ran for 30 seconds with the redesigned watch
page, a 1384×778 physical video target and the related-list empty state (normal
local playback, not fixture mode). Binary SHA256:
`a36b84a0f86d0ef133ddf671996b6fc220a427d60a7dfc9be3d2d30dca9f4761`.
The source was a dirty working tree; base commit and media-source hashes are in
`artifacts/display-clock-parent-commit.txt` and
`artifacts/display-clock-source.sha256`. This is identified by binary/source
hashes rather than mislabeled as a clean committed release.

The 10- to 25-second interval had 900 notifications and zero additional VO drops
(3 startup drops, unchanged through exit). VideoToolbox, H264 1080p60 and audible
AVFoundation remained observed; decoder drops were zero. There were 1,777 video
draws, 1,899 UI draws, two target allocations totaling 8,614,016 bytes, and zero
catalog changes/resets. The legacy Slint deadline timer scheduled zero wakes.
The native clock started twice and stopped once during startup, then remained
phase-stable through playback. Clean teardown completed. Log:
`artifacts/slint-display-clock.log`.

This is a pacing correction, not a complete optimized-gate claim. The
instrumented mean render-start deadline lateness was 27.8 ms and mean gated
callback-to-render latency 16.5 ms. Actual A/V presentation latency and scanout
remain separate validation requirements. The next run samples the complete
owned process set for 60 seconds after 10 seconds of warm-up, with optional timing
instrumentation disabled; fixture-list lifecycle and paused/hidden clock stop
are separate follow-ups.


The subsequent 60 one-second resource samples followed 10 seconds of warm-up
with optional timing instrumentation disabled. The full owned-process set plus a
new temporally attributed VideoToolbox service averaged **185.77 MiB RSS and
52.79% of one logical CPU**; CPU p95 was 64.82%. This fails both the 25% target
and 50% release ceiling. There were zero warm drops (one startup drop unchanged
at 10, 25, 70 seconds and exit). Two targets occupied 8,614,016 bytes; unified
GPU/shared memory must not be added to RSS as independent physical allocations.
Evidence: `artifacts/display-clock-playback-60.json` and its `.log`. RSS can
double-count shared pages, and service attribution is temporal rather than
proven exclusive ownership. This fixes frame throughput, not the resource gate.

The next fixture lifecycle initially failed because the display had gone to
sleep during unattended tests. The native setup error was
`macOS create display clock failed (-6661)`; public CoreGraphics queries observed
main display 1 active=0, asleep=1, online=1. A bounded diagnostic
`caffeinate -u -t 5` changed those observations to active=1/asleep=0, and the
same 20-second lifecycle then passed paused-seek assertions, resize/fullscreen,
subtitle controls, minimize/restore and teardown. The hidden stage reported its
display clock stopped. This is an explicitly recorded test condition, not a
change to system power preferences. Logs:
`artifacts/display-clock-lifecycle-retry.log` and
`artifacts/display-clock-lifecycle-awake.log`.

The application now requests `PreventUserIdleDisplaySleep` only while its engine
reports Playing and not paused; it releases the assertion on other states and
teardown. Host occlusion pauses the engine, so hiding releases it too. The SDK's
IOPMLib.h documents that this assertion does not wake an already sleeping
screen, override a closed lid, or prevent explicit sleep. The implementation
uses IOPMAssertionCreateWithName/IOPMAssertionRelease, level 255, scoped to the
media owner; Snapshot exposes `prevents_display_sleep` for lifecycle checks.
A later native run validated the assertion: pmset named `Serein video playback`
and reported PreventUserIdleDisplaySleep=1 during Playing; at the hidden/paused
stage it was 0, and after teardown it remained 0. Snapshot agreed. Paused seek,
resize/fullscreen, subtitles and restoration passed in that run. Initial-sleep
recovery remains a separate check. Binary SHA256:
`e5eb5ccee0659fa8b5b96488bb08c652cb05f57b9ce05d6adba6cd068b88872f`; logs:
`artifacts/display-clock-power-lifecycle.log` and power-playing/hidden/ended text
files. The file named ended records post-process teardown, not an EOF test.


A matched 85-second standalone run with 10-second warm-up and 60 resource samples
used mode 5, nonblocking render, 50 ms lead, advanced-control 0, and a 692×389
logical / 1384×778 physical drawable. It retained three startup drops unchanged
through exit. Mean CPU was 44.59%, p95 53.17%; mean RSS 163.46 MiB. Both it and
the application sample include a newly launched VideoToolbox service with the
same attribution caveat. The independent mean differences (8.20 CPU percentage
points and 22.31 MiB) estimate app overhead; matched video dimensions do not make
the standalone window's composition equivalent to the full Slint UI. Profile
stacks are the next step before selecting an optimization. Reproducible harness
and source hashes plus sanitized sample arrays are retained in `docs/evidence/`.


The same binary's five-second `sample` trace after ten seconds of warm-up found
2,127 main-thread samples, including 202 in UI subtree traversal and 107 in the
media render call. One border/gradient branch accounted for 49 samples allocating
a gradient texture (40 in glTexImage2D). Counts are inclusive stack samples,
not CPU-time percentages or GPU timing. Physical footprint was 275.9M, peak
289.4M in this one capture; it is not substituted for RSS samples. The trace is
`artifacts/display-clock-profile.sample.txt`.

Pinned Slint's FemtoVG renderer flushes its clear before invoking BeforeRendering
(`internal/renderers/femtovg/lib.rs:246–253`); FemtoVG 0.27's flush_to_output calls
release_old_gradients each time, which advances its two-frame gradient cache.
This provides a source-backed explanation for repeated gradient allocations when
a notifier causes two flushes per UI frame, consistent with the observed stack.
It is not an argument to remove the required pre-notifier flush or modify the
locked upstream source. A narrow application-level cache-rendering-hint on the
header and sidebar is the next measured candidate. At 1320×860 logical and scale 2
these two RGBA8 layers would total approximately 3.95 MiB. FemtoVG also
allocates STENCIL_INDEX8 for each layer FBO, adding approximately 0.99 MiB
nominally, so the combined estimate is 4.94 MiB before padding/driver metadata.
Actual child-bounding rectangles may be smaller than the container bounds. Dynamic focus/hover/text/theme changes invalidate the cached subtree;
video and progress remain outside it. Actual allocation and performance impact
need the next sample, and full-window composition still occurs.


The same-binary cache A/B on c77ebe91 did **not** qualify the cached path. Cache-off
mean CPU 47.73% had one warm VO drop; cache-on mean CPU 38.77% had 297 warm drops by 70s,
and 592 by 85s. Its first 10/25-second checkpoints had 2 drops unchanged, so judging
that partial interval alone would be misleading. Native clock starts/stops rose
to 95/94. The full metrics and provenance are in performance.md and evidence/.
A separate lazy-render diagnostic confirmed no newly created layers on reported
steady frames, but also dropped frames. The cache is now opt-in while investigated.

The latest clock source removes the two-empty-tick stop heuristic. A ~33ms gap
in engine callbacks is not an idle state; restarting the native link can lose
phase under jitter. It now preserves phase while the engine reports Playing and
stops immediately otherwise, preserving any paused final-frame readiness. The
callback only tests the engine-pending atomic and never wakes the UI without
work. A regression test exercises 600 empty ticks with zero wakes, then a real
notification with one wake. This is display timing during active playback, not
an engine property poll or permanent redraw loop. Native requalification of
this corrected policy is pending.
