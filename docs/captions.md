# Online caption integration

Latest source-associated functional repeat: release `ca21de92` passed the full
guest caption sequence in 71.730 seconds. [The retained log](evidence/2026-09-29-raster-corrected-release-captions.log)
records selected-track confirmation, observed Off, cached reselection using one
private cache file, and reattachment after a second file load during paused
quality replacement. Final native state was VideoToolbox H.264 1280×720 at
59.94 fps, Opus/AVFoundation, sid 1 and no media error; catalog changes/resets
were zero. Exit was zero without forced termination. [Metadata, exact executable
hash and all 125 captured source inputs](evidence/2026-09-29-raster-corrected-summary.json)
associate this run with the corrected build. It used a public guest fixture,
not a real account, and adds no subtitle-glyph, audible-sync, performance or
other-platform qualification. Earlier evidence and limitations below remain.

The provider reads manual `subtitles` and `automatic_captions` entries from the same genuine yt-dlp JSON used to resolve playback. It keeps at most 64 VTT tracks, prioritizes manual tracks and original automatic entries, and reports truncation. The automatic bucket may also contain translations; it must not be labeled exclusively human-authored or exclusively original speech recognition. Missing returned tracks do not prove that the video has no captions.

The exact installed yt-dlp 2026.08.19 [`_video.py` caption path](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_video.py#L4207) creates format-specific signed addresses and may omit tracks requiring a missing proof token. Its `impersonate:true` is a downloader preference: [`common.py`](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/common.py#L894) distinguishes that preference from mandatory impersonation. The new Rust caption fetch does not imitate browser fingerprints or attempt challenges; rejected requests remain errors.

`CaptionUrl` is a separate redacted domain type. It permits only HTTPS `www.youtube.com/api/timedtext`, exactly one matching video `v` parameter and `fmt=vtt`, bounded query/address length, and no userinfo, nonstandard port, fragment or controls. It does not widen content-CDN MediaUrl policy. Signed caption URLs are not logged or persisted.

An explicit selected-track fetch runs off the UI thread, supplies no cookies, ignores ambient proxies, rejects redirects, verifies TLS, and disables automatic retries. It has a 30-second deadline, 10-second connect timeout, cancellable wait, and 2 MiB body bound. HTTP 401/403/429 and unsupported VTT bodies become typed errors. Retry-After is honored with at least 60 seconds; unrepresentably long cooldowns remain blocked for the provider lifetime. VTT header/text/line bounds are checked before handing bytes to native subtitle parsing. This is request-origin control, not whole-process DNS/egress confinement.

## Observed provider evidence

On 2026-09-29, deterministic core/provider tests and strict Clippy passed after the initial implementation. Generated cases exercise URL authority/video/format mutations, output caps and cancellation before transport; raw caption addresses remain redacted. A guest-only live probe used `wsQiKKfKxug`, the installed upstream extractor's multiple-language caption fixture. It returned 2 manual VTT tracks (no truncation), then fetched and validated 79,661 bytes through the Rust HTTPS path. Only counts/byte length were printed:

```sh
cargo run --locked -p serein-youtube --example captions_smoke -- wsQiKKfKxug
```

Automatic/translated live tracks and visual subtitle glyph appearance still need their own qualification. Provider download success alone is not a subtitle presentation pass.

## Shared UI and file lifetime

The app caption-file worker creates private 0600 files under a random 0700 instance directory in the app-owned `Serein/captions` cache and bounds live files to 8 files / 16 MiB. Creation happens on a worker. Final lease Drop only changes state and wakes cleanup; the cleanup thread performs filesystem deletion. The app must retain selected/cache leases through the playback generation, and media additionally retains pending commands through their reply and successfully attached captions through their exact native END_FILE/engine termination. The cleanup owner must outlive App/Player and join only after their destruction. Normal-shutdown cleanup and bounded recovery of unlocked crash leftovers are implemented; legacy system-temp directories from earlier builds are not scanned. The shared Slint caption popup now drives selected-only downloads through the cancellable catalog worker. It caches at most eight tracks per playback, uses native `sub-add cached` for reselection, retires old files only after actual replacement/stop, and reattaches the selected track after an observed quality-change FILE_LOADED event. Native selection confirmation also checks the selected external filename against the requested private lease without exposing that filename in diagnostics. The cleanup owner is declared before App/Player and explicitly joined after both are dropped.

The opt-in `--captions-smoke-test --url https://www.youtube.com/watch?v=wsQiKKfKxug` finite diagnostic exercises paused selection, observed Off, cached reselection, and caption reattachment across a quality change. On 2026-09-29 it passed with exit 0 using the debug app after the central workspace checks (139 tests passed, 3 explicit ignores, formatting and strict all-target Clippy passed). The first track was real English; native exact-file selection, observed Off, cached reselection without a new download, and reattachment after a second FILE_LOADED all passed while retaining one cache file. Cleanup joined successfully after player destruction. Windows protected caption-file creation fails closed until a platform implementation is validated.


## Native evidence and limits

```sh
caffeinate -u -t 5
caffeinate -d -i ./target/debug/serein \
  --url 'https://www.youtube.com/watch?v=wsQiKKfKxug' \
  --captions-smoke-test --snapshot /tmp/serein-captions-repeat.5YqJOQ/captions.png \
  --ui-size 760x600 --ui-theme light \
  --data-root /tmp/serein-captions-repeat.5YqJOQ/state
```

The observed graphics backend was Apple M1/OpenGL 4.1; final native media reported VideoToolbox H.264 1280×720 at 59.94 fps, Opus/avfoundation audio, selected sid 1, two file loads and no media error. Catalog model changes/resets were both zero. The functional run reported no decoder or VO drops, but it spent most of its time paused and included a diagnostic screenshot: it is **not a performance acceptance result**.

The [760×600 logical-window screenshot](evidence/2026-09-29-guest-captions.png) was inspected: the selector, real language, native confirmation and Off action fit without overflow. The popup obscures video, so this image does not qualify subtitle glyph timing/appearance. The [sanitized native log](evidence/2026-09-29-guest-captions.log) contains counts/state only, without signed addresses or private caption paths. Paused selection/quality ownership passes do not establish automatic-caption, Windows, Linux/X11 or native Wayland support.


## Replacement failure handling

The post-diagnostic hardening tracks the accepted native load request, the active
playlist entry's matching request, and a failed-load request. A previous file's
terminal state or stale command failure cannot clear the new caption operation.
An exact load failure, matching started file ending, or explicit stop clears the
caption-owned pending download/selection and loading indicator. Reattachment waits
for the matching active request and an actual file load. Deterministic cases cover
stale failures, failure without START_FILE, terminal states, and stop. This source
change and the compact popup passed the follow-up central checkpoint (143 tests,
strict Clippy) and another 70-second native caption diagnostic. The updated
760×600 screenshot shows Close and “English captions on”; exact-file selection,
Off, cached reselection, and paused quality reattachment passed again. The media
coordinator obtained a fresh position before reopening; the native log records
two started/loaded files and no playback error. This repeat still preceded the
private-cache crash-recovery change described below.


## Private-cache recovery hardening

The new file worker takes the app-owned `Serein/captions` path. Its startup runs
on the cleanup worker and creates/checks private profile/cache directories.
An exclusive registry lock serializes startup/recovery and final directory removal;
lock acquisition has a two-second deadline. Each random instance holds its own
exclusive lock until cleanup ownership ends. Other live instances are preserved.
Recovery is limited to 128 known instance directories, each with at most eight
canonical VTT files plus its lock. Unknown entries, unsafe modes/ownership,
symlinks and hard-linked files fail closed.

Both enumeration (`fdopendir` with a CLOEXEC duplicate) and mutation
(`openat`/`unlinkat`, no symlink following) use opened directory descriptors.
A renamed path cannot redirect the recovery scan into another directory. No broad
system temporary-directory scan occurs. Legacy temporary directories from earlier
development builds are deliberately not searched or removed. Seven focused caption tests passed, including live-owner preservation, unlocked
crash leftovers, symlinks, bounds, cold profile creation, and directory replacement
races. Strict app all-target Clippy passed. The subsequent packaged b1f4074 70-second
caption diagnostic passed with this private-cache location and clean shutdown;
see [packaging evidence](packaging.md). That run predates the coordinated-clear
and engine-held playback-lease changes below. This does not promise forensic
erasure, filesystem availability, or validated Windows protected-file behavior.


## Coordinated local-data clearing

The explicit library **Clear local data** action now cancels catalog/caption work,
blocks new catalog and preference/history submissions, stops playback, releases UI
caption references and requests a worker-owned purge. It deletes only known private
caption files and bounded unlocked crash leftovers. Another live application
instance prevents a successful all-cache acknowledgement; close it and retry.
The account vault and remote YouTube account are outside this action.

Playback retains successfully attached caption leases until the matching native
END_FILE or complete engine destruction, while pending `sub-add` commands retain
an independent lease until their reply. The source contract is mpv v0.41.0
[`terminate_playback`](https://github.com/mpv-player/mpv/blob/v0.41.0/player/loadfile.c#L1937):
subtitle/demux teardown completes before END_FILE. A stop command reply or
`idle-active` value is **not** that guarantee. The asynchronous
[`mp_add_external_file`](https://github.com/mpv-player/mpv/blob/v0.41.0/player/loadfile.c#L817)
may attach after a new playback begins, so replacement is rejected while a caption
add reply remains pending. Normal playback replacement can be retried once that
bounded operation completes.

The cleanup worker acknowledges only after all creators and final leases retire
and confined unlinks succeed. Partial writes remain tracked even when deletion
fails; an explicit retry retries failed deletions. Missing known files count as
already deleted. An epoch rejects stale download/file results after a timeout or
superseding clear. No filesystem deletion, directory scan or wait runs on Slint.
A 15-second file-phase deadline reports incomplete cleanup without deleting leased
files or claiming that the library was cleared. The library transaction begins
only after the caption acknowledgement and the [artwork purge](artwork-cache.md);
once accepted, its own typed terminal
response is required. Unrelated startup/read failures cannot finish that phase.
Committed privacy defaults and empty local models are installed before reopening
admission, so a later volume/theme change cannot resurrect an old history opt-in.

Focused app regressions passed, and the media suite passed 18 tests with one
explicit synthetic-TLS-server ignore. Behavioral validation covers final engine lease retention, Off versus
unload, pending-add replacement rejection, stale creators/purge identifiers,
search during clear, failed initialization response correlation, and injected
partial-write plus failed-unlink recovery. Native coordinated-clear acceptance is
recorded separately after the isolated opt-in diagnostic; these source changes
alone do not establish that result or forensic erasure.


### Executed isolated clear diagnostic

The new `--clear-local-smoke-test` ran for 85 seconds and exited 0 on macOS/M1.
Release binary SHA-256: `6791c634802d1649b5d4ff26a7c5cfd6d3e1c334527a527760fe26233edcf874`.
It requires a public URL and an absolute `--data-root` that **does not exist**;
startup creates that root privately and rejects existing directories or symlinks.
This prevents the diagnostic from deleting a normal profile.

The run created one visibly labeled test collection, completed real guest caption
selection/Off/reselection and paused quality replacement, then invoked the actual
clear action at 67 seconds. A concurrent search was rejected by the admission
barrier. At 80 seconds the application verified acknowledged deletion, inactive
playback, empty collections, the Off-only caption model, unchanged media load
count and restored default preferences. After exit, a read-only SQLite check found
zero collections/items/follows/history, one default-preferences row, and no VTT
files. [Native log](evidence/2026-09-29-clear-local-native.log) and
[post-exit counts](evidence/2026-09-29-clear-local-native.json).

The window became occluded at 9.004 seconds and correctly paused; this mostly
paused run validates ownership/deletion behavior, not presentation or performance.
The selected decoder/audio observations were VideoToolbox/Opus/AVFoundation.
The idle snapshot retains some last-observed stream fields; `Idle`, load identity
invalidation and confirmed cache deletion establish the terminal state, not a
stale subtitle-ID field alone. No account credentials or remote writes were used.
