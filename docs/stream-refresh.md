# Stream replacement and expiry

The shared UI now coordinates quality changes and known stream expiry in
`crates/app/src/playback_ui.rs`. The finite native guest diagnostic passed on the TLS-verified direct path;
a later scoped HTTP functional repeat also passed with unresolved diagnostic
observations documented below. Neither run is a performance qualification.

A single-shot timer targets 60 seconds before the earliest declared video/audio
expiry. Unknown expiry never triggers an automatic retry. A distant deadline is
rechecked at most once per day to bound timer arithmetic. Paused, hidden or busy
playback defers extraction until an ordinary state/result event makes it eligible;
there is no background polling timer. The existing single supervised extractor
performs at most one automatic refresh per explicit video selection. Quality
changes preserve that budget. Cancellation, errors and HTTP 403 responses do not
reset it or authorize account authentication.

After resolution, the coordinator requests a separately tokened asynchronous
native playback position. It does not use the display clock, which correctly
stops when controls are hidden. A three-second one-shot deadline bounds the UI
wait. New worker generations, replacement selections and timeout cancel the exact
position request; late replies cannot become another operation's position.
Accepted seek/pause actions invalidate a pending or delivered sample. Before
loading, the coordinator checks video identity, worker generation and presenter
availability again. The player keeps its existing context and loads at the fresh
position with the current pause state. Cached caption selection is reattached
only after the exact replacement file is observed on the supported guest path.
Account captions remain unsupported.

If a position is unavailable, the UI explains the failure and leaves the current
selection intact. An expired or inaccessible stream cannot always be resumed;
this implementation does not reinterpret arbitrary failures as expiry or loop
through credentials, client identities or retries. The implemented account path
retains the original session scope for quality/expiry resolution and native
replacement. It validates both selection and session generation plus revocable
media authority before asking for a position and again before loading. It never
falls back to guest extraction or direct unguarded media. Sign-out/expiry cancels
pending native-position replies and releases that operation's busy state without
discarding unrelated account work. Same-session repeated extraction preserves
the playing stream's authority while replacement is pending. These account
transitions have synthetic/component checks only; the executed native diagnostic
below is guest playback. See [account media](account-media.md).

`--refresh-smoke-test --url URL` is an explicit 70-second live diagnostic. It
changes only in-memory expiry metadata, seeks and pauses a genuine public video,
verifies that paused expiry starts no extraction, resumes with controls hidden,
and requests one refresh. It then checks that the reopened stream used a fresh
position and that another expiry does not cause another extraction. Signed media
URLs are not changed or printed. This is separate from normal operation and from
performance measurement.

## Executed native result

The 70-second diagnostic exited successfully on macOS/Apple M1 with debug binary
SHA-256 `bedd6b66e75fb1cfd7704cb1b08ad3dc3138be880ea2cad7b2e09d76d8b1ce68`.
Playback used VideoToolbox H.264 1920×1080 at 60 fps and Opus/AVFoundation.
A seek established 45 seconds; expiry while paused started no request. Controls
were hidden and their progress timer stopped. The later once-only refresh loaded
at 52.700 seconds, proving it did not reuse the stale 45-second UI value. A second
forced expiry started no additional extraction/load. There were two native file
loads, two persistent targets and zero catalog changes/resets.

[Sanitized native log](evidence/2026-09-29-stream-refresh.log). The diagnostic
modifies in-memory expiry metadata; it does not wait hours for a real CDN URL to
expire. It does perform genuine re-resolution and native replacement. Debug
execution, competing host work, seeks and file replacement make its drop counters
unsuitable for a performance pass. Real deadline expiry, failed refresh and
other-platform native qualification remain separate tests.


## Scoped HTTP functional repeat

Release `785ed516f5d703031b1756161ea7a9676197ca963665fc2a6ffe2db3a090222d`
passed the 70-second guest diagnostic using `--scoped-media` and the public
`aqz-KE-bpKQ` fixture. The log records paused expiry without a new extraction,
a fresh 49.933-second native replacement position, two total loads and no second
automatic refresh. Controls were hidden during resumption; the repeated cached
position value is therefore not a continuous playback clock. Catalog changes and
resets stayed zero. The native path observed VideoToolbox H.264 1080p60 and
Opus/AVFoundation. [Native log](evidence/2026-09-29-account-integration-refresh.log)
and [binary/source provenance](evidence/2026-09-29-account-integration-summary.json).

The released initial source reported 23 read errors without category information.
They remain unexplained by that log, not presumed benign cancellation. The
replacement's cached audio sample-rate field was zero (unknown/unavailable),
while codec/output names remained Opus/AVFoundation. This does not establish
audible output, silence or decoder failure. Later classified transport errors
and a finite fresh-property probe were exercised separately in the repeat below.
The retained `aedd865c` attempt failed its two-load assertion; the successful
repeat does not erase that failure or supply missing causes. Both binaries came
from an evolving working tree without a frozen build-input capture. These are
public guest functional tests; authenticated refresh and perceptual A/V
qualification remain open.


## Classified transport and fresh audio repeat at `287a7c0`

Release SHA-256
`9a48e90f8a7f75adba352ae11d3a2f3df3f0f28d98cf3a71f184a2b1b776a799`
passed the same scoped guest diagnostic in 71.074 seconds without harness timeout.
It retained two loads and resumed at a fresh native **51.550 seconds**; paused
expiry started no extra load and the second forced expiry did not refresh again.
Both [fresh audio probes](evidence/2026-09-29-account-integration-refresh-9a48e9.log)
were correlated with the replacement load: decoder and output sample rates were
48,000 Hz, and audio PTS advanced from 79.037692 to 84.038267 over approximately
five seconds. The coalesced snapshot's rate remained zero/unknown. The fresh
queries demonstrate that it was not the actual decoder/output rate; they do not
establish audible quality or perceptual A/V synchronization.

All four released sources recorded zero errors in every classified category:
1 + 8 + 17 + 2 completed range reads, validating 28 MiB total. This repeat did not
reproduce the earlier 23 errors and does not retrospectively identify them.
VideoToolbox H.264 1080p60/Opus/AVFoundation remained observed, with zero catalog
changes/resets. GPU/resource/energy acceptance was not measured.

The [post-build input manifest](evidence/2026-09-29-account-integration-source-9a48e9.json)
contains 108 relative source/configuration hashes. Every hash was verified against
commit `287a7c0c0d64a62df55ce9011ebadbe9791706f5` after that checkpoint was committed.
This establishes the recorded source association without assigning the earlier
dirty binaries an invented commit. Native dependency provenance remains separate;
this is not an independent reproducible rebuild. [Run metadata and all evidence
hashes](evidence/2026-09-29-account-integration-summary.json).

## Corrected-build guest repeat at `ca21de92`

Release `96ad51a9d54014204b49925e00d8e59a47f01780513f7c011e033aee65d82645`
passed the finite guest refresh diagnostic in **71.606 seconds**, exit zero with
no forced termination. The paused position remained 45 seconds while expiry was
deferred; the real re-resolution loaded at a freshly sampled **51.650 seconds**
after resuming with controls hidden. Exactly two loads were observed, and a
second forced expiry did not cause another replacement. Fresh replacement-scoped
audio probes returned 48,000 Hz for decoder and output, with audio PTS advancing
from **78.839638 to 83.840191**. The coalesced sample-rate field remained unknown
at zero; the finite replies supply the actual queried rates without adding a
permanent observation stream.

VideoToolbox H.264 1920×1080/60 and Opus/AVFoundation were observed, with no media
error or unrelated catalog changes/resets. The [full log](evidence/2026-09-29-raster-corrected-release-refresh.log)
and [run/build summary](evidence/2026-09-29-raster-corrected-summary.json) retain
the exact 125-input source association. This is a guest functional regression
using explicitly changed in-memory expiry metadata, not a naturally expired
URL or authenticated-account test. It does not establish audible quality,
perceptual A/V synchronization, HTTP range error totals, or steady performance.
Earlier failures and unclassified transport observations remain as recorded.
