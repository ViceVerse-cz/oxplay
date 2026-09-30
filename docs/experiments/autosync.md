# Audio-clock smoothing diagnostic

`OXPLAY_DIAGNOSTIC_AUTOSYNC=1` selects libmpv `autosync=30`; the production
default remains `0`. `Snapshot.autosync_factor` and the diagnostic startup line
record the selected factor. This changes neither the audio output selection nor
the video codec, resolution, or decoding path. It requires separate A/V
qualification before any consideration as a default.

The source basis is mpv v0.41.0
[`update_avsync_before_frame()`](https://github.com/mpv-player/mpv/blob/v0.41.0/player/video.c#L590-L623):
with audio-master timing and default autosync, `ao_get_delay()` directly
influences the next frame's target time. A nonzero factor smooths the difference
between reported audio delay and the prediction from prior timing. The source
warns that the implementation's behavior depends on how often it runs, including
frame rate. The installed mpv 0.41 manual and its
[upstream option description](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/options.rst)
recommend trying factor 30 for imprecise audio delay reports and describe a
one-to-two-second settling period after abrupt A/V changes.

This experiment follows measured callback-to-UI handoff of approximately 0.863 ms
on average, compared with 14.3 ms average render-start deadline lateness. The
actual host audio output was AVFoundation after CoreAudio initialization failed.
AVFoundation's
[`feed()`](https://github.com/mpv-player/mpv/blob/v0.41.0/audio/out/ao_avfoundation.m#L61-L86)
uses 100 ms sample requests and synchronizer time to report the audio position.
This motivates testing clock smoothing; it does not establish the cause of the
late frames. A timed null-output diagnostic also failed the frame-loss gate.

The controlled comparison uses audible output, unchanged 1080p60 fixture,
`OXPLAY_VIDEO_LEAD_MS=0`, `OXPLAY_VIDEO_PREPARE_MS=0`, and
`OXPLAY_MEDIA_TIMING=1`, with the same 10-second and 25-second checkpoints.
No successful outcome is implied by the existence of this diagnostic.

The completed 30-second run (`artifacts/autosync-30.log`) observed
`autosync_factor=30`, audible AVFoundation output, and active VideoToolbox.
At 10 seconds there were 192 VO drops; at 25 seconds there were 495:
303 / 900 expected frames, or 33.7%. Decoder drops remained zero. The run ended
with 1,191 video draws and 591 VO drops. Mean newest-callback-to-render latency
was 2.83 ms (maximum 49.0 ms); mean render-start deadline lateness was 9.55 ms.
The optimized frame-loss gate still fails. This result does not justify changing
the default or claiming the host AO clock is the sole cause. Native timing and
presentation remain unresolved; no additional speculative tuning was selected.

The [retained raw log](../evidence/2026-09-29-autosync-rejected.log) comes from
commit `e9dee47`, binary SHA256
`87873744e6dcb98e5ccbec0354d54b94ad98a6336de7b6acd57b8dd541701daf`.
