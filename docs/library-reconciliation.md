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
suite, and the compiled Slint UI passes strict Clippy. Physical keyboard delivery remains unqualified. The later native injected-key
validation is recorded below.

## Keyboard name editing and confirmations

The editor remains mounted and temporarily read-only while its write and
follow-up reads are pending. Enter and the Create/Save action focus that editor
before submitting; committed results do not schedule a later focus steal. Rename
selects the existing name for replacement. Escape cancels an unsubmitted rename
and restores the independent Create draft. Escape during an accepted write does
not pretend to cancel SQLite. Invalid input retains its text and adjacent error.

Deletion confirmation explicitly focuses Cancel, scrolls into view, and restores
its originating control/scroll position on Escape or Cancel. Confirm parks focus
on the stable page before controls become busy. Hidden controls are excluded
from keyboard traversal. The playlist dropdown returns to the acknowledged
selection if its request is rejected. The shared `ConfirmedChoice` mirrors later
acknowledged changes even after the underlying ComboBox changes its index; the
same component now serves playback speed, default quality, artwork settings and
history retention. It restores both index and displayed value synchronously:
index-only rollback can coalesce away its change callback and leave a rejected
label to reverse-select the wrong row on the next turn. The two history toggles
use the same acknowledgment pattern for their checked state.
Accepted video-page navigation retires
both the rename authority and its visible editor, avoiding an unusable stale Save
control.

`--library-keyboard-smoke-test` is a finite, explicitly labeled offline check in
a fresh private profile. It dispatches keys through the pinned Slint
`Window::dispatch_event_with_result` API: real widget traversal/text/Enter/Space/
Escape, real SQLite writes, and actual delivered focus/model observations. Route
bootstrap and the deliberate navigation-away use application callbacks. It does
not qualify macOS/Winit key delivery, physical keyboard input, IME or screen
readers. It admits no startup media, account, helper or online inputs; accidental
search/video-selection callbacks record failure instead of starting requests.

The strengthened twelve-stage debug check passed create/rename, invalid names,
cancellation, two independently persisted collections and navigation during an
accepted write. It also verified actual Cancel focus while confirmation was open
and Delete focus after Escape. The 1000×800 dark capture was inspected; only
Cancel retains the active focus outline. The native process exited 0 and was
reaped (`artifacts/keyboard-debug-v3`). Post-exit read-only SQL confirmed exactly
the two expected fixture names/IDs and history remained disabled.

The initial check passed, but a stronger delayed confirmation checkpoint exposed
stale focus on a disabled Delete control (`keyboard-debug-v2`, failed). Moving
focus to the stable page before disabling/hiding the previous control fixed it;
assertions were retained and passed in v3. The final debug check in
`artifacts/keyboard-debug-v4` also passed the selector regression: displayed
index and label match after both Creates; a deliberately rejected keyboard
choice remains rolled back across event-loop turns; a later real selection
reads SQLite and displays the acknowledged original playlist. Its capture was
inspected and post-exit SQL again matched both expected IDs/names. This injects
a rejecting callback, not actual worker queue saturation. The final locked release
also passed all twelve stages at 760×600/light and exited 0; its inspected capture
shows the confirmation in view with Cancel focused. Post-exit read-only SQL again
confirmed both expected playlists and history disabled. Both owned processes were
reaped. Evidence is in `artifacts/keyboard-release-v2`; executable SHA256 is
`11659bbc976c2b6f8fd3c8bc9c9ac24bd50091b76644048dc2cdb9bad96f6dc8`.
Source `fa67cba` is pushed. Its
[CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36594008344) failed before
either runner started; both check annotations reported failed account payments
or a spending-limit issue. There are no runner logs or remote compilation/test
results for this source. The exact-commit source preview includes the new
diagnostic module, shared widgets and these notes; all asset checksums verified.
No tag or release was created.
Tests exposed an unrelated existing DNS-fixture PID publication race: readiness
now requires a complete newline-terminated PID, with process-reaping assertions
unchanged. The integrated workspace suite then passed 414 Rust tests (four
external integrations ignored). No resource/usage benchmark was run.

At that checkpoint, creation beyond the first 100 playlists remained broken:
the summary omitted the new ID and selection fell back to an older playlist.
The bounded-window fix is documented below. Actual queue-saturation rejection and video-page
rename cancellation still need dedicated native interaction cases; the finite
selector check injects a rejecting callback rather than saturating the worker.

Repeat the functional diagnostic with a new absolute profile directory:

```sh
cargo build --workspace --release --locked
./target/release/oxplay --library-keyboard-smoke-test \
  --data-root /absolute/path/new-keyboard-profile --ui-size 760x600 \
  --ui-theme light --snapshot /absolute/path/new-keyboard-profile/keyboard.png
```

It now exits after 68 seconds and returns an error if any stage fails or never completes.
The screenshot is an explicit one-shot capture; normal presentation is unchanged.

### Video-page cancellation and real worker backpressure

The extended keyboard diagnostic retains the earlier twelve stages, then saves
101 explicitly labeled, thumbnail-free fixture videos through the real bounded
worker in batches of at most 20. A subsequent FIFO page read must publish the
exact first 100 identities. Injected Slint keys begin and edit Rename; the actual
Next callback must immediately cancel it and restore the separate Create draft.
The last one-item page and return to the first 100 must retain the original
playlist ID/name and draft. All 22 checkpoints are finite; the mode requires a
fresh absolute data root and excludes unrelated media/network/helper inputs.

A separate worker regression fills both actual 32-slot channels using the
existing wake callback for deterministic synchronization. The next write must
be rejected. Dropping the worker without consuming its results must unblock
shutdown, join the thread and preserve all 64 accepted writes; reopening SQLite
must find no rejected write. It adds no production hook, polling or sleep. This
is actual worker backpressure coverage, not a native UI queue-saturation check;
the native dropdown rejection stage still uses an explicitly rejecting callback.

The extended native check passed all 22 stages in both debug (1000×800/dark) and
release (760×600/light) on macOS 27 / Apple M1. Read-only post-exit SQL confirmed
two playlists, exactly 101 videos in the original playlist, its unchanged name
and history disabled. Both applications exited zero; the harness confirmed its
owned process groups were gone. Captures of the earlier confirmation checkpoint
were inspected, not treated as visual evidence of the later pagination stages.
Artifacts are `artifacts/keyboard-page-debug-v1` and
`artifacts/keyboard-page-release-v1`. Release executable SHA256:
`3ab759dd848c2630aabf5e969ee741aa535e91400bb306c2b1e878c552eb1f0e`.
The complete local suite passed 448 Rust tests (four explicit integrations
ignored), 163 Python tooling tests, formatting, strict Clippy and locked
debug/release builds. No performance/usage measurements or real-account tests
were performed.
The [public functional manifest](evidence/2026-09-29-library-page-functional.json)
retains exact source/binary hashes, post-exit results and complete native logs.

## Created playlist windows beyond the first page

The worker now acknowledges Create with its committed typed playlist ID. A later
read failure cannot turn a committed write into an apparent failed creation.
The UI requests a bounded window ending at that ID only while the originating
playlist editor context remains current. The preferred ID is correlated with
that read ticket and checked again before publication. Leaving during either
operation keeps the acknowledged window/selection and does not request focus or
navigate back. The current window is refreshed to expose any newly available
Next page without selecting an off-page creation.

Collection navigation uses a constant-size window state instead of a stack of
previously visited cursors. Storage returns at most 100 ascending rows, with typed
forward/backward requests computed in the same SQLite read snapshot. A backward
request includes its opaque boundary, so an empty trailing page can return to
its surviving boundary row. Empty collection windows retain navigation controls.
The existing forward-only storage API remains available to exports and other
callers; content/history pagination is unchanged.

A revealed page gets a canonical refresh request based on its lower boundary,
not the created ID. Rename and deletion can therefore refresh that window even
when the originally revealed playlist no longer exists. These are live keyset
windows: inserts/deletes can change the number of rows or backfill a refreshed
page. No catalog-sized rank scan, page-offset table or fabricated cursor history
is needed. A change of selected collection retires the previous video's rows
before its new read, so a failed read cannot leave old rows actionable under the
new identity.

Storage and worker regressions cover creation beyond 100, bounded bidirectional
traversal, deleted boundaries, empty pages, correlated failures and canonical
refresh after rename/deletion. The complete workspace suite passed 428 Rust tests
(four explicitly ignored integrations). The debug 14-stage native diagnostic
passed bounded 206-collection navigation, same-ID rename, deletion and an accepted
creation while Settings/search retained focus. Post-exit read-only SQL confirmed
206 playlists, created 207 present, deleted 206 absent and history off. The labeled
1000×800 dark capture was inspected (`artifacts/collections-debug-v1`). This uses
real worker/application callbacks, not physical pointer/keyboard interaction or
resource measurement.

The final locked release passed the same fourteen stages at 760×600 in the light
theme (`artifacts/collections-release-v1`). Its capture was inspected, exit was 0,
and owned processes were reaped. Read-only SQL again confirmed 206 final
playlists, created 207 present, deleted 206 absent and history disabled. Release
executable SHA256:
`784dae0bed05a158a386b827866dac7c3c41c21bc3fb615ff04f9b35cc1add1e`.
The final diagnostic reports zero remote thumbnail starts and media loads; it
is not a whole-process egress audit. Hosted checks are tracked in [CI notes](ci-release.md).
