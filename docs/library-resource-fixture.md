# Offline large-library and raster fixture

This diagnostic is implemented for SPEC sections 11 and 12; its finite native page/hover/keyboard sequence passed in debug and release. It is not a passed resource gate. Earlier `--demo-related` rows have empty thumbnails and cannot stand in for this workload.

Preparation is a separate, explicit developer operation, outside both the UI thread and measurement windows:

```sh
python3 scripts/prepare_library_resource_fixture.py /absolute/new-fixture-root /absolute/local-1080p60.mp4
```

The destination leaf must not exist. The preparer creates a private SQLite profile with one conspicuously labeled synthetic collection and 10,000 synthetic items, using the application's versioned schemas. It samples thirty distinct compressed PNG stills from the selected local clip through FFmpeg. The project's existing testsrc2 clip is suitable: these are decoded raster test-pattern images, not icons, real provider metadata, or a claim to real catalog results. Fixture preparation uses local files only. It does not resolve videos, connect an account, discover browser profiles, or read an ordinary application profile.

The final fixed-format marker records the expected item count and exactly thirty known image names and byte lengths. A separate provenance file records local clip, FFmpeg, schema, initial database and image SHA256 hashes without exporting their private filesystem paths. The marker is written only after successful database/image preparation. Interrupted preparation leaves an unadmitted directory; the preparer never recursively deletes it or overwrites an existing destination.

Admission accepts only an explicitly selected absolute private directory, the fixed bounded marker, thirty private regular images (each at most 2 MiB compressed), and the expected private profile/database. Image reads use a retained directory descriptor, fixed basenames, `openat`, `O_NOFOLLOW`, `O_NONBLOCK` and `O_CLOEXEC`; symbolic links, hard-linked files, FIFO/socket entries, oversized files and out-of-range indices fail closed. The fixture image capability is a separate typed source. Provider URLs do not gain local-file support. The recorded hashes are provenance for the external harness; admission validates the file set and lengths rather than claiming cryptographic revalidation.

The application route uses the ordinary library worker's acknowledged 100-row pages and the existing grouped, virtualized Slint feed. It keeps at most one displayed page (100 rows, below the model's 120-row ceiling), never 10,000 UI rows or components. Thumbnail work retains its four-job limit, eight-result queue, forty-row viewport cap and bounded image decoder (4096-pixel dimensions, 16 MiB decode allocation, resize to at most 320×180 before UI publication). Scrolling drops stale model references and cancels superseded work. The diagnostic must distinguish a stale result discarded at handoff from a successful image upload.

The finite interaction variant visits the first three real database pages and return through the previous-page callbacks. It compares exact acknowledged item identities, settles pending/ready thumbnail work at each page checkpoint, then asserts zero unrelated flat/grouped/local model notifications and zero thumbnail work during pointer-only phases. Additional finite stages through 42 seconds exercise actual offscreen keyboard focus, Tab exit/reentry, column-changing resize identity and cancellation of a delayed focus handoff; see [keyboard scope](feed-keyboard.md). These interaction stages are not resource measurements. The fixture-only variant has no recurring diagnostic timer. Externally sampled settled/minimized resource qualification remains separate from the finite interaction sequence.

The readiness instrumentation emits one `library fixture quiescent`
record per admitted generation only after the worker has drained and every
near-viewport row has a ready image. The record includes the separate intersecting
image count, model/queue counters and hidden state. It neither starts a timer nor
requests a redraw. A fixture-only completion wake lets the UI observe the last
worker's retirement without polling. Its deterministic state tests pass; native readiness evidence remains pending.

The sampler's `--library-fixture-ready` option waits for that bounded
record **before** its ordinary ten-second warm-up. It fails after thirty seconds,
an early process exit, malformed counters or oversized logs. It reads only new
log bytes during admission and does no readiness polling during sampling. Keep
the application's finite lifetime long enough for admission plus warm-up and
sampling; the sampler still fails if the app exits early. Readiness is a point
observation: the complete log and final counters must also establish unchanged
workload and the intended visible/minimized state. No marker establishes GPU
completion or physical compositor visibility. The sampler regression tests pass. Actual native admission and a subsequent
resource measurement still remain pending.

Fixture-only event tracing records elapsed time and fixed window/input/library
event kinds, with current model counters, up to 256 events. It excludes cursor
motion, typed text, paths, URLs and provider data and creates no timer. This is
intended to diagnose an unexpected restore/reload; it is not an input-origin
audit or a request/response identity protocol. Publication traces are emitted
before publication, so their counters describe the preceding state. A capped
trace explicitly cannot establish later chronology. This instrumentation is also
authored but unrun. Normal application mode does not emit these events.

## Counts and limits that must remain distinct

- A compressed fixture file is not a decoded thumbnail.
- A decoded result is not proof of Slint upload or retained GPU storage.
- A model image reference is not proof that its thumbnail rectangle is visible.
- The viewport's overscan range includes rows outside the clip; it cannot supply the visible-thumbnail count.
- App model-change counters do not measure Slint hit testing, bindings, texture-cache eviction, GPU drawing or compositor presentation.

The measured responsive feed allowed at most four columns with large thumbnail cards. A source-authored six-column extension is described below; it has no larger-display runtime validation yet. On the available reference display, about thirty thumbnails may not fit visibly in the actual window. Native evidence must record the actual positive-intersection image count, window/display geometry and scale. Thirty fixture files, thirty cumulative decodes, or thirty cached/overscan references must not be presented as passing SPEC's approximately thirty-visible-thumbnail settled-window gate. That gate remains open if the supported native geometry cannot display the required workload.

Image CPU byte estimates should count distinct retained image allocations once, while separately reporting transient decoder/result buffers. Slint GPU texture retention and unified-memory overlap are currently unmeasured; do not add a guessed GPU allocation to RSS or claim it is zero. The fixture's 10,000-row database proves neither rendering all rows nor instantiating 10,000 components; bounded page/cache instrumentation and native scrolling evidence are required.

After the exclusive soak, the preparer generated `artifacts/library-resource-fixture-v1`. Independent read-only inspection verified schema 4, one collection with 10,000 items, empty history/subscriptions, all five privacy flags off, and default 1080p/1×/100% playback preferences. All thirty 640×360 PNGs have distinct hashes matching their recorded lengths/provenance (1,525,980 compressed bytes total), single hard links and mode `0600`; both directories are `0700`, and the database/marker/provenance files are `0600`.

The first native run of checkpoint `0f311dd` passed all five page/hover checkpoints: 100 acknowledged rows per page, six intersecting thumbnails and nine near-viewport images, zero input-only model changes, zero media loads and no connected account. It failed the later keyboard deferred-handoff assertion at 32 seconds; see [keyboard failure and correction](feed-keyboard.md). That first complete diagnostic failed; the corrected repeat below passes its functional assertions, while resource acceptance remains open. Existing empty-thumbnail measurements are not relabeled.

At `ca21de92a94abf209b76576c8b584751d3d20b4f`, the complete corrected diagnostic
passed in debug (44.825 seconds) and release (43.887 seconds), both exit zero.
It traversed acknowledged pages 1→2→3→2→1 with 100 rows each, then passed the
offscreen focus, rapid pending navigation, Tab boundary, column-resize identity
and search-cancellation checks. Every settled page checkpoint had six intersecting
ready thumbnails, nine near-viewport images and zero input-only model changes.
The final release counters retained 100 model rows, zero remote thumbnail starts,
zero media loads and no connected account. Later keyboard scrolling leaves
thumbnail work active at diagnostic shutdown (three pending, one inflight and
two ready in this run); that final state is not a settled resource sample.

[Complete logs, metadata and frozen source inventories](evidence/2026-09-29-raster-corrected-summary.json)
retain exact binary hashes and the earlier failure. Actual visible workload is
six thumbnails at the page checkpoints, not thirty. No idle CPU/RSS, GPU texture
inventory, physical visibility, screen-reader or other-platform gate is passed
by these finite interactions. The previously inspected raster screenshot belongs
to the original `0f311dd` capture, not this new release.

## Bounded wider-feed change — unit tests passed, native validation pending

The shared feed, grouped model and keyboard navigation now coordinate a maximum
of six columns. The existing 270px column breakpoint and 20px gutters are
unchanged; this does not reduce the previous minimum card width. Narrow/current
four-column geometry remains unchanged. The ListView still virtualizes row
components, while the flat model remains capped at 120 records and actual local
pages at 100. Column transitions regroup once; thumbnail completion still
updates one child row and preserves its model identity.

Source review found a concrete overscan starvation case: at six columns, a
viewport beginning in a preceding card's text band can have 12 admitted images
before 30 actually visible thumbnail rectangles. The former first-40 truncation
then dropped the last two visible images. The bounded admission helper now
trims only leading overscan when an actual visible interval of at most 40 images
would otherwise be truncated. The admitted image range remains capped at 40, with four concurrent jobs and
eight queued results. Decoded dimensions and cache budgets are unchanged. Both thumbnail
intersection boundaries trigger reevaluation, including a text-band crossing
without a whole-row scroll; unchanged admitted ranges return without model or
worker notifications. Related-list admission keeps its prior behavior.

Passing deterministic regressions cover admission of all 30 visible images with six columns, text-only
leading rows, partial final groups, unchanged child identity on thumbnail
updates/append, and keyboard row/page/Tab boundaries through six columns.
Existing finite native checks still use their original small-window geometry;
they do not establish six-column runtime behavior. More than 40 simultaneous
visible images still exceeds the existing population budget and cannot count
as a fully populated resource fixture. No thirty-visible, idle CPU, memory,
screen-reader or larger-display gate is claimed by this source change.
