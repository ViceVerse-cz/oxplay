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
| Home recommendations | `browse` | `browseId=FEwhat_to_watch`; `richGridRenderer` → `richItemRenderer` → `videoRenderer` or `lockupViewModel`; `continuationItemRenderer` token for the next page | Observe a real signed-in feed; confirm ad/Shorts exclusion and continuation shape locally |
| Subscribe/unsubscribe | `subscription/subscribe`, `subscription/unsubscribe` | `channelIds`, endpoint parameters; reference uses different subscribe/unsubscribe parameters | Deliberate user action, response checked and channel state reread |
| Like/dislike/remove rating | `like/like`, `like/dislike`, `like/removelike` | Target and available endpoint parameters; interaction reference selects TV context; `removelike` clears either rating | Deliberate user action and tri-state (like/dislike/none) rating reconciliation; WEB compatibility not assumed |
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
   Keychain adapter. Windows uses a Credential Manager generic credential
   (implemented; only its ignored synthetic roundtrip exists). Linux returns
   unsupported rather than persisting plaintext. Platform references:
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

Executed `cargo test --locked -p oxplay-storage`: 17 tests passed; the actual
Keychain integration test is ignored in routine runs. Executed
`cargo clippy --locked -p oxplay-storage --all-targets -- -D warnings`: passed.
Then explicitly ran
`cargo test --locked -p oxplay-storage vault::tests::macos_keychain_synthetic_roundtrip -- --ignored --exact`:
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

Ratings are tri-state (like, dislike, none). The watch page shows a joined
like/dislike pill with the public like count for everyone; the former explicit
“Check like” step is gone. For a connected identity the app reads the rating
once per video load when the single account slot is idle (a busy slot defers
it to the next idle wake), and each Like/Dislike click submits one reconciled
write: the same button again sends `like/removelike`. The pill shows the
requested state and a ±1 like-count adjustment optimistically while both
buttons are disabled; a definite failure rolls back, and an unconfirmed outcome
shows no rating until explicit reconciliation. YouTube exposes no dislike count.

If verification after a write loses authentication, reconciliation now returns
the terminal identity error. This clears the connected UI and revokes account
playback instead of showing an ordinary pending outcome under a stale identity.
The provider retains the unconfirmed operation until the session is discarded;
it never automatically repeats the write. A generic, memory-only warning survives
expiry/disconnect and explicit reconnect in the running application. It carries
no private mutation identifiers and tells the user to check YouTube before
repeating the action. Reconnect does **not** reconcile a previous session's write:
stable identity-bound recovery across sessions is not implemented. Restarting the
application does not preserve this warning. Synthetic regressions cover both a
successful write and a timeout followed by expiry during verification, revoked
media authority, absent connection/vault export and zero replay.

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
plaintext persistence. On Windows, file import opens the canonical path without
following reparse points and the vault uses Credential Manager; both are
implemented but have no native account test.

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

### Home recommendations (implemented, not qualified)

`AccountClient::recommendations()` reads the signed-in `FEwhat_to_watch` feed
through the same verified session, fixed-origin `browse` transport, generation
checks, cancellation and expiry handling as subscriptions/playlists. Only the
direct item arrays of `richGridRenderer` and continuation actions are read.
Shelves (Shorts, news, posts), the topic-chip bar and nested navigation commands
never become items or cursors. Items flagged by the shared promoted-renderer
check or an ad badge, and Shorts, are omitted. Video IDs, channel IDs and
artwork URLs are validated. Mixes/playlists and unknown shapes set `partial`.
More than one distinct continuation token, an unknown page or more than 200
items is an error. The cursor is bound to this feed and session generation.
Capability reporting adds `recommendations`, marked verified for the session
only after a successful read.

The app submits this read only when a connected user opens Home or selects
Refresh, Next page or the Recommended chip; see [local Home](local-home.md).
Parser, client and worker tests use small synthetic fixtures. **No real account
feed has been observed**: whether the live response matches these shapes,
whether exclusion is complete and whether the feed is personalized as expected
all remain unverified, pending an authorized human test.

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
identity and channel choice; real subscriptions/private playlists; the Home
recommendation feed and its paging, including ad/Shorts exclusion; separate
consent for each subscribe/like/save and inverse action; remote reconciliation;
sign-out during each pending operation and authenticated playback; restart
without account data reappearance; secret scans; vault denial; helper process
and temp-file cleanup. No credentials should be sent through chat or public CI.

## Launch restore of a remembered session (2026-09-30)

When a session was imported with **Remember this session and sign in
automatically**, the next normal launch reads the nonsecret marker and submits
one `Reconnect` for it, without asking for the consent checkbox again: consent
was given when the session was saved. The restore runs at most once per launch,
is never retried after a failure (the saved-session actions remain for a manual
retry), is skipped for every finite diagnostic/smoke/fixture mode, and is
cancelled by Disconnect. If the user is still on the untouched local Home when
identity verification succeeds, Home switches to the account's
recommendations; any other page is left alone. This deliberately relaxes the
earlier "no network work on clean launch" rule only for this opt-in. It is
unit-tested only; a real remembered account was not exercised.

## Sign in with your browser (2026-09-30, macOS)

An optional convenience path lets the user pick one installed browser profile
and have Oxplay import ONLY that profile's YouTube/Google session cookies
directly, instead of exporting a Netscape file by hand. It is the SPEC's
explicitly-consented, narrowly-scoped browser-profile helper — not a browser
scan or an implicit import.

- **Consent and gating.** The same risk-consent checkbox governs both paths. The
  "Sign in with <Browser>" action is enabled only when consent is checked and no
  account operation is busy; the shared **Remember** checkbox still applies (and
  still also means "sign in automatically at launch"). File import remains as an
  "Other options" fallback below the browser block. Detection runs when the
  account page opens (not at launch) and is filesystem metadata only: Chromium
  profile folders that contain a cookie database (names from `Local State`),
  Firefox `profiles.ini` entries whose `cookies.sqlite` exists, and Safari when
  `/Applications/Safari.app` exists — Safari's protected container is not probed,
  so no privacy prompt appears before the user acts. No cookie database, Keychain
  item or network request is touched. A browser with several profiles is offered
  per profile, default profile first; the selected profile id is re-validated
  against fresh discovery before any read.
- **What is read.** Only cookies whose host is
  `youtube.com`/`.youtube.com`/`www.youtube.com` or
  `google.com`/`.google.com`/`www.google.com`/`accounts.google.com` are read,
  restricted by exact host match in the SQL query (Chromium/Firefox) or during
  parsing (Safari, before the value is extracted) before any decryption; other
  Google subdomains such as `mail.google.com` are never read. The Keychain is
  queried only if an allowed row has an encrypted value. The rows are converted to the same in-memory
  `SessionCookies` the Netscape parser produces and run through the identical
  filtering, validation, expiry, de-duplication, identity verification, consent
  gating, protected `Remember` storage and typed error handling as file import.
  As with file import, only the YouTube auth cookies survive that shared filter;
  Google-domain cookies are read but dropped.
- **Supported browsers/formats (macOS).** Chromium family — Google Chrome,
  Brave, Microsoft Edge, Arc, Chromium, Vivaldi — under
  `~/Library/Application Support/<vendor>/<Profile>/[Network/]Cookies`
  (AES-128-CBC, key = `PBKDF2-HMAC-SHA1(<Vendor> Safe Storage password,
  "saltysalt", 1003, 16 bytes)`, IV = 16 spaces, `v10` prefix; at meta
  `version >= 24` a 32-byte SHA-256 host prefix is stripped). Firefox —
  `profiles.ini` → `cookies.sqlite` (plaintext; the DB and its `-wal` are copied
  to a private temp dir before opening because the browser locks it). Safari —
  `~/Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies`
  (binary format; requires Full Disk Access — a permission error tells the user
  how to grant it). Other platforms compile and report "not supported yet".
- **Secrets.** The Keychain password, derived key and decrypted values are held
  in zeroizing buffers. Keychain access uses `security-framework`; macOS shows
  its own access prompt and denial/cancellation is a typed permission error with
  no plaintext fallback. Keychain access is isolated behind a small trait so
  tests inject a synthetic password; the real Keychain is never used in CI.
- **Validation.** Chromium decryption (v10, with and without the 32-byte host
  hash), host filtering before decryption/Keychain (other sites, other Google
  subdomains and look-alike hosts never read), expired-cookie handling, Keychain
  denial, Firefox fixture DBs (including uncheckpointed WAL data while a
  "browser" connection stays open), Safari binarycookies fixtures, profile
  discovery and default-first ordering from a synthetic tree, permission-denied
  and corrupt-source mapping, and app-side consent gating/choice mapping are
  covered by synthetic-only tests. No real browser session, profile, Keychain
  item or human credential was used; the real Keychain prompt, Full Disk Access
  behavior, current browser schema versions and live account acceptance remain
  unverified.
