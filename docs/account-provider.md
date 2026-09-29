# Account provider research and implementation gate

The current application implements the connection flow: explicit browser-session
export selection, bounded import, remote identity verification, optional protected
storage, re-verification on reconnect, account reads/writes and sign-out. Open
the account page, sign in with your Google account on YouTube in the system
browser, follow the export guide, then select **Connect with session file…**.
The UI does not enable connected capabilities merely because parsing succeeded.
This is a YouTube browser-session connection, not Google OAuth or a password form.

Failed authenticated playback now offers an explicit **Retry from start** through
the same authenticated resolver. It retains the exact failed load, current
video/session and quality policy, requires a valid account lease and rejects stale
results after replacement or sign-out. Retry never switches to guest credentials.
Provider cooldowns and Retry-After delays disable the action until a bounded
one-shot timer allows it; authentication/policy failures require reconnect or a
new selection. Synthetic regressions cover unverified identity, explicit repeated
resolution, cooldown admission, revoked leases and post-sign-out rejection.
Guest and account retry wakeups account for Slint's millisecond clock rounding;
an early callback retains the action and arms one weak timer for the remaining
deadline. Precise monotonic deadlines continue to prevent premature retries.
No live account or real credentials were used for these checks.

Research date: 2026-09-29. The implemented flow and synthetic checks do not
complete real-account acceptance. No browser profile was inspected, no human
credential was imported, and no live authenticated request or remote account
mutation was made.
AC-08, AC-09 and live AC-10 remain unverified. Parsing a cookie file will not
change those statuses.

## Selected direction

Implement the required subset of InnerTube in Rust in the provider boundary,
using explicitly imported YouTube browser-session cookies. Do not run a
persistent JavaScript service. YouTube.js is a protocol reference, not an
application dependency or an assurance that an endpoint remains available.

The reference default branch was resolved with
`git ls-remote --symref https://github.com/LuanRT/YouTube.js.git HEAD` and fetched
for inspection: `main`, `bad89d2657e88f907011655f199fba9fb615c339`. The inspected
reference has an MIT license; retain its notice if substantial implementation
text is copied. Protocol observations below are candidates for local tests,
not evidence that this application can use a real account.

yt-dlp's current guidance says OAuth no longer authenticates its YouTube
extractor and recommends cookies; it also warns of temporary or permanent
account bans. Google desktop OAuth is a different authorization mechanism and
does not supply a browser cookie jar. Do not offer an OAuth-looking connection
button or reuse another application's OAuth client identity. See the
[extractor guidance](https://github.com/yt-dlp/yt-dlp/wiki/Extractors#logging-in-with-oauth)
and [Google desktop OAuth documentation](https://developers.google.com/identity/protocols/oauth2/native-app).

## Observed protocol surface

Requests use the YouTube InnerTube origin and a reviewed client context. The
reference attaches cookie authentication to InnerTube requests, derives an
authorization value from SAPISID, and supplies the account index and optional
delegated channel ID. The Rust adapter must scope those headers to exact
approved HTTPS origins and reject credential-bearing redirects. Do not copy
the reference's default redirect following or raw response error formatting.
See [HTTPClient.ts](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/utils/HTTPClient.ts)
and [Utils.ts](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/utils/Utils.ts).

| Capability | Endpoint relative to `/youtubei/v1/` | Source-observed payload or behavior | Acceptance before enabling |
|---|---|---|---|
| Identity/channel enumeration | `account/accounts_list` | WEB channel-switcher request: `requestType=ACCOUNTS_LIST_REQUEST_TYPE_CHANNEL_SWITCHER`, `callCircumstance=SWITCHING_USERS_FULL`; reference also uses TV for active channel | Parse an actual account identity; offer explicit channel choice; verify selected identity again |
| Account subscriptions | `browse` | `browseId=FEchannels` lists channels; `FEsubscriptions` is the video feed | Bounded pagination and an independently observed subscribed channel |
| Account playlists | `browse` | `browseId=FEplaylist_aggregation`; individual playlist browse ID starts with `VL` | Distinguish owned/editable/saved playlists; verify private reads locally |
| Subscribe/unsubscribe | `subscription/subscribe`, `subscription/unsubscribe` | `channelIds`, endpoint parameters; reference uses different subscribe/unsubscribe parameters | Deliberate user action, response checked and channel state reread |
| Like/unlike | `like/like`, `like/removelike` | Target and available endpoint parameters; interaction reference selects TV context | Deliberate user action and rating reconciliation; WEB compatibility not assumed |
| Save/remove playlist item | `browse/edit_playlist` | `playlistId`, `ACTION_ADD_VIDEO` with `addedVideoId`; `ACTION_REMOVE_VIDEO` with item `setVideoId` | Editable playlist verified; reconcile by playlist item identity, including duplicate video IDs |

The table is grounded in [AccountManager](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/core/managers/AccountManager.ts),
[account endpoint](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/parser/classes/endpoints/GetAccountsListInnertubeEndpoint.ts),
[browse entry points](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/Innertube.ts),
[InteractionManager](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/core/managers/InteractionManager.ts),
[LikeEndpoint](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/parser/classes/endpoints/LikeEndpoint.ts),
[PlaylistManager](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/core/managers/PlaylistManager.ts),
and [PlaylistEditEndpoint](https://github.com/LuanRT/YouTube.js/blob/bad89d2657e88f907011655f199fba9fb615c339/src/parser/classes/endpoints/PlaylistEditEndpoint.ts).

Do not execute arbitrary navigation commands returned by the provider. Allowlist
the implemented endpoint types, validate typed identifiers, bound JSON nesting
and response size, and preserve continuation tokens as opaque session-scoped
data. Unknown renderers are ignored with a sanitized partial-results status;
missing identity is an authentication/unsupported response, never guest data
presented as successful login. Avoid copying the reference's unbounded recursive
playlist continuation search.

## Credential and connection contract to implement

1. Present the account-risk/privacy explanation in the shared Slint dialog.
   Opening YouTube in the system browser is optional. The app never requests
   passwords or authentication codes. Only the user selects a local export;
   no browser scan, extension installation, or implicit import is permitted.
2. Parse a bounded Netscape-format export (4 MiB and 10,000 entries maximum).
   Handle `#HttpOnly_` lines, host-only/domain cookies, secure/path/expiry rules,
   malformed values, duplicate keys, Unicode/domain ambiguity and CR/LF
   injection. Retain only required YouTube-domain cookies; never expand import
   to all Google/browser cookies merely to make authentication succeed.
3. Keep the candidate session in memory until identity is verified. Derive
   request cookies by destination and path rather than constructing one global
   Cookie header for all traffic. On failure, clear candidate credentials and
   show a typed error. Cookie presence is not a Connected state.
4. Store a small random encryption key through an OS vault and keep the jar in
   a versioned authenticated-encryption envelope in the app's private directory.
   Bind profile, format and key ID as authenticated data. Generate a fresh nonce
   per write and use atomic replacement. The storage implementation now uses
   XChaCha20Poly1305 with a fresh random 24-byte nonce, a versioned authenticated
   header and profile binding; see the validation record below. A tested vault
   does not verify a YouTube account or complete the connection UI integration.
5. macOS: Keychain Services; Windows: Credential Manager or a reviewed
   user-bound protection adapter; Linux: Secret Service, with locked/unavailable
   service explicitly handled. If protection fails, offer session-only memory
   use; never silently persist plaintext. macOS now has an independently tested
   Keychain adapter; Linux/Windows return unsupported rather than persisting
   plaintext. Platform references:
   [Apple](https://developer.apple.com/documentation/security/keychain-services),
   [Microsoft](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw),
   [Secret Service](https://specifications.freedesktop.org/secret-service/latest/).
6. Enable identity, subscription reads, playlist reads, subscription writes,
   likes, playlist writes and authenticated playback independently after their
   checks. Preserve account/channel identity across pagination. No background
   write is implied by connection, import, or possession of credentials.
7. Give every operation a cancellation token and session generation. On
   disconnect, advance generation first, cancel authenticated requests/helpers,
   stop authenticated playback, clear account caches, delete vault/envelope
   material and prevent stale results from being applied. Local collections
   remain separate. Disconnect does not revoke the original browser session.

For an explicitly authorized authenticated extractor job, a protected ephemeral
cookie file may be needed. Create it inside a private directory with restrictive
permissions and no symlink traversal; give yt-dlp only the path, never raw cookie
arguments. Reap the whole job before cleanup, bound its lifetime, and remove
stale app-owned temporary directories on next launch without following links.
Document that deletion does not guarantee forensic erasure. Account cookies
must never become global libmpv CDN headers.

## Expiry, writes and ad suppression

Expired session, challenge, throttling and unsupported account configuration
are separate outcomes. Stop retries on authentication/challenge failure and
offer reimport. A mutation timeout has an unknown outcome: reread remote state
before retrying, especially playlist addition. Controls remain pending or roll
back honestly; HTTP 200 alone is insufficient if the action response reports an
error. A read-only connection test grants no permission to test writes.

Use native resolved content playback and exclude known promoted/ad renderers;
do not load YouTube's advertising player or manufacture impression acknowledgments.
Guest and non-Premium authenticated tests must be distinct. Neither the endpoint
table nor a single ad-free clip proves universal suppression. Unidentifiable or
inseparable advertising remains an unsupported delivery path. The
[YouTube API developer policies](https://developers.google.com/youtube/terms/developer-policies)
restrict advertisement blocking in official API clients. This project has no
official approval; using cookies does not establish a policy exemption. Review
terms and account risks before distribution.

## Next concrete slice and human gate

### Storage slice validation, 2026-09-29

`crates/storage::vault` implements opaque protected session persistence with a
4 MiB bound. It accesses only the selected random application-local profile,
never scans browsers, and never stores credentials in SQLite. Local Keychain
items exclude iCloud synchronization. A private directory lock serializes
operations; reads reject unsafe ownership/permissions, symlinks and hardlinks.
Ciphertext replacement is atomic, and in-memory key/session byte buffers zeroize
on drop. Keychain denial is an explicit error with no plaintext fallback.
Deletion attempts removal of the key before ciphertext and reports any denial.
Account generation invalidation, request cancellation and authenticated playback
shutdown must still be coordinated by the account controller.

Executed `cargo test --locked -p serein-storage`: 17 tests passed; the actual
Keychain integration test is ignored in routine runs. Executed
`cargo clippy --locked -p serein-storage --all-targets -- -D warnings`: passed.
Then explicitly ran
`cargo test --locked -p serein-storage vault::tests::macos_keychain_synthetic_roundtrip -- --ignored --exact`:
passed. That local test created one random application-owned **synthetic** key,
encrypted and loaded synthetic bytes, deleted the item and verified its absence.
It refused any pre-existing record and used an unwind cleanup guard. No real
cookie, account, browser profile or human secret was accessed. Signed/package
Keychain behavior and other operating systems remain unqualified.

### Account adapter and worker integration

`crates/youtube/src/account` now implements bounded Netscape import with
zeroizing secret values, fixed-origin authenticated transport, selected-identity
verification, paginated subscription/playlist reads, and explicit subscription,
rating and playlist mutations. A mutation sends at most one write request;
uncertain outcomes require a separate reconciliation action. Playlist checks
preserve exact item IDs and reject incomplete snapshots. Delegated-channel
switching remains unsupported. Provider fixture tests use synthetic credentials
and responses; these do not establish compatibility with a real account.

`crates/app/src/account.rs` connects these APIs to a bounded, sleeping worker.
Only a user-selected regular file is opened and bounded to 4 MiB; no browser
profile discovery occurs. Import and reconnect invalidate the previous session
before work begins. Disconnect synchronously invalidates generation and cancels
in-flight operations, discards pending commands/results, then deletes saved
material on the worker. A queued disconnect is retained during application
shutdown. UI results contain identity/capability metadata and typed statuses,
never cookies or private provider JSON.

Remembering is explicit and occurs only after identity verification. A private
0600 marker contains only the random vault reference and Google account slot.
It is committed before Keychain allocation so interrupted writes retain a
cleanup reference. A private directory lock prevents multiple app instances
from modifying the same saved profile concurrently. Startup inspection never
loads secrets or reconnects; an explicit reconnect decrypts, reparses and
re-verifies identity. Save failure is reported as session-only rather than
plaintext persistence. Windows file import currently fails closed pending a
reviewed native file-handle path; the protected vault remains macOS-only.

Startup inspection also asks the provider to remove stale, narrowly identified
app-owned extractor jars without reading their contents. Explicit authenticated
extraction uses a private temporary jar, checks generation while the supervised
helper runs, and cleans up after the process tree is reaped. This path currently
requires account slot zero and exactly one returned identity; other
configurations fail closed. Normal public playback remains guest by default.
The shared UI now offers an explicit Play with account action and coordinates
revocable native playback, source-preserving quality/expiry replacement, and
sign-out/expiry cleanup. A publication epoch rejects reads completed after the UI
identity was cleared without losing mutation outcomes. Guest playback never
escalates automatically. Account captions/comments and private local collections
remain unsupported. See [the implemented media boundary](account-media.md) for
the actual handoff, cancellation semantics and synthetic-only validation.

The central locked workspace test run on 2026-09-29 passed 81 tests with one
opt-in Keychain test ignored, including eight account-worker regression tests
and three local-library worker tests. Workspace clippy with `-D warnings`
passed. Worker checks cover bounded files, malformed imports without creating
an account transport, private markers, concurrent credential-store exclusion,
cancellation, priority sign-out and stale result rejection. They do not verify
real account connectivity or provider compatibility. This is an earlier checkpoint, not the count for the current media integration;
current session evidence is in
`docs/progress.md`. Human account acceptance remains outstanding. Capture
synthetic/sanitized parser fixtures, never real session material or full private
responses.

An authorized local human test must cover: import failure and expiry; displayed
identity and channel choice; real subscriptions/private playlists; separate
consent for each subscribe/like/save and inverse action; remote reconciliation;
sign-out during each pending operation and authenticated playback; restart
without account data reappearance; secret scans; vault denial; helper process
and temp-file cleanup. No credentials should be sent through chat or public CI.
