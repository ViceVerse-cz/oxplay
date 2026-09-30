# Optional GPU timeline diagnostics

Set `OXPLAY_GPU_TIMING=1` for an explicit diagnostic run. With the variable absent,
the presenter creates no query objects and performs no diagnostic GL queries.
This instrumentation is not enabled in resource acceptance runs.
`OXPLAY_GPU_TIMING=ui-elapsed` separately selects the UI-only elapsed-query mode
described below. It does not silently change the meaning of the existing timestamp
mode or populate its media/frame counters.

The implementation uses the pinned glow0.18 native query APIs and desktop GL3.3
TIMESTAMP queries. It checks the implementation's counter width through
GetQueryiv; zero useful bits is unsupported. Results are read only after the
last timestamp in a sample reports QUERY_RESULT_AVAILABLE. The ordering and
availability rules come from [ARB_timer_query revision13](https://registry.khronos.org/OpenGL/extensions/ARB/ARB_timer_query.txt).
If advertised, [EXT_disjoint_timer_query](https://registry.khronos.org/OpenGL/extensions/EXT/EXT_disjoint_timer_query.txt)
is checked before and after collection; a discontinuity discards affected samples.
Desktop core timers provide no universal disjoint indicator. The diagnostic
reports that limitation rather than assuming an unchanged GPU clock.

In timestamp mode, sixteen slots contain four query objects each, allocated once in the owning
Slint context. New samples are skipped when all slots are in flight. Collection
occurs only on existing BeforeRendering callbacks; it creates no polling timer,
extra redraw or idle wake. Drop deletes queries in the same current context.
There is no glFinish, synchronous-result wait, pixel readback or full-frame copy.
Intervals of500ms or more, invalid ordering and unsupported counter widths are
rejected and counted, not silently included in means. This bounds rollover
interpretation; it also means long stalls are excluded from duration aggregates.

The timestamps delimit:

- `frame_ns`: BeforeRendering entry to AfterRendering entry. Slint's initial
  clear/flush before the notifier is excluded, as are actual swap, compositor and
  scanout. This is not an end-to-end whole-window presentation measurement.
- `media_ns`: immediately around libmpv's OpenGL render call, on media-draw frames.
- `ui_after_media_ns`: media-end to AfterRendering; on UI-only frames it uses the
  complete measured notifier interval. Divide by frame_samples, not media_samples.

These are GPU-timeline elapsed intervals. CPU command-submission gaps and idle
periods can be included; they are not GPU utilization, energy or isolated shader
execution time. Native scanout feedback and perceptual A/V sync remain separate.
RenderStats.gpu also reports sample counts, maxima, invalid/dropped/pending samples,
counter width and optional disjoint support. Native availability and useful timing
on each GPU must be demonstrated before reporting measurements; unit tests only
validate arithmetic and bounded interval handling.

## Current host observation

On 2026-09-29 the Apple M1 OpenGL4.1/Metal91.7 diagnostic reported
`counter_bits=0` in a twelve-second functional run. The diagnostic
therefore reported requested=true, supported=false, zero samples and no query
objects. Local H2641920×1080@60 VideoToolbox/AAC playback continued, with 599 video
draws and clean teardown. Zero timestamp bits is an allowed unsupported result;
this run provides no GPU duration measurements.
That historical diagnostic used the same zero value if `glGetQueryiv` could not
be resolved; its log cannot distinguish that case from an actual zero-bit query.
The new diagnostic records `query_api_available`, independent
`elapsed_counter_bits`, and allocation failure explicitly. The native repeat below distinguishes the capabilities;
it preceded the optional elapsed sampling implementation.

Debug binary SHA256 was
`5659929cfc053fee645a93cdc8d426640008a8d078976f80e648f0407d41f24a`.
The first attempt encountered the separately known sleeping-display native-clock
error -6661. A repeat used a diagnostic-only `caffeinate -u -t5` wake and successfully
created the presentation clock; timestamp bits remained zero. Logs are
`artifacts/gpu-timing-functional.log` and `artifacts/gpu-timing-awake.log`.
The debug build and concurrent workspace compilation make this a functional
capability check only. An independently validated TIME_ELAPSED query or native
GPU profiler would be required before attributing GPU costs on this host.

## Elapsed-query fallback review

The alternate `GL_TIME_ELAPSED` capability must be queried independently on the
actual context; TIMESTAMP bits zero neither proves nor disproves it. The latest native probe reports 32 elapsed counter bits, as recorded below. An outer elapsed query
around `mpv_render_context_render` is unsafe: mpv0.41
[ra_gl.c](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/ra_gl.c)
already starts elapsed queries for internal passes, and its nesting guard only
knows about its own queries. A host query would conflict with that state.

The optional diagnostic below measures only the post-mpv Slint interval,
after confirming no existing elapsed query and checking the counter width.
Media pass costs could separately come from mpv's existing `vo-passes` property,
whose implementation reads the renderer's recorded pass samples. These are
different measurement boundaries and could not be added into an asserted
end-to-end GPU duration. However, the current libmpv configuration has advanced
control disabled; mpv0.41 `vo_libmpv.c` only dispatches performance-data control
to the render thread when advanced control is enabled. `vo-passes` is therefore
not immediately available through the current path. Enabling it would require
separate render-dispatch, cancellation and shutdown qualification, not a silent
profiling toggle. The existing mpv timer pool reads a reused query after
eight slots without an availability test; a new diagnostic must not copy that
potentially blocking collection pattern. The implementation below leaves the
renderer and libmpv control mode unchanged.


A subsequent 85-second release clear-local-data diagnostic on the same M1/OpenGL
context reported `query_api_available=true`, `counter_bits=0`,
`elapsed_counter_bits=32`, `allocation_failed=false`. Binary SHA-256:
`6791c634802d1649b5d4ff26a7c5cfd6d3e1c334527a527760fe26233edcf874`.
Thus the timestamp capability query was actually invoked and returned zero;
the missing-function ambiguity is resolved for this run. The independent
TIME_ELAPSED capability query returned 32. This is capability evidence only:
no elapsed query sampling was implemented, and all duration/sample fields remain
zero. [Native evidence](evidence/2026-09-29-clear-local-native.log).

## Explicit UI-only elapsed mode

`OXPLAY_GPU_TIMING=ui-elapsed` allocates sixteen TIME_ELAPSED query objects in
the owning context, only when GetQueryiv reports 30–64 useful elapsed bits.
The same context/thread requirements and nonblocking availability checks apply.
No native execution of this new mode has been recorded yet.

The start boundary is the end of `GlPresenter::render`, after the complete media
render operation, GL-state restoration and image bookkeeping. A common wrapper
places that boundary on UI-only early returns and render errors too. The end
boundary is entry to `GlPresenter::after_render` in Slint's AfterRendering
callback. The pinned
[Slint renderer](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs)
submits the UI command buffer and drains its image caches before that callback;
actual presentation happens afterward. The pinned FemtoVG OpenGL backend contains
no timer-query calls. Any future renderer change requires this contract to be
reviewed again.

This interval includes Slint's drawing of the displayed video texture together
with the rest of its UI. It excludes libmpv's video conversion/render passes,
Slint's pre-notifier clear, and swap/compositor/scanout. It is neither media GPU
duration nor a whole-frame duration; it cannot be added to a CPU render-call
duration to derive either. GPU timeline elapsed time can also include command
submission gaps and scheduling delays, so it is not isolated shader execution or
GPU utilization.

`CURRENT_QUERY` is checked before starting and ending. A foreign elapsed query
is never nested, ended or deleted. An ownership conflict disables further
sampling and increments `elapsed_conflicts`; it does not interrupt playback.
If an AfterRendering callback is missing, the next BeforeRendering first ends
only a still-owned query, invalidates it and disables sampling before entering
libmpv. During teardown the diagnostic is dropped before freeing the libmpv
render context, while the GL context remains current. An unexpectedly active
foreign reuse of an owned query name is left for context destruction rather than
deleting that active query.

Only `ui_elapsed_samples`, `ui_elapsed_ns` and `max_ui_elapsed_ns` accumulate in
this mode. `ui_elapsed_requested` identifies the mode; `supported` describes its
selected capability. All timestamp `frame_*`, `media_*` and
`ui_after_media_*` duration/sample fields stay zero. Shared pending, dropped,
invalid and disjoint counters retain their documented meanings. Results of
500 ms or more are rejected. A narrow timer can overflow during extreme GPU
stalls; core GL has no universal disjoint or overflow indicator, so this filter
does not establish that every smaller result is free of clock discontinuities.

The focused mocked-GL tests pass for explicit selection, no elapsed query around
media work, sixteen-slot exhaustion without reading unavailable results,
separate UI totals, missing AfterRendering, teardown, and foreign-query
collisions. These validate application ownership and accounting logic, not
driver behavior or measurement accuracy. A native opt-in functional run is the
next required step before collecting or publishing UI GPU durations.

## Native UI elapsed-query result

Release binary `ac87f2c3ad53ff066fcb5596ec9ffc614c5f0e81c737700ed25f1a7f8628ea80`
passed the20-second local lifecycle with `OXPLAY_GPU_TIMING=ui-elapsed`,
`--search-cache`, light theme and the labeled related-row fixture. It exercised
pause, paused seek, resume, resize/fullscreen, text-input shortcut ownership,
hidden controls, minimize/restore and cleanup. Catalog changes stayed zero;
the only reset was fixture initialization.

On Apple M1/OpenGL4.1 Metal91.7, the elapsed query reported32bits and supported=true.
There were1,523 accepted UI intervals totaling2,120,306,919ns (mean1.392ms,
maximum6.755ms), zero conflicts/invalid samples/dropped diagnostic samples, and
one pending result at the final snapshot. That pending query was not synchronously
waited on. Timestamp frame/media sample counts and durations correctly stayed zero.
The process exited normally. [Native log](evidence/2026-09-29-ui-elapsed-lifecycle.log)
and [source/binary provenance](evidence/2026-09-29-ui-elapsed-provenance.json).

These mixed lifecycle UI intervals establish that the optional instrumentation
works on this host. They are not steady playback timings, GPU utilization,
media-render duration, scanout, energy or resource acceptance. Resize/fullscreen
transitions reallocated targets and88VOdrops were recorded; no throughput claim
is made from this diagnostic. The noninstrumented18-second control screenshot
run logged requested=false and zero query samples.
