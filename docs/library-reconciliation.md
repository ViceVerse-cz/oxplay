# Incremental local-library pages

The implementation keeps a stable Slint model for local collection names and
one for the visible library page. A committed local mutation still triggers an
authoritative SQLite read on the bounded library worker. On the same route and
page, the UI compares stable playlist, video, or channel identities and emits
only the required row additions, removals, and display changes. Unchanged rows
receive no notification. Reordering removes and inserts the affected identity;
it does not promise to preserve that moved delegate's focus.

Each page contains at most 100 rows. The comparison is deliberately bounded
quadratic work over that one page; it does not retain an unbounded catalog or
add a polling loop. A genuine destination/page change may reset the model.
Same-page collection refresh preserves the selected playlist by ID and keeps
its item cursor; a rename no longer resets the selected playlist's videos to
their first page. Explicit page navigation first secures worker admission, so
a full queue cannot attach a new cursor to old displayed content.
An accepted change of playlist or video page clears its old rows while the
new read runs, so a failed read cannot leave another playlist's rows actionable.
A failed collection-page request restores its prior cursor and page identity.
Same-route refresh preserves both rows and pagination until success.

Page requests use one pending ticket with a typed route and navigation epoch.
Both success and error carry that ticket, including database-initialization
failure. A stale or duplicate terminal response cannot release a newer read's
busy state or publish rows/errors for it. The accepted result must also match
the current route and, for playlist videos, the selected playlist ID. Existing
accepted writes, preferences, and Save acknowledgments retain their separate
response semantics; no account requests or catalog models are involved.

The focused model regressions attach an actual Slint `ModelChangeListener`,
rather than checking a second list of events maintained by the implementation.
This is a **test-only** use of the private re-export from the already pinned
Slint revision `cf3b07d4917e6759a63b0c03913a2594ec653414`; see
`api/rs/slint/private_unstable_api.rs` and the listener attachment in
`internal/core/model.rs`. Runtime code uses the public `Model`/`VecModel` API.
Tests cover rename, deletion/unfollow, insertion, a full-page deletion with
backfill, reordering, duplicate/oversized rejection, typed route/ticket checks,
and preservation of selection identity. Worker tests retain real SQLite
pagination and verify correlated initialization failures.
The finite Save diagnostic now requests three real refreshes and checks their
exact acknowledged tickets; unchanged membership no longer needs a fabricated
model notification to prove that SQLite was read. Its previous native pass
predates this diagnostic migration and must be repeated for the new source.

The central `cargo test --workspace --locked` run passed **354 tests**, including
all **157 application tests**, with four explicitly ignored external checks.
This includes the attached-listener, ticket/route, selection, rollback, and real
SQLite regressions above. Formatting and strict workspace/all-target Clippy
passed. The initial lint run found a test-only `let_and_return` in the observer
helper; after its correction all four observer tests passed again.
The first application compile found an unnecessary `Debug` derive on a key
containing `ChannelId`, which does not implement `Debug`; that derive was removed
before the passing run. No native focus/scroll or performance qualification is
claimed. Row notifications describe application model work, not partial GPU
drawing or reduced energy use.

## Playlist name editing

Rename now starts an explicit inline edit prefilled with the selected playlist
name. Enter and Save submit the same captured playlist identity and route epoch;
selection or navigation changes invalidate that identity. Cancel restores the
separate new-playlist draft. Create and Rename retain text after validation or
storage errors, and clear it only after their committed success response. Errors
appear beside the editor. The Save dialog uses the same name validation messages.
Focused identity and UTF-8 boundary regressions pass in the364-test workspace
suite, and the compiled Slint UI passes strict Clippy. Physical keyboard input
and complete native create/rename/cancel validation remain pending.
