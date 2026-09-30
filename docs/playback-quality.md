# Maximum playback quality and resume

The shared Slint playback-settings popup offers explicit ceilings from 144p to
1080p. These are labeled “Up to …”, not advertised as a list of exact formats.
The resolver chooses the highest available resolution/frame rate within the
selected ceiling, preferring H.264 only at equal resolution/frame rate. It does
not lower quality to meet a performance budget. New videos initially use 1080p
maximum. Live/4K/HDR remain unqualified.

A user change submits one cancellable request to the existing supervised guest
resolver. Current playback continues while extraction runs. Failed extraction
leaves the old stream active and reports the error. Successful resolution checks
the current video identity and header policy, then reloads the same long-lived
player at its current position and preserves pause state. The popup retains the
active ceiling until resolution succeeds. Public playback remains anonymous even
when account browsing is connected; this control does not escalate credentials.

The installed mpv 0.41 manual defines `loadfile <url> <flags> <index> <options>`.
The resume position is a per-file `start` option (fourth argument, with `-1` for
unused insertion index), avoiding a seek racing FILE_LOADED or leaking to the
next video. Volume/speed and the GPU presenter are retained. Player initialization,
texture allocation and graphics context are not reconstructed by this control.

## Executed native check

The explicit `--quality-smoke-test` used a real public URL, an isolated local
profile, and three anonymous extractions. In the debug run:

- Initial embedded playback observed H.264 1920×1080 at 60 fps, VideoToolbox,
  Opus/AVFoundation audio.
- Changing to a 720p ceiling while playing produced height 720 and preserved
  forward playback position (23.933 s before selection, 42.700 s at the later check).
- After pausing at 42.700 s, selecting a 480p ceiling produced 854×480 at 30 fps,
  still paused at exactly 42.700 s with VideoToolbox active.
- No media error was observed. The process exited 0 after all assertions. There
  were zero catalog changes/resets and only two persistent target allocations.
- At the 70-second checkpoint and 80-second exit, the paused run had identical
  draw/event/display-clock counters; the clock and playback sleep assertion were off.

The intentional quality reductions exercise a user control; they are not 1080p60
performance results. This network/debug run had startup/playback drops and concurrent
build work. No A/V perception or resource-gate pass is claimed. Per-file audio
attachment is being hardened separately against queued FILE_LOADED events; rerun
this test after that change. Local log: `artifacts/quality-native.log`.

```sh
target/debug/oxplay --data-root /absolute/isolated-profile \
  --url 'https://www.youtube.com/watch?v=aqz-KE-bpKQ' \
  --quality-smoke-test --diagnostics
```

The test requires network access and an active native display. It does not import
credentials. Early exit or a failed assertion is a failure, not a skipped pass.

## Repeat after file-scoped tracks and verified TLS

The later debug binary explicitly enabled TLS peer verification and the reviewed
Mozilla CA resource. Genuine 1080p60 H.264/VideoToolbox and Opus/AVFoundation were
observed playing at both 5 and 10 seconds. At 12.136 seconds the native window
became occluded; playback correctly paused at 9.350 seconds and stopped its
display clock/sleep assertion. It remained hidden, so the 30-second quality-test
precondition failed and the process exited 101. This is **not** a completed
quality-switch repeat. The log is `artifacts/quality-tls-native.log`; an active
uninterrupted native window is still needed to complete all stages.
