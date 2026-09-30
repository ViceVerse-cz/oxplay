# Public video details and comments

`ResolvedPlayback.details` contains optional genuine description, upload date, views, likes, reported comment count, subscriber count and chapters. For public guest selections these now come primarily from the native InnerTube `next` watch page (below), requested on its own worker in parallel with the extractor's stream resolve; the already extracted yt-dlp video JSON fills any field the native page lacks and is the whole source when the native read fails. Missing values stay absent. Descriptions are bounded to 20,000 Unicode characters; control characters other than line breaks/tabs are removed. The shared watch page displays the description directly below the video title/actions. Its selectable read-only preview shows up to three lines / 300 characters; Show more expands the actual bounded description inline. The outer watch page scrolls the content, without reconstructing the video host or opening a modal. Already-resolved account description/metadata can be displayed in memory too; disconnect clears it through the existing account lifecycle.

Comments are guest-only reads on the existing supervised catalog worker. At the
user's request, the first bounded page now loads automatically after an accepted
public guest video selection. **Show comments and load the first page automatically**
in Settings persists in SQLite schema 7 and defaults on; disabling it cancels
only the comment request it owns and hides/clears the comment model. Launching,
local playback and account playback do not start anonymous comment requests.
Authenticated/private playback never falls back to guest comments. Existing
profiles receive this preference through the checked migration, without changing
history or other preferences.

A zero-duration single-shot UI handoff defers submission until accepted playback
state is committed. It checks the selected typed video, worker generation,
loaded/remote/guest state and setting again before starting. Replacing the video,
searching, account handoff or disabling comments retires stale work. Generation
cancellation cannot stop a newer foreground request. Comment loading uses its
own request-active state instead of disabling unrelated application controls;
there is no polling timer or new helper stack.

The finite page contains at most 20 inline comments with selectable bodies,
author/date, genuine optional creator badge and compact likes. The Copy action
and reply clutter are removed. Missing avatar data uses the existing attributed
Lucide glyph. Refresh/cancel controls are compact, and pending-only skeletons
have no animation. Dates and counts are presentation-only formatting; absent
provider values remain absent. Explicit Previous/Next replaces the bounded page.
Automatic publication preserves watch scrolling; an explicit load/page may
reveal the comments heading without stealing focus. Account comments/writes
and replies remain unsupported. No credentials are consulted even when an
account is connected.

## Native watch page and comments (InnerTube `next`)

`serein_youtube::watch::watch_page` performs one anonymous `next` request
(`{"videoId", "racyCheckOk", "contentCheckOk"}`) through the shared
`innertube::GuestTransport` (WEB client context, `hl=en`/`gl=US`, no cookies,
bounded 8 MiB response, cancellation, shared 429 cooldown). The response must
name the requested video in `currentVideoEndpoint` before any field is read, and
at least one of `videoPrimaryInfoRenderer` / `videoSecondaryInfoRenderer` must be
present; otherwise it is `MalformedOutput`, never an empty success.

- Title, `dateText` (English `Mon D, YYYY`, optionally prefixed "Premiered" /
  "Streamed live on"; relative texts stay absent), displayed view total (a live
  concurrent-viewer count is not a total), and likes from the segmented like
  button's accessibility text.
- Channel name, typed `ChannelId`, subscriber count and the owner portrait from
  `videoOwnerRenderer`; the portrait passes the same exact-host
  `safe_avatar` policy (HTTPS `yt3.ggpht.com` / `yt3.googleusercontent.com`) and
  is handed to the channel-avatar worker for the same resolved channel only,
  which then skips its separate yt-dlp channel-metadata run.
- Full description from `attributedDescription` (else the
  structured-description panel, else legacy `description.runs`). `commandRuns`
  use UTF-16 offsets; YouTube's own `/redirect?q=` links are restored to their
  real http(s) targets and link chips to their YouTube URLs. Output stays plain
  text, bounded to 20,000 characters with control characters removed.
- Comment count from the comments entry point or the comments engagement panel.
- The first comments continuation from the `comment-item-section` item section
  (as yt-dlp uses it) or the comments engagement panel; a section with only a
  message (comments off) is `Unavailable`.

Counts parse only reviewed English forms (`2,261,131 views`, `4.72M`, `2.8K`);
anything else is absent. Native values replace extractor values field by field
(`WatchPage::merge_details`). If the native page arrives after playback was
published, the same accepted guest item is upgraded in place (description,
chapters) without touching comment rows, cursors or the expanded state.
Unsupported/malformed/server-error classes fall back to the extractor values;
cancellation, rate limiting, offline and timeouts are not repeated through the
helper. Account playback never receives or requests this anonymous page.

Comments use the returned continuation directly: `next` with `continuation`.
Current responses (observed 2026-09-30) are two `reloadContinuationItemsCommand`
endpoints (a `commentsHeaderRenderer`, then 20 `commentThreadRenderer` rows plus
one `continuationItemRenderer`); later pages use `appendContinuationItemsAction`.
Each thread's `commentViewModel.commentViewModel.commentKey` names a
`frameworkUpdates.entityBatchUpdate.mutations` entry whose
`commentEntityPayload` carries `properties` (`commentId`, `content.content`,
`publishedTime`, `replyLevel`), `author` (`displayName`, `channelId`,
`avatarThumbnailUrl`, `isCreator`) and `toolbar` (`likeCountNotliked`,
`likeCountA11y`). Legacy `commentThreadRenderer.comment.commentRenderer` and
bare `commentRenderer` rows are still accepted. Bounds: 8 endpoints, 100 items,
2,000 mutations, one continuation token (printable ASCII, 16 KiB), 20 published
comments per page, 10,000-character bodies, 200 browsable comments with the
existing limit message. Replies (`replyLevel` > 0) are rejected. A thread whose
entity is missing is skipped; a page whose threads all fail is malformed. Comment
IDs already published by earlier pages are dropped, and a page of only repeats
ends paging instead of looping.

Native cursors carry the real continuation token plus the video, session
generation, offset and published IDs; they are **not** prefix replay, so later
pages cost one request and do not re-extract earlier comments. The first page
reuses the watch page's continuation when it has already arrived, saving one
request; otherwise it requests the watch page itself. Only a first page falls
back to the extractor (on the fallback-eligible classes above); a native
continuation never silently turns into a differently ordered replay, and an
extractor replay cursor never reaches the native path. The existing UI
(`comments_ui.rs`, avatars in `comment_avatars.rs`, Previous/Next) is unchanged;
its Previous stack simply holds native cursors.

## Extractor fallback path

The installed yt-dlp `2026.08.19` source was inspected, including [YouTube comment extraction](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_video.py#L2374), `_comment_entries` (root depth starts at **1**), `_get_comments`, and [the common post-extractor](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/common.py#L3882). yt-dlp performs the real guest InnerTube `next` requests and normalizes comment identity, text, author and counts. The adapter requests:

```
--get-comments --no-ignore-errors --ignore-no-formats-error --no-playlist
--extractor-args youtube:comment_sort=top;max_comments=COUNT,COUNT,0,0,1
```

COUNT is the requested prefix plus 20 comments and one lookahead. The five limits bound total comments, parent comments, replies, replies per thread and depth. Normal helpers retain the 45-second deadline, 32 MiB stdout/64 KiB stderr bounds, zero retries, disabled configuration/plugins/cache/remote components and process-group cancellation/reaping. The helper receives no cookies. `comments:null` is unavailable; a missing/mistyped comment field is an unsupported response, never a successful empty page. A genuine empty array is reported explicitly as no public comments returned.

The opaque cursor binds the video, session generation, prefix offset and ordered IDs of every previously published comment. Its immutable prefix contains at most 180 IDs of at most 256 bytes each; cloned navigation cursors share that allocation. At most 200 top-level comments are browsable, with a distinct limit message. This is **prefix replay**, not an exposed InnerTube continuation: each next page re-extracts earlier comments. Any changed published identity or ordering fails with a refresh action, including a previously seen comment moving across an unchanged page boundary. Matching identities do not guarantee a stable text/count snapshot: public comments can be edited. Deep-page latency, helper memory and quota cost are not qualified. A direct Rust continuation adapter could remove replay once separately validated.

## Observed checks

On 2026-09-30, `cargo run --locked -p serein-youtube --example comments_smoke`
(guest, macOS Apple Silicon host, home network; single sequential runs, not a
benchmark) measured the native watch page against yt-dlp. It prints counts,
dates and timings only. Wall-clock milliseconds:

| Video | native `next` | native comments p1 / p2 | yt-dlp resolve | yt-dlp comments p1 / p2 |
|---|---:|---:|---:|---:|
| `OBJZw3bF0dg` | 614 | 245 / 197 | 5,702 | 6,805 / 6,272 |
| `aqz-KE-bpKQ` | 423 | 287 / 206 | 2,004 | 2,664 / failed (prefix changed) |
| `rfscVS0vtbw` | 704 | 267 / 233 | 2,052 | 2,821 / failed (prefix changed) |
| `8jLOx1hD3_o` | 668 | 272 / 252 | 5,716 | 3,544 / 4,000 |

A second run the same day (after the fallback-error reporting change to the
example) measured `OBJZw3bF0dg` 682 / 288 / 220 ms native against 9,845 ms
resolve and 15,956 / 16,558 ms yt-dlp comment pages; `aqz-KE-bpKQ` 508 / 209 /
258 ms native against 8,755 ms and 5,708 / 5,057 ms; `rfscVS0vtbw` 742 / 232 /
241 ms native against 4,345 ms and 4,377 ms / failed (prefix changed) again.

Every native comment page returned 20 rows, all with a policy-accepted portrait,
with no overlap between pages 1 and 2 and a further continuation. Native
metadata agreed with yt-dlp's for upload date, views, likes, comment count and
subscribers on all four. Native descriptions were equal in length, or longer
where truncated redirect links were restored to their targets (`rfscVS0vtbw`:
2,301 vs 2,211 characters). The yt-dlp second page failed on two videos because
the replayed prefix changed order between runs; the native continuation does
not have that failure mode. These are sequential samples from two runs; network
and helper variance is not characterized.

On 2026-09-29, `cargo run --locked -p serein-youtube --example comments_smoke` fetched the public Big Buck Bunny video `aqz-KE-bpKQ` in guest mode: first page 20, second page 20, both with more results. The example reports counts only; no comment text, author identities, credentials or signed media addresses are logged. An initial probe used depth 0 and returned no entries; inspecting the exact implementation corrected it to depth 1 before the successful two-page probe.

Deterministic fixtures cover disabled/missing/foreign responses, bounded pages/text, changed boundary identity, safety ceiling, cursor generation/video scope, and stale request cancellation. Generated JSON mutations and external-URL authority mutations assert output bounds, plain-text preservation, typed identity and rejection invariants. The native harness evidence below validates automatic opening/paging/cancellation at the small supported window. Manual keyboard/clipboard interaction and actual screen-reader qualification remain pending.

The complete-prefix check and its new regressions were authored during the exclusive soak measurement and have not yet been compiled or run. They cover cross-boundary reordering with an unchanged final ID, legitimate text edits, shared cursor storage and each page through the 200-comment ceiling. The earlier native checks below predate this correction.

The opt-in native harness `--comments-smoke-test --url URL` now reveals the inline description, explicitly loads two comment pages, checks bounded nonempty results and Previous state, then exercises request cancellation. It exits after 70 seconds and fails if assertions did not finish. `--comments-snapshots /absolute/existing/directory` writes two new diagnostic PNGs (`description.png`, `comments.png`); readback/encoding are diagnostics only and invalidate performance sampling for that run. The example fixture URL is the public Big Buck Bunny URL above. Its previous modal version passed on the available macOS host; that historical evidence follows and does not qualify the new inline layout.


## Inline layout validation status

The inline layout and account-metadata handoff were authored in the current UI pass. The finite native harness was adapted at source level but has not been executed for this layout. Keyboard selection/copy, long description expansion, full-page comment scrolling, account disconnect clearing, screen-reader output, and small-window geometry require native qualification. Historical screenshots below show the replaced modal layout, not this implementation. No performance or runtime checks were run during this pass.

## Historical native shared UI evidence (replaced modal layout)

The release binary built after 110 passing workspace tests (one ignored OS-secret test) and strict Clippy completed the 70-second harness with exit 0 at 760×600 logical pixels in light theme and a fresh isolated data directory:

```sh
caffeinate -u -t 5
caffeinate -d -i ./target/release/serein \
  --url 'https://www.youtube.com/watch?v=aqz-KE-bpKQ' \
  --comments-smoke-test --comments-snapshots /tmp/serein-comments-native.gotBGn \
  --ui-size 760x600 --ui-theme light \
  --data-root /tmp/serein-comments-native.gotBGn/state
```

The [description capture](evidence/2026-09-29-guest-description.png) and [second comment-page capture](evidence/2026-09-29-guest-comments.png) were inspected visually. The description/date/counts, comment text and author metadata, copy actions, scrollbar and page controls fit the window. These intentional diagnostic images contain real public YouTube content; they are neither fixture data nor shipped application chrome. The harness asserted 20 first-page rows, 20 second-page rows, retained Previous state and successful cancellation of an explicit third request. It did not test OS clipboard contents or actual screen-reader output.

The [native log](evidence/2026-09-29-guest-comments.log) observed Apple M1 OpenGL 4.1, VideoToolbox H.264 1920×1080 at 60 fps, Opus audio via avfoundation, and no media error. Catalog model notifications and full resets stayed at 0 while playback and comment paging ran. Two persistent GPU targets were reported. This run had 60 VO drops and included screenshot readbacks; it provides no optimized-playback, CPU, RAM or energy pass. No account credentials or writes were involved. The application and its finite caffeinate wrapper exited before the next measurement slot.

## Author portraits

Each comment shows its author's public portrait in the existing 40px circle. The
guest provider keeps the native `author.avatarThumbnailUrl` (or legacy
`authorThumbnail`), or yt-dlp's `author_thumbnail` on the fallback path, only when it is an HTTPS
`yt3.ggpht.com` / `yt3.googleusercontent.com` URL without credentials, port,
fragment or more than 4096 bytes (the same rule as channel portraits); anything
else is simply "no portrait". The app then fetches at most one page (20) of
portraits with the existing anonymous thumbnail fetcher (exact image hosts, no
proxy/cookies/redirects, 2 MiB input, bounded decode), four at a time, and resizes
each to at most 88x88 before it reaches Slint. The placeholder icon stays until a
portrait is ready or if it fails. A new page, a different video, disabling
comments, account playback or clearing supersedes the job, and late results are
dropped by generation and by row identity. Portraits are not persisted.

Validation: unit tests cover URL policy, resize bounds, stale-result dropping and
cancellation; a one-off live check fetched a real `author_thumbnail` from
yt-dlp output through the worker and got an 88x88 image. The full comment page
with portraits was **not** rendered natively: this session had no display clock
(`presenter setup failed -6661`), so guest playback and the comments smoke could
not start. Visual review of the 40px clipped portrait is still open.

