# Render-ahead experiment (not shipped)

The experiment rendered libmpv frames immediately into a bounded pool of five GPU
textures and published them through one-shot deadline timers. A maximum of four
prepared frames and 128 MiB aggregate target allocations applied. The default
presenter was unchanged. The complete media-only change against commit `3846416`
is preserved in [render-ahead.patch](render-ahead.patch); this patch is evidence,
not an enabled production feature.

The 30-second local 1080p60 audible trial used `OXPLAY_RENDER_AHEAD=1`,
`OXPLAY_VIDEO_LEAD_MS=50`, `OXPLAY_VIDEO_PREPARE_MS=0`, and
`OXPLAY_MEDIA_TIMING=1`. `hwdec-current` remained VideoToolbox. Between the
10-second and 25-second checkpoints, VO drops increased from 192 to 469:
277 / 900 expected frames, or 30.8%. There were 1,226 GPU preparations and 1,226
texture publications over the run, with three allocated targets (12,513,600
bytes). Mean CPU-side publication lateness was approximately 7.1 ms, maximum
46.9 ms. These timestamps are image assignment times, not display scanout.
There was no target exhaustion. The optimized playback gate still failed.
The [diagnostic output](../evidence/2026-09-29-render-ahead-rejected.log) is retained.
Release binary SHA256: `a149e6815e116618721b7ce5de7f20ccbe5c80f06b2c4c22c6007bfdaab9d1b1`.
Source was the working tree based on `3846416` with the preserved patch and
subsequent application diagnostic/lifecycle changes. This is an engineering
experiment, not a resource acceptance sample. A separate 20-second ring lifecycle
run completed pause/seek, fullscreen/resize, minimize/restore and teardown without
an observed error; that did not repair the pacing failure.

Why 50 ms did not produce 50 ms of render lead: in upstream mpv v0.41.0,
[`vo_is_ready_for_frame()`](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/vo.c#L835-L865)
subtracts the configured timing offset only when admitting the single next
queued frame. The same file's
[`render_frame()`](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/vo.c#L916-L1020)
draws frame N, then waits until N's target time before calling `flip_page()` and
proceeding to N+1. The
[libmpv adapter](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/vo_libmpv.c#L488-L550)
raises its render callback in `draw_frame()` and consumes the application render
in `flip_page()`. Rendering into more application textures does not remove that
VO-thread timing wait. At 60 fps, the next callback ordinarily has at most one
frame interval of lead; the experiment's maximum observed preparation lead was
15.2 ms despite the configured 50 ms admission offset.

The experiment was removed after measurement: it added ownership and scheduling
complexity without improving the missed-frame rate. Audio-output isolation also
failed to fix pacing, so the cause is not established as solely the host's
AVFoundation fallback. The next useful measurement separates callback-entry to
`BeforeRendering` latency from the already measured frame-deadline lateness.
Only after that distinction should a VO clock, native event-loop scheduling, or
presentation-feedback change be selected. No frame dropping, CPU-copy path,
early A/V presentation, or permanently running redraw timer was introduced to
claim a pass.
