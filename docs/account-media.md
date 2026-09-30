# Account media authorization boundary

The provider handoff, shared UI coordinator and guarded native transport are
implemented. Account-specific validation is synthetic only.
It does not establish real-account playback, private-content access, identity
propagation, authenticated advertisement suppression or a production capability.
No real credentials, browser profile, account request or mutation was used here.

## Provider handoff

`AccountClient::resolve_authenticated_with_policy(&mut self, &YtDlp, &VideoId,
ResolutionPolicy, &OperationContext)` returns `AuthorizedPlayback { playback,
authorization }`. The default-policy `resolve_authenticated` convenience returns
the same type. `playback` remains a normalized `ResolvedPlayback` with
`guest=false` and its account generation. Errors stay typed as
`AuthenticatedResolveError::Account(AccountError)` or `::Resolver(ProviderError)`.

`AccountPlaybackLease` is cloneable, Send + Sync and nonsecret. It exposes
`generation()`, `is_valid()`, `valid_for()` and async `revoked()`.
`valid_for()` gives a known remaining monotonic lifetime for a single-shot UI
timer: None only for a live unknown expiry, and Some(Duration::ZERO) for revoked
or expired authority. This allows expiry while paused without a polling task or
a Tokio reactor on the UI thread. It holds only a generation,
revocation state, notification and expiry bounds—no cookies, identity, signed URLs
or reference to the account provider. The session control stores a weak lease
registration, avoiding an ownership cycle. Repeated extraction for the same
session and expiry reuses the authority; it does not revoke the playing stream
while a quality replacement is awaiting a fresh native position. A changed
expiry cannot silently replace existing authority.

Only an explicitly invoked extraction from a verified supported identity can
issue the lease. Nonzero account slots and multiple/delegated identities remain
unsupported because no source-verified extractor selector binds those identities.
Possession of an imported cookie file alone never grants playback authority.

All retained cookie expiries are checked conservatively before the helper and
again before publishing; SAPISID must cover the HTTPS YouTube watch path. Session
cookies with expiry zero have no invented expiry and remain explicitly revocable.
A lease's known expiry has an anchored monotonic deadline as well as a wall-clock
check. `revoked()` registers its Notify waiter before inspecting state; explicit
invalidation wakes every waiter, late subscribers finish immediately, and finite
expiry uses a single sleep rather than polling. No task/timer exists when nobody
awaits the lease. Provider-detected expiry revokes it without advancing the
operation generation, so the current worker can still deliver SessionExpired.
Sign-out/account switching advances SessionControl and revokes synchronously;
dropping/replacing the verified session also revokes retained leases.

## Shared UI and media integration

An account playlist video has a separate **Play with account** action. Its title
continues to request guest playback. The account action requires a verified
supported identity, no unresolved account mutation, an available guarded media
transport, and an available presenter. Normal search, public playback, failed
guest extraction and startup never select account authority automatically.

`resolver::SharedResolver` lazily creates one `Arc<YtDlp>` on an owned worker.
Catalog and account jobs share its helper admission and cooldown state; UI
construction performs no helper validation or I/O. The account worker retains
one operation/result slot. Playback cancellation discards only authenticated
resolution and its pending result, including errors. It never cancels an
unrelated account read, mutation or reconciliation, and does not manufacture a
completion acknowledgment while the old helper is still being reaped.

`account_playback.rs` records video, quality policy, initial/replacement reason,
selection epoch and session generation. A result must match that complete scope;
expired authority during the worker-to-UI handoff is an explicit expiry failure,
including initial playback where there is no installed lease to observe yet.
Quality replacement and the once-per-selection URL refresh retain account scope
and require a fresh, correlated native position. They never call the guest
resolver as a fallback. The previous stream continues until replacement is
accepted. See [stream replacement](stream-refresh.md).

`media_network::load_with_authorization` validates the playback/lease generation,
then constructs guarded video/audio sources. `AccessLease` is checked on open,
redirect admission, in-flight requests, and reads from the bounded compressed
cache. Revocation wakes blocked network operations. Cookies are never copied
into those sources. The direct libmpv URL entry point accepts guest generation
zero only; missing account transport is a visible failure, never a direct-load
fallback. The macOS native resolver helper remains part of this process tree;
whole-process proxy/DNS/network qualification remains open.

Known session expiry uses a weakly captured single-shot Slint timer, including
while paused. Distant deadlines rearm at most once per day; unknown expiry adds
no polling timer. Sign-out, replacement account import/reconnect and observed
expiry cancel account resolution and stop installed account media, clear its
signed URLs, captions, current video, descriptions, title/channel and displayed
texture, and invalidate pending native-position replacement. Guest playback is
kept when only an uninstalled account request is cancelled. On an account-to-guest
transition, `guest_playback.rs` retains one resolved guest result and clears the
account player first. It waits for terminal stop and a fresh native confirmation
that the previous video output was destroyed before submitting the guest load.
Guest generation changes cancel this handoff; a three-second single-shot
deadline releases the busy state on failure without loading the result. No
polling or anonymous fallback is introduced. Ordinary restart/decoded-width
checks alone are not sufficient for this privacy boundary: mpv can retain an old
frame across file changes. See [the source-reviewed video contracts](video-integration.md).

A separate identity-publication epoch prevents an account read started before
expiry from restoring cleared rows afterward. Late mutation outcomes and terminal
errors are still delivered; expiry does not silently cancel a remote write.
Disconnect retains its stronger synchronous session invalidation and credential
removal semantics. An invalid installed lease stays classified as account media
until cleared, preventing a brief anonymous-history write on expiry.

Account captions and comments are currently unsupported. The coordinator clears
the guest caption/comment state instead of using a signed account caption URL in
the guest fetcher. Account playback is excluded from anonymous local history,
Save and Follow-current callbacks; profile-scoped private local storage is not
implemented. Existing guest/local features retain their own explicit controls.
These limitations are visible capability boundaries, not a completed account gate.

## Cookies stay origin confined

The existing importer retains only allowlisted `youtube.com`/`www.youtube.com`
cookies. Account HTTP remains fixed to HTTPS www.youtube.com with no redirects.
The supervised extractor receives only a private ephemeral cookie-file **path**;
raw cookie bytes never enter arguments or diagnostics. Helper reaping precedes
file cleanup on success, failure, cancellation and expiry. Existing private-file
crash recovery qualifications still apply; deletion is not forensic erasure.

The inspected installed yt-dlp 2026.08.19
[`YoutubeDL._calc_headers`](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/YoutubeDL.py#L2690)
removes Cookie from `http_headers` and serializes matching scoped cookies into a
separate `cookies` field. Parsing now rejects a nonempty or malformed `cookies`
field on both the common playback response and selected video/audio tracks.
Absent or empty-string fields are accepted. Existing origin/header allowlists
are unchanged: Cookie, Authorization, unknown headers and unsafe origins remain
rejected. This prevents silently dropping a required credential and calling the
result usable.

YouTube session domains do not match googlevideo CDN domains. The implemented native
path therefore uses authorized extraction's signed CDN URLs plus a revocable
lease, while sending no account cookies to CDN hosts. A source requiring separate
CDN cookies remains unsupported. A lease is not permission to broaden headers,
forward credentials across redirects or fall back to direct unguarded media.
Arbitrary CDN403 does not by itself prove session expiry or authorize a retry.

## Validation scope

Synthetic cases cover generation revocation, multiple and late async waiters,
finite expiry, weak lifetime ownership, provider-expiry response generation,
expired retained cookies before helper launch, explicit quality-policy delivery
to a synthetic helper, and common/selected-track cookie-requirement rejection.
Run the focused provider tests with the locked workspace after any coordinated
performance window. Account acceptance still requires an explicitly authorized
local human test; capabilities remain implemented/unverified or unsupported.

On 2026-09-29, `cargo test --locked -p oxplay-youtube` passed all 64 tests,
and `cargo clippy --locked -p oxplay-youtube --all-targets -- -D warnings` passed.
These checks used synthetic fixtures/helpers only and made no account requests.

An earlier intermediate central locked check on 2026-09-29 passed **216 Rust tests** with
three explicitly ignored tests: 68 app, 5 core, 30 media, 29 network plus 3 DNS
helper, 17 storage and 64 provider tests. Workspace formatting and strict
all-target Clippy passed, as did 50 Python tooling tests. The app cases include
selective cancellation, late-read publication rejection, initial-response expiry
admission and pending-position cleanup. The network suite includes synthetic
revocation during delayed responses, redirects, cached reads, expiry and
seek/reopen. Counts describe that source checkpoint; later runs belong in
[progress.md](progress.md).

These are component/coordination checks, not native authenticated YouTube
playback. Human acceptance still needs identity/private reads, separately
authorized writes, authenticated playback and seek/quality/sign-out/expiry,
secret inspection and non-Premium authenticated ad-suppression checks on each
claimed platform. No real account or credential was used in these checks.


## Native regression scope after integration

The rebuilt release `785ed516` passed a 30-second local video/audio/terminal-stop
handoff, a 70-second scoped **guest** refresh and an 85-second **guest**
caption/clear-local lifecycle. They exercise underlying presenter, replacement
and cleanup behavior used by the coordinator; they do not exercise a verified
human account or validate authenticated playback. The isolated clear profile
was inspected read-only: local collections/history were empty, defaults were
restored and no VTT payload remained. No private profile path or row contents
were exported. Logs retain the earlier failed attempts, the wrong caption-fixture
choice, 23 unclassified old-source HTTP read errors during successful refresh,
and an unknown cached replacement audio sample rate. No benign error explanation,
audible-output pass or source commit is invented. See the
[exact summaries and hashes](evidence/2026-09-29-account-integration-summary.json)
and [integration observations](video-integration.md#rebuilt-functional-regressions-at-785ed516).


A later **guest-only** scoped refresh at source checkpoint `287a7c0` passed with
fresh decoder/output rates of 48,000 Hz, advancing audio PTS and 28 recorded
ranges with zero classified errors. Its 108 captured input hashes all match the
committed checkpoint. The earlier cached zero remains unknown notification
state; the earlier 23 errors were not reproduced or explained. This strengthens
component regression evidence without changing the unverified human-account
gate. The later `287a7c0` checkpoint passed **222 Rust tests** and strict workspace
Clippy; it supersedes the earlier 216-test count above. See
[the separate native result](stream-refresh.md#classified-transport-and-fresh-audio-repeat-at-287a7c0).
