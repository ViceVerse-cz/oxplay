# Local Home

Home now has a distinct local route. It opens at startup without a provider
request and lists videos saved to local playlists, newest surviving playlist
membership first. A video saved in several playlists appears once. Re-saving
an existing membership updates its metadata without moving it to the top.
History and account collections do not populate this feed. Playing a saved item
uses the normal guest resolver; connecting an account does not escalate it to
authenticated playback.

The page reads at most 100 videos and one continuation sentinel on the existing
SQLite worker. Next/Previous replace the bounded page; at most 1,024 previous
cursors are retained. Schema v5 adds an index on video identity and membership
ID, preserving existing library data. Cursors address live data, not a snapshot:
committed collection mutations invalidate navigation cursors and refresh the
first page when Home is visible. An operation still using the shared busy state
defers that refresh until its terminal event. Refresh and Home return to the
first page. Failed reads show an error and preserve acknowledged rows.

All controls and cards remain in the shared compiled Slint UI. The existing
catalog and row-group models retain their identities. A same-page refresh
reconciles video IDs, notifying only changed rows; a real route/page change
may reset the dataset. Focus is parked before affected virtual cards retire and
restored by video ID. Home clears public navigation and advances thumbnail-work
generation before replacing the catalog, preventing old image completions from
painting local rows. Local saved summaries do not contain thumbnail URLs, so
these cards show the ordinary unavailable-image treatment instead of fetching
remote images at startup. Cached local artwork is not yet implemented.

Home reads use their own ticket namespace, separate from library writes and
collection pages. Navigation, new provider work and local-data clearing retire
those tickets. Successful clear drops Home-derived rows even when they are the
watch page's related list. No local SQL, filesystem or image decoding runs in
Slint callbacks; there is no new production polling timer.

## Validation

Source `42c937e` passed [macOS and Linux CI](https://github.com/ViceVerse-cz/yt/actions/runs/36585092789):
395/377 Rust tests respectively, 163 Python tests on each, formatting, strict
Clippy and locked release builds. CI performs no native window or account test.
The exact-commit source archive includes the new schema migration and Home
modules; its generated checksums were verified locally.

The final integrated workspace suite passed 395 Rust tests, with four explicit
external integrations ignored. Formatting and strict workspace/all-target Clippy
passed, along with all 163 Python tooling tests and the locked release build. Tests cover deduplication across 205 videos/multiple playlists, all three
pages, membership deletion, history exclusion, absence of URLs, schema-v4
migration, independent worker tickets, stale read rejection, deferred refresh
cancellation and actual Slint model-listener notifications. An initial compile
caught a diagnostic callback named `on_select` instead of the actual
`on_select_video`; it was corrected before successful validation.

The debug native run on Apple M1/macOS 27.0 passed all eleven stages, exited
successfully and was reaped. Its 1000×720 dark-window capture was inspected:
fixture labels are visible, cards and navigation fit, and the diagnostic observed
zero remote thumbnail starts and no media loads. Evidence is retained under
ignored `artifacts/home-v1`. This does not establish whole-process egress,
resource budgets, physical keyboard/screen-reader behavior or additional platform
qualification.

The final release also passed all eleven native stages and clean shutdown at
760×600 in the light theme. The inspected capture keeps essential navigation,
two responsive card columns and pagination visible. Evidence is in
`artifacts/home-release-v1`; executable SHA256 is
`bdbb5dd88fd7a2fb774acb4089b3f74d71123bad6d13e6844958ea0952a5affa`.
Both runs reaped their owned native process and display-wake helper.
An ordinary release launch into a fresh empty profile also exited successfully;
its 760×600 light-theme capture shows the local empty state and explicit search
shortcuts without fixture data (`artifacts/home-empty-v1`).

The finite offline native exercise uses a new private profile with explicitly
labeled synthetic saved videos and no thumbnails. It tests real worker mutations
and Home rendering, not the Save dialog or native import picker. It never sends
synthetic video IDs to the extractor. Repeat with:

```sh
cargo build --locked
./target/debug/serein --home-smoke-test \
  --data-root /absolute/path/new-home-profile --ui-size 1000x720 \
  --snapshot /absolute/path/home.png
```

The command has a 40-second watchdog and rejects online/media inputs, other
smoke modes and preexisting profiles. Its fixture preparation occurs before the
UI starts; normal launch does not seed any data. No CPU/RAM/usage benchmark is
part of this diagnostic.
