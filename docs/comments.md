# Public video details and comments

`ResolvedPlayback.details` contains optional genuine description, upload date, views, likes and reported comment count from the already extracted video JSON. Resolving a video does not start a separate metadata request. Missing values stay absent. Descriptions are bounded to 20,000 Unicode characters; control characters other than line breaks/tabs are removed. The shared watch page displays the description directly below the video title/actions. Its selectable read-only preview shows up to three lines / 300 characters; Show more expands the actual bounded description inline. The outer watch page scrolls the content, without reconstructing the video host or opening a modal. Already-resolved account description/metadata can be displayed in memory too; disconnect clears it through the existing account lifecycle.

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
account is connected. The provider prefix-replay limits below are unchanged.

## Reviewed protocol path

The installed yt-dlp `2026.08.19` source was inspected, including [YouTube comment extraction](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_video.py#L2374), `_comment_entries` (root depth starts at **1**), `_get_comments`, and [the common post-extractor](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/common.py#L3882). yt-dlp performs the real guest InnerTube `next` requests and normalizes comment identity, text, author and counts. The adapter requests:

```
--get-comments --no-ignore-errors --ignore-no-formats-error --no-playlist
--extractor-args youtube:comment_sort=top;max_comments=COUNT,COUNT,0,0,1
```

COUNT is the requested prefix plus 20 comments and one lookahead. The five limits bound total comments, parent comments, replies, replies per thread and depth. Normal helpers retain the 45-second deadline, 32 MiB stdout/64 KiB stderr bounds, zero retries, disabled configuration/plugins/cache/remote components and process-group cancellation/reaping. The helper receives no cookies. `comments:null` is unavailable; a missing/mistyped comment field is an unsupported response, never a successful empty page. A genuine empty array is reported explicitly as no public comments returned.

The opaque cursor binds the video, session generation, prefix offset and ordered IDs of every previously published comment. Its immutable prefix contains at most 180 IDs of at most 256 bytes each; cloned navigation cursors share that allocation. At most 200 top-level comments are browsable, with a distinct limit message. This is **prefix replay**, not an exposed InnerTube continuation: each next page re-extracts earlier comments. Any changed published identity or ordering fails with a refresh action, including a previously seen comment moving across an unchanged page boundary. Matching identities do not guarantee a stable text/count snapshot: public comments can be edited. Deep-page latency, helper memory and quota cost are not qualified. A direct Rust continuation adapter could remove replay once separately validated.

## Observed checks

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
guest provider keeps yt-dlp's `author_thumbnail` only when it is an HTTPS
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

