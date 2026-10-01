# Actual YouTube-site comparison

`scripts/youtube_browser_baseline.py` launches the same pinned native ARM64
Chrome for Testing as the [local browser diagnostic](browser-baseline.md), in a
fresh throwaway profile over a private CDP pipe. It opens only the explicit
public video supplied with `--url`. It never imports credentials, cookies or a
personal browser profile. The existing local harness remains unchanged.

Prepare the pinned browser outside the measurement window:

```sh
python3 scripts/browser_baseline.py prepare
```

After the native app's functional qualification, choose a public video longer
than the warm-up and sample interval, with an available 1920×1080 stream at
approximately 60fps. Record the app's actual selected codec and physical video
rectangle, plus its audio codec, sample rate and channels. Use those observed
values in the website comparison:

```sh
caffeinate -d -i python3 scripts/youtube_browser_baseline.py \
  --url 'https://www.youtube.com/watch?v=PUBLIC_ID11' \
  --codec h264 --width 1384 --height 778 \
  --warmup 10 --seconds 60 --include-new-vt-services --macos-footprint
```

The ID and dimensions above are placeholders. The actual video rectangle must
match; the window size is a separate quantity. `--window-width` and
`--window-height` allow native browser-window calibration before sampling, with
1320×860 as the default. The harness uses normal website layout and quality
controls and does not alter video CSS or intercept media traffic to manufacture
a match. A geometry mismatch preserves sanitized observed state and admits no
resource interval. Use that state to choose equivalent app/browser geometry
for the next explicit run.

Startup waits at most 60 seconds for ordinary consent/advertisement handling.
If presented, it clicks an exact “Reject all” consent control before sampling.
It selects the normal 1080p quality menu item, requests unmuted playback, and
waits for actual 1920×1080 frames. Unavailable content, a challenge, missing
quality controls, or unresolved consent fails admission. No account or challenge
workaround is attempted. Setting 1080p cannot guarantee a particular codec:
the actual selected CDP track must equal `--codec`, or the run is rejected.

`--comparison default-playback` instead accepts the site's observed allowlisted
codec and decoder, including software decoding, while keeping the same 1080p60,
audio, visibility, stable element, throughput and zero-drop gates. `--codec`
still records the app's reference codec. Results explicitly record whether the
codec differs and cannot establish a native API advantage. This mode allows
one physical pixel per axis for normal site CSS rounding; the default
`matched-codec` mode retains its 0.1-pixel tolerance and hardware requirement.
Neither mode isolates backend cost from the complete website/app workloads.

Use `--admission-only` for finite window calibration: it stops after the
two-second throughput preflight, closes the owned browser, and labels its result
as a diagnostic with no resource interval. Failed admission still records
sanitized geometry. It never produces a performance comparison.

After a ten-second warm-up, a two-second preflight must prove at least 50fps
and continuous forward playback. Both preflight and the subsequent 60-second
interval require one unchanged video element, the selected public video ID,
no advertisement, a finite duration, visible/focused/in-viewport playback,
normal speed, unmuted volume and matched physical dimensions. The default
`matched-codec` mode also requires actual `VideoToolboxVideoDecoder` hardware
decode. DOM-node association identifies the
current CDP player when available. Otherwise its live source must uniquely match
the current element; source hashes are transient in memory and never exported.
A historical advertisement decoder cannot qualify a current software player.

The interval rejects pause/seek/reload/rate/volume/visibility/focus/resize/scroll
or advertisement transitions, source/decoder/track changes, insufficient frame
progress, and dropped video frames. Before/after state and exact frame/drop
deltas accompany the samples. Completed evidence still needs an external review
of actual audible output, display wake/occlusion, audio equivalence, power mode,
network conditions and unrelated work. DOM visibility and submitted frame
counters do not prove native scanout or perceived A/V synchronization.

Results cover the whole owned browser tree, with the same one-second process
CPU/RSS sampling, bounded per-process breakdown and separate host-other CPU
as `scripts/measure.py`. Optional new VideoToolbox services are temporally
attributed, never proven exclusively owned. Optional macOS footprint is a
separate OS ledger; unavailable entries are explicit and it is never added to
RSS or GPU memory. Preexisting decoder services, short-lived/reparented helpers
and shared-page double counting remain practical limits.

During each one-second resource tick, the online sampler drains queued CDP
events without issuing browser or DOM commands. Each drain stops at a five
millisecond budget check, 256 messages, or 256 KiB of combined received/parsed
bytes. Partial and unprocessed frames stay in the bounded 4 MiB framing buffer;
no command reply or event is silently discarded. Numeric per-tick telemetry
records bytes, messages, retained input and budget exhaustion. Two consecutive
ticks with an exhausted budget and remaining input invalidate the interval.
An individual parse/event handler can finish beyond the time budget; there is
no background reader or concurrent command handling.

The script writes only sanitized numeric playback state, allowlisted codec and
decoder names, public video IDs, process membership and measurements. Browser
stdout/stderr are discarded; signed stream URLs, cookies and provider responses
are not written. Cleanup closes the browser and checks its owned process group,
with bounded TERM/KILL escalation. Uncertain cleanup marks the run incomplete
and retains the temporary profile until ownership is resolved. Successful
cleanup removes it and preserves `result.json` under
`artifacts/youtube-browser-baseline/run-*`.

Failed admission also preserves sanitized playback state and bounded live-player
counts before parsing the selected decoder. Startup failures record a fixed
substep plus numeric video/menu state. These diagnostics distinguish missing
association, unavailable track metadata, codec/hardware mismatch and quality
selection failure without exporting provider labels or media URLs. Failed runs
remain incomplete and cannot be used as qualified performance comparisons.

This is a full website versus full native-app workload. A minimal local HTML
video baseline isolates the media path more closely. Do not attribute their
difference solely to Metal/OpenGL: website chrome, network requests, codec,
audio format, ambient effects, ads outside sampling and startup/extractor work
are separate costs. If identical codecs or geometry cannot be established,
report that limitation instead of promoting a mismatched run.

The authored harness has offline regression coverage; a successful real-site
run must be recorded separately:

```sh
python3 -m unittest discover -s scripts -p 'test_*browser_baseline.py' -v
```
