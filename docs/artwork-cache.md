# Local guest artwork

Home can display artwork previously encountered during explicit guest browsing.
Its source is a typed cache-only lookup by video ID: a missing or damaged image
never becomes an HTTP request. Guest video catalog rows may populate the cache;
channel/playlist artwork and account-derived data are not persisted here.
Fixtures are excluded from ordinary cache population. The explicit offline Home
diagnostic seeds labeled synthetic artwork only inside its fresh private profile.

Settings offers Off, 32, 128 and 256 MiB; 256 MiB is the initial disk policy.
SQLite schema v6 persists the choice, validates it on both write and read, and
preserves existing library contents. Off deletes stored artwork. Bootstrap is
unconfigured until the persisted preference arrives; it does not briefly open
an Off cache and destroy the previous session's files. Failed configuration
leaves guest browsing available and reports a static error.

The profile-local namespace is `artwork-youtube-guest-default-locale-v1`:
YouTube provider, guest mode, extractor-default locale and first cache policy.
No personalized cache is shared with it. This namespace must change if provider
locale or relevant policy changes. Files contain normalized PNG artwork keyed
by validated video IDs, with no URLs, credentials or raw provider responses.
Artwork can still reveal browsing interests even when optional history is off;
the Settings copy discloses this.

Images are decoded/resized/encoded on the existing thumbnail worker, never on
the UI thread. PNGs are at most 320×180 and 512 KiB each. There are at most 4,096
cache entries. The selected cap reserves space for metadata and atomic staging;
regular files are charged by the larger of logical length and allocated blocks.
Eviction uses in-memory last use, with last-write ordering after reopen. Directory
metadata allocation is filesystem-dependent; these bounds are implementation
limits, not measured resource acceptance. Visible Slint references remain
separately bounded by the existing viewport window. Queues retain at most forty
requests, four active image tasks and eight ready images.

Unix cache operations retain a private directory descriptor, reject symlinks and
hardlinks, and use a nonblocking exclusive instance lock. Atomic replacement and
bounded PNG structure/CRC checks protect stored files; the image decoder still
validates compressed image data. Unsupported platforms fail closed for disk
caching. An unavailable cache does not silently enable a different persistence
path. Existing unsafe directories are retained and reported rather than claimed
cleared.

Clear local data blocks new image admission and invalidates pending publication.
The caption lifetime barrier completes first, then the thumbnail worker aborts
and reaps its tasks and acknowledges disk cleanup, then SQLite commits deletion.
Configuration waits until the artwork barrier ends. Failed or partial deletion
is reported; an unfinished purge cannot release the barrier. Cached public rows
are retired on successful deletion so old visible results cannot immediately
repopulate the cleared cache. The YouTube account is unchanged.

Home reconciliation retains artwork by video ID when a row survives. Reindexing
invalidates old thumbnail generations before changing model membership. Each
completion must also match the current row's typed video identity. Unchanged
Home refreshes preserve model/image identity; one completion updates one flat
row and its corresponding virtual group slot.

## Validation

Local locked workspace validation passed 411 Rust tests (four explicitly ignored
external integrations), all 163 Python tooling tests, formatting and strict
workspace/all-target Clippy. Tests cover migration/defaults/reopen, invalid
limits, disk eviction/Off, corruption, private paths/links, cache-only misses,
unconfigured startup, purge/readmission and image identity preservation.

The debug native Home exercise passed all 12 stages on macOS 27.0 / Apple M1 and
exited 0. It seeded 207 synthetic images before UI startup, released that cache,
then used the production worker's reopen/hydration/cache-only path. This proves
reopen, not a separate-process cached-library restart. Unchanged Home refreshes
emitted no catalog row/reset notifications. The actual clear callback completed;
post-exit read-only inspection found schema 6, zero saved items/collections and
only `.lock` in the artwork directory. The 1000×720 dark screenshot was inspected.
Evidence is under ignored `artifacts/artwork-debug-v1`. An initial post-exit
inspection query used an incorrect table name; the corrected read-only query
completed successfully. Initial test failures in fixture temporary-directory
canonicalization, a test borrow lifetime and expected resized image dimensions
were corrected without relaxing production path checks.

The locked release build passed the same twelve native stages at 760×600 with
the light theme and exited 0; its screenshot was inspected. Post-exit inspection
again found schema 6, no saved entries and only the empty lock file. Evidence is
in `artifacts/artwork-release-v1`; executable SHA256:
`f033e08343782a44129836ebe167ff89204f425349f6c89eb5c2e3921999c1f5`.
The 1000×850 light Settings screen was also inspected after an ordinary debug
launch and clean exit (`artifacts/artwork-settings-v1`). All owned native and
display-wake processes were reaped. Remote CI is pending. No CPU/RAM/usage benchmark was run;
resource gates remain unqualified. No real account credentials were used.
