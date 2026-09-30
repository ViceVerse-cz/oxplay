# Public channel portraits and subscriber counts

The watch page requests a genuine channel avatar only after public guest playback
has selected a video with a typed channel ID. Account playback and local files
show the existing Lucide account glyph; private account metadata never enters the
anonymous channel lookup.

A single latest-request worker shares the existing resolver and its rate-limit
state. It uses the isolated supervised yt-dlp runner with `--flat-playlist
--playlist-items 0` for channel metadata, verifies the returned channel identity,
and accepts only artwork tied to the extractor's explicit `avatar_uncropped`
entry. Channel banners and video thumbnails are not identity pictures. The
original cropped rendition of the same avatar is preferred when available.

When the native InnerTube `next` watch page (see [comments](comments.md)) has
already returned the owner portrait for the same resolved channel ID, the worker
uses that exact-host `yt3` URL and subscriber count and skips the yt-dlp channel
metadata run; otherwise the run below remains the fallback. Channel pages still
use the extractor.

The metadata contract was inspected in the installed official yt-dlp 2026.8.19
`YoutubeTabIE._extract_metadata_from_tabs` and `_real_extract` source. Unknown
metadata shapes yield the icon fallback. This is source inspection, not a claim
that a live channel request or the rendered image passed native qualification.

Optional avatar extraction has lower priority than foreground provider work.
Search, stream resolution, and captions cancel an active avatar helper and wait
on their worker for its process-tree teardown before acquiring the provider slot.
No extra simultaneous extractor is admitted. The single avatar image request
then uses the existing anonymous image downloader: exact HTTPS image origins,
no credentials, inherited proxy or redirects, 12-second request timeout, 2 MiB
encoded data, bounded decoder allocation/dimensions, and resized pixels before
UI publication. The image is transient and never written to the artwork cache.

Changing the selected video, local/account handoff, clearing local data,
leaving the visible watch page, PiP/fullscreen, minimizing and shutdown cancel
unneeded work. Generation and selected public channel/video checks reject stale
results. One pending request and one completed image are retained; the worker
is joined during shutdown. No new progress timer or polling loop is added.

Live provider behavior, native circular clipping and screen-reader behavior
remain unqualified in this fast formatting/lint/build pass.

## Channel-page integration

The same bounded worker now serves public channel headers as well as the watch
creator row. It retains at most one accepted watch profile and one current public
channel profile. Accepted metadata must match the canonical typed channel; the
page clears portrait/count state on a different channel identity. The verified
profile contains an optional genuine follower count, so a missing/negative count
stays absent and a known zero is displayed accurately. Extracted watch metadata
can supply the count without another request; public channel metadata can update
it even if image download fails. Count labels use compact English K/M/B notation.

Channel pages show the genuine profile image beside the title and subscriber
count, with the existing glyph if unavailable. Watch creator hover is limited to
the avatar/name/count action rather than the full action row. Recommended cards
omit the author line. No avatar/count is fabricated. Focused sanitized provider
fixtures pass identity, explicit-avatar-marker and absent/negative-count checks;
live portrait/native circle clipping remain unverified for this slice.
