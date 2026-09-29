# Explicit guest request recovery

A failed guest search, channel/playlist page or initial video extraction can
retain one **Retry search**, **Retry catalog** or **Retry video** action. It
retains the submitted typed request, opaque page cursor and selected quality;
it never reconstructs a request from edited search text. Retrying a catalog
request does not push its navigation history again. Initial video retry goes
through the ordinary resolver and selection handoff without recreating the
player.

A current guest file that actually fails in the media engine offers **Retry
from start** through the shared Play control and its keyboard action. It resolves
the retained video identity again at the active maximum quality, then starts
from the beginning; it does not reuse the failed signed stream URL or guess a
recoverable native position. Current metadata remains until resolution succeeds.
The action requires the exact accepted native load, normalized guest metadata
with session generation zero, and no retained account authorization, including
an expired authorization. Account and local-file failures do not fall back to
guest resolution. Volume and speed remain owned by the existing player.

A resolved restart now rechecks the exact failed native load at its final
handoff. Guest recovery performs this check before clearing any account state,
so a late guest result cannot revoke a newer account playback lease. Account
recovery likewise rechecks the live session/lease and failed load before loading.
Both preserve the latest accepted user pause intent during extraction; a pause
or navigation action is not undone when resolution completes. Ordinary new
video selections still begin playing, and terminal recovery still starts at
zero rather than inventing a trustworthy position from the failed snapshot.
These changes have source review and compilation/lint validation only in this
slice; the historical runtime evidence below does not qualify them.

If that fresh resolution fails, repeated transport retries share its existing
manual delay and attempt count. A nonretryable provider rejection disables
restart for that exact failed load and preserves its explanation. A stale
worker failure or an unrelated catalog operation cannot remove that block.
This is explicit recovery, not an automatic playback retry loop.

For failed provider requests, only `Offline`, `Timeout`, `ExtractorFailed` and
`RateLimited` offer a retry action.
Invalid input, unavailable/restricted content, authentication or proof requirements,
unsupported formats/platforms, malformed/oversized output and security-policy
rejections retain their distinct explanation. Cancellation is not a retryable
failure. Comments, captions, quality/expiry replacements and account operations
retain their separate controls and ownership; this action never authenticates a
guest request or retries an account mutation.

Manual retries wait 2, 4, 8, 16, 32 and then at most 60 seconds between failures.
Rate-limited failures always wait at least 60 seconds. The extractor currently
exposes a typed rate-limit category, not a server `Retry-After` value; its own
shared 60-second provider cooldown remains authoritative and is not bypassed.
There is one weak, single-shot readiness timer, with no ticking countdown,
polling or automatic retry. Explicit further retries remain possible after the
capped delay; a user who later restores connectivity is not permanently locked
out by a retry-count limit.

Every retained request-retry action belongs to a worker generation, page, account-session generation
and account-selection epoch. A new request, cancellation, navigation or account
transition synchronously retires it. The callback rechecks that scope and the
monotonic deadline before admission. A checked, non-reused failure serial also
binds a full-message dialog to the error it captured: an old dialog cannot retry
a newer failure. Serial exhaustion disables recovery. Worker publication rejects
stale results before the error handler; that handler also checks its delivered
generation before changing recovery state.

## Validation scope

Deterministic tests cover the complete provider-error classification, retained
search/channel/playlist/video parameters, cooldown rejection, single consumption,
stale generations, navigation/sign-out/selection scope, old-dialog serials,
serial exhaustion and bounded long retry sequences. The worker regression checks
that typed submission observers run after retirement and never admit blocked
work. These tests do not manufacture a successful provider response.

The opt-in `--recovery-smoke-test` is a separate, labeled offline failure fixture.
It requires exactly `--yt-dlp /usr/bin/false` and a new absolute private
`--data-root`, uses a fixed 20-second watchdog, and rejects other content,
account, transport and helper inputs. The actual supervised local executable
exits unsuccessfully; ordinary provider classification produces `ExtractorFailed`.
No catalog metadata or playback result is fabricated.

Eleven finite stages exercise real search, retry and navigation callbacks across
search, channel and initial-video requests. They check cooldown rejection, an
accepted same-request retry, stale-dialog rejection, retirement and no later
resurrection. The driver expects five admitted worker requests, **not** five
instrumented OS launches, and asserts zero published rows, media loads and
connected accounts. It opens the full message at 13 seconds. An optional snapshot
must be a direct PNG child of the new private root and uses the existing one-shot
capture/encoder at 15 seconds. The fixture does not qualify real YouTube error
recovery, account behavior, keyboard/screen-reader access or resource budgets.

The first focused application compilation failed because the concurrent library
migration derived `Debug` for a type containing `ChannelId`; its failure remains
recorded in the session evidence. After that separate fix, the centralized
`cargo test --workspace --locked` run passed **354 tests**, with four explicit
external integration tests ignored. The application contributed 157 passing
tests, including all seven recovery-state tests, the real worker admission-order
regression and the restricted diagnostic CLI test. Formatting and strict
workspace/all-target Clippy passed. The first lint run found a test-only
`let_and_return` in the library observer helper; after its correction all four
observer tests passed again. The
prepared native harness subsequently ran against frozen source `578a14a`
(binary SHA256 `47cd30afd3f5453083d0af276af9dcd664b9cbc4a9a4a2ea54d3ee304248a404`).
All eleven functional stages passed in22.052s with five admitted requests,
exit0 and both owned process groups absent. However, actual inspection of the
1520×1200 capture found that ordinary decoder updates had erased the fixture
label. This attempt does **not** pass the labeled-diagnostic contract. Its raw
evidence and separate visual failure review remain in
`artifacts/library-recovery-v1/recovery-release`.

The correction separates immutable fixture identity from decoder warnings and
includes it in both the footer and captured full message. Every finite stage now
asserts label persistence; stage13 also checks the actual message text. All157
application tests pass after this correction. A corrected native rerun remains
pending for that frozen release. Subsequent product changes passed all364 Rust
tests (four external integrations ignored), formatting, locked development
build and strict Clippy. The revised native offline check passed all11 stages
with clean owned-process teardown. Its inspected760×600 dark capture confirms
persistent fixture labeling, correct video-error context, bounded message text
and modal dimming. This working-tree development check is retained in
`artifacts/usability-v1`; no live YouTube failure recovery is claimed.

Guest browsing now owns separate loading, error and empty-page text. A terminal
catalog failure clears its loading subtitle even when it cannot be retried.
Failed video extraction preserves a previously acknowledged catalog, including
genuinely empty results; without one, it shows video-specific context. Typed
request generations reject stale completion and cancellation updates. Error
text no longer inherits an unrelated global status string or expands beyond
the empty-state panel.

The subsequent failed-playback restart implementation passes five focused
regressions covering accepted-load identity, guest/session classification,
stale terminal events, preserved request backoff and nonretryable failure
retirement. The full workspace passes369 tests (four explicit external checks
ignored), the locked development build, formatting and strict Clippy. Actual
native failure-to-success restart is not yet qualified; no account restart or
new performance result is claimed.
