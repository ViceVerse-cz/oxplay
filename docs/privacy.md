# Privacy and account boundaries

This project is an experimental implementation of [SPEC.md](../SPEC.md).
Current capability evidence lives in [progress.md](progress.md). Account
connection/playback are implemented but have no real-account acceptance result.
Comprehensive proxy protection is not established. Requirements still lacking
end-to-end evidence must not be advertised as verified capabilities.

The app is intended to minimize its own collection, not make direct YouTube
requests anonymous. YouTube and media/CDN operators can observe network access,
including IP addresses. Connected account activity can be associated with the
account. Disabling optional local history does not mean YouTube receives no
viewing-related information. The operating system and current user account are
trusted; a compromised OS or media engine is outside this protection boundary.

## Defaults and network use

The product starts in guest mode. Telemetry, automatic crash uploads, local
watch/search history, autoplay-next, hover video previews, automatic update
checks and background subscription refresh are off initially. No app account,
cloud synchronization, remote fonts or project-owned analytics service is
required. Offline launch must show the shell/local library without network
requests; this remains an acceptance test rather than an inferred guarantee.

User-requested search, metadata, thumbnails, subtitles and playback cause
requests to their providers. **Search suggestions from YouTube** are on by
default at the product owner's request. While the search field is focused, typed
text (not URL-like input) is sent to `suggestqueries-clients6.youtube.com` after
a short pause, without cookies. The Settings toggle turns this off; local search
history then still works. See [search suggestions](search-suggestions.md). yt-dlp and libmpv make requests independently of a
Rust HTTP client. Helpers and JavaScript challenge runtimes belong to the
application's measured process tree. The project does not claim strict proxy
coverage until egress, redirects, DNS, helper behavior and failures are tested
together. Do not interpret an HTTP proxy setting as anonymity.

Typed video, channel, playlist, media and caption addresses now share a 16 KiB
input/canonical-URL limit and reject raw control characters before parsing.
Watch links with more than one decoded `v` parameter are rejected as ambiguous,
including duplicate identical IDs. Standard URL canonicalization still applies
(for example, the default HTTPS port); this is not a promise to reject every
noncanonical spelling. Address validation does not replace transport-level DNS,
redirect or credential policy. The generated synthetic URL regression corpus is
authored but has not yet run during the reserved native measurement.

Public content should use guest credentials even when account features are
connected, where that playback path allows it. Authenticated playback requires
a clear user action; no silent escalation. No cookies may be sent to public
metadata services or arbitrary media hosts. Do not submit extra watch-reporting
or ad beacons by default; that does not guarantee account history is unaffected.

## Data and credentials

Local subscriptions/playlists are distinct from YouTube account collections.
Connecting must not upload them. Submitted plain-text searches are stored only with the local-history opt-in.
Otherwise they are kept in memory for the session. They share its retention,
and disabling history, Clear history and Clear local data delete them.
Local-history storage APIs require opt-in and provide retention and deletion;
complete UI controls remain subject to the implementation progress report. Metadata and thumbnail caches
can reveal interests even when history is off, so bounded caches and clear-local-
data controls are required. Signed media URLs are transient secrets and must not
be kept in ordinary storage or logs.

The account adapter uses an explicitly selected cookie export and verifies
identity before enabling account operations. Its real-account acceptance tests
have not been run. Never provide a password, authentication code or cookie file through
chat, a bug report or public CI.

On macOS the user may instead pick one installed browser profile and let Serein
import that profile's YouTube session directly ("Sign in with your browser").
This is an explicit, consented choice, not a browser scan. When the account page
opens (never at launch) Serein lists installed browsers by checking for their
profile folders and non-secret profile metadata (`Local State` profile names,
Firefox `profiles.ini`) and, for Safari, `/Applications/Safari.app`; no cookie
database, Keychain item or Safari container is touched until you press **Sign in
with <Browser>**. After the same risk-consent checkbox it reads ONLY
cookies whose host is a YouTube or Google sign-in domain
(`youtube.com`/`.youtube.com`/`www.youtube.com`,
`google.com`/`.google.com`/`www.google.com`/`accounts.google.com`), filtered
before any decryption; only the YouTube session cookies are retained, exactly as
with file import. It reads Chrome/Brave/Edge/Arc/Chromium/Vivaldi
(`~/Library/Application Support/<vendor>/<Profile>/Cookies`, decrypting values
with a key from the browser's `<Vendor> Safe Storage` Keychain password),
Firefox (`cookies.sqlite`, copied with its `-wal` to a private temp directory
before opening) and Safari (`Cookies.binarycookies`, which needs Full Disk
Access). macOS shows its own prompt for Keychain access; a denied Keychain or
missing Full Disk Access is reported with guidance, never bypassed. The Keychain
password, derived key and decrypted values are held in zeroizing buffers and are
never logged or persisted in plaintext. Because the running browser locks its
SQLite database, the selected profile's cookie database (Chromium: values still
encrypted; Firefox: plaintext) and its `-wal`/`-shm` files are copied into a
private (`0700`) per-user temporary directory for the duration of the read and
deleted immediately afterwards; the copy is unlinked, not securely overwritten.
Only the filtered rows are ever read from it. Other platforms report the
feature as not supported. What is then stored is identical to the file path:
nothing unless the session is explicitly remembered.

Imported material is filtered and kept in memory by default. Explicitly remembered
sessions use a macOS Keychain key plus authenticated-encrypted private files;
other platforms currently fail closed for protected persistence. If protection is unavailable, only an
explicit session-only mode is acceptable. See
[the account implementation contract](account-provider.md).

The implemented disconnect flow invalidates account work synchronously, stops
installed account playback, clears transient private media/account metadata and
queues protected-credential deletion. A deletion failure remains visible and
requires retry; no forensic-erasure guarantee is made. Known lease expiry also
stops paused playback and prevents late reads from restoring cleared account
rows. Mutation outcomes remain observable. Account-derived video metadata is
excluded from anonymous local history and Save/Follow-current actions pending
profile-scoped private storage. Account captions/comments cannot silently use
the guest fetchers. These paths have synthetic/component tests; no real account
was used. See [account media](account-media.md). It does not sign
the browser out of Google everywhere or guarantee forensic deletion from SSDs,
swap or memory. Local-only collections have their own retention choice.

## Diagnostics and disclosures

Routine diagnostics must contain sanitized operational categories, versions and
timing, not cookies, tokens, signed URLs, names, titles or search queries.
Diagnostic export must be user initiated and previewable before sharing. Do not
publish raw packet captures, provider responses or memory dumps as normal bug
attachments. Report suspected leaks according to [SECURITY.md](../SECURITY.md).

This is an independent, unofficial client. YouTube compatibility can change;
account use can result in restrictions or bans according to
[yt-dlp's authentication guidance](https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies).
Supported advertising suppression is required but universal suppression is not
claimed. Creator-embedded sponsorships are a separate matter. Platform terms,
branding and distribution obligations require review; open source and cookie
import do not imply approval from Google or YouTube.

Guest video artwork may be retained within the configurable local disk limit
(Off/32/128/256 MiB; default 256). These images can reveal browsing interests
even when history is disabled. Home reads them without HTTP fallback. Clear
local data includes artwork and waits for acknowledged cleanup; account artwork
is excluded from this cache. See [cache ownership and limitations](artwork-cache.md).

Public comments now load their first bounded page after a selected guest video,
as explicitly requested. The persisted Settings toggle disables/cancels/hides
them. Launching, local playback and authenticated/private playback never start
this anonymous comment request. Loading comments does not disable unrelated
controls. See [comments policy and bounds](comments.md).
