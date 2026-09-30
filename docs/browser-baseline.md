# Browser baseline diagnostic

`scripts/browser_baseline.py` is a development-only, headed Chrome for Testing
harness. It is not shipped in Oxplay and is not a second application frontend.
Only the generated `artifacts/local-1080p60.mp4` fixture is admitted, in a fresh
private automation profile. No personal browser profile is opened. The browser
uses a CDP pipe rather than an exposed debugging TCP port.

## Pinned browser and trust boundary

The harness pins native ARM64 Chrome for Testing 154.0.8037.57 from its
[official version metadata](https://googlechromelabs.github.io/chrome-for-testing/154.0.8037.57.json).
The recorded archive SHA-256 is
`0e6b3439469c1b8b95b2e89c72ea29f7af00fb2c28a8878358a0b6002b6d3a64`;
this is an observed HTTPS artifact digest, **not** a publisher-signed checksum.
Every extracted member is compared with that archive before a run. Extraction
checks size/count bounds, traversal, special files and symlink parents.

The actual upstream artifact is linker ad-hoc signed, lacks a TeamIdentifier and
has unsealed resources. Strict whole-bundle resource signature verification fails;
code-page verification with `--ignore-resources` passes. The harness records these
facts and neither re-signs the browser nor changes Gatekeeper settings. Its trust
basis is official HTTPS provenance plus the recorded artifact digest and exact
member checks, not authenticated Developer ID distribution.

## What the harness checks

The fixture video must remain the same DOM element with the exact admitted local
source. It must be playing at normal speed, unmuted, within the viewport, focused,
and DOM-visible; its natural dimensions must be 1920×1080. CSS size multiplied by
DPR must match the requested physical video dimensions. Event counters detect
pause, seek, reload, volume/rate change, blur, resize and visibility transitions
between warm-up and final observation.

Decoder admission requires actual `VideoToolboxVideoDecoder` and
`kIsPlatformVideoDecoder=true` for the selected player. Chromium's pinned
[Media protocol](https://github.com/chromium/chromium/blob/154.0.8037.57/third_party/blink/public/devtools_protocol/domains/Media.pdl)
provides an optional backend DOM node identifier. The observed build omits it.
The validated alternative requires exactly one player in the selected CDP session,
exactly one unchanged video element and that player's source-verified
[`kLoad` event](https://github.com/chromium/chromium/blob/154.0.8037.57/media/base/media_log_events.h)
matching the fixture URL. Matching is performed in memory; paths are not retained
in player evidence. Additional/historical players, decoder changes, source loads
or destruction during the measured interval reject admission.

Frame/drop counters must advance consistently with playback time, without counter
regression. [`VideoPlaybackQuality.creationTime`](https://w3c.github.io/media-playback-quality/)
is the timestamp of each quality snapshot, not a player generation identifier;
the harness compares its interval with elapsed wall time. Reload continuity comes
from element identity, source/load events and stable navigation time origin.

Measurements cover the browser process tree rather than incremental tab overhead.
Aggregate RSS can double-count shared pages; GPU/unified memory is not added.
CDP process inventories accompany `ps` samples, but short-lived or reparented
helpers may still escape attribution. Optional newly appearing VT services are
only temporally attributed. Host-other CPU records harness/system/foreign work
separately. DNS suppression, background flags and page URL blocking do not prove
whole-browser egress confinement.

DOM visibility/focus does not prove native unoccluded scanout or a waking display.
Unmuted volume and an audio decoder do not prove audible output on the intended
hardware route. Video quality counters are not compositor presentation or energy
measurements. These require separate review before a fair comparison.

## Functional observation — 2026-09-29

```sh
caffeinate -u -t 1
caffeinate -d -i python3 scripts/browser_baseline.py run --warmup 3 --seconds 5
```

The [sanitized functional result](evidence/2026-09-29-browser-functional.json)
records Chrome 154.0.8037.57, revision
`73c14f6228d7cd537c855007e8f88678969cc0eb`, actual VideoToolbox H.264 1920×1080
BT.709 limited-range SDR, AAC mono 48 kHz through FFmpegAudioDecoder, and
1384×778 requested physical video dimensions (692×389 CSS at DPR 2). The same
selected player/source and decoder remained active. Playback advanced 5.143 s;
308 total frames and zero dropped frames accumulated across approximately 5.142 s.

This is **functional validation only**, not a performance gate or evidence that
Oxplay is more efficient than a browser. It used only a five-second sample, an
application build appeared in the host-other CPU record, and newly appearing VT
services were not included in owned attribution. SPEC requires at least 60 seconds
after warm-up and matched codec, dimensions, display, audio, power and helper
accounting before comparison. Raw resource counters are retained for audit but
are not reported as qualified baseline results.

Two earlier three-second startup attempts failed closed because they required the
optional DOM node association; they accepted no measurement interval. The strict
single-player/source association described above then passed the functional run.

## Shutdown and regression coverage

The harness asks the browser to close, reaps its direct child, and checks its entire
process group. Remaining group members receive bounded TERM/KILL escalation;
group disappearance is checked after KILL. An uncertain group keeps its private
profile and clip link, marks the run incomplete, and preserves `result.json`.
Cleanup failures cannot discard collected samples. A disappeared group permits
profile/link removal; filesystem failures remain recorded as incomplete.

The functional run exited 0 without forced termination, confirmed its process
group gone, and removed both the fresh profile and fixture hard link. No cleanup
error was reported. This proves the observed process-group cleanup, not arbitrary
escaped/reparented helper termination.

Sixteen offline tests pass, covering archive admission, bounded CDP framing,
selected-session/player association, exact source fallback, decoder revocation,
frame/visibility continuity and cleanup failures. Python syntax and diff checks
also passed. Run them without opening a browser:

```sh
python3 -m unittest discover -s scripts -p test_browser_baseline.py -v
```

## Full 60-second observation — 2026-09-29 04:03 UTC

```sh
caffeinate -u -t 1
caffeinate -d -i python3 scripts/browser_baseline.py run \
  --warmup 10 --seconds 60 --include-new-vt-services --width 1384 --height 778
```

The [complete sanitized result](evidence/2026-09-29-browser-60s.json) retains all
60 samples. The same VideoToolbox H.264 1920×1080 BT.709 SDR decoder, AAC mono
48 kHz audio, source/player and 1384×778 physical video dimensions remained
active. Playback advanced from 9.980 to 70.114 seconds. The counters advanced
3,608 total frames with zero dropped frames across 60.133 seconds; the navigation,
load/decoder and visibility/focus transition guards remained unchanged.

| Observed aggregate metric | Mean | p95 | Peak |
|---|---:|---:|---:|
| Browser descendant tree + newly appearing VT services, CPU (% of one core) | 23.45 | 28.44 | 32.74 |
| Same selection, RSS (MiB) | 1029.08 | 1133.83 | 1133.95 |
| Other host CPU (% of one core), reported separately | 49.01 | 71.00 | 77.00 |

The coordinated team slot had no deliberate builds or other native tests.
The bounded top-eight host audit contains no `cargo`, `rustc`, `clippy` or
`build-script-build` entries. It does contain WindowServer, wallpaper animation,
existing VT decoder services and ordinary system/agent activity. Absence from a
bounded top-eight list is not proof of zero foreign work. No native scanout,
energy, audible-output route or graphics-allocation qualification was added.

CDP enumerated 11 browser processes at the start and 9 at the end; the sampler
selected 12 then 10 processes. These counts are consistent with one additional
VT service, but this measured harness did **not** preserve `ps` PID membership.
An exact CDP-versus-`ps` PID-set crosscheck cannot be reconstructed from counts.
The `--include-new-vt-services` flag attributes newly appearing services by time,
not proven exclusive ownership; preexisting VT work remained in host-other CPU.
Therefore these aggregate values remain a qualified observation, not a definitive
whole-ownership baseline or an application-versus-browser efficiency claim.

The run exited 0, without forced termination or cleanup errors, confirmed its
process group disappeared, and removed its private profile and fixture hard link.
The evidence records the exact measured harness SHA. A subsequent source change
now exports sanitized endpoint `ps` PID/parent/name membership and CDP set
mismatches using the already acquired tables, without additional sampling.
Seventeen offline tests pass including that inventory/redaction regression; a
future native run must exercise the new PID-crosscheck evidence before that gap
is considered closed.
