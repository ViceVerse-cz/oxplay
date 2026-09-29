# Resolved video chapters

The shared player offers a Chapters panel for remote videos. Its bounded
standard Slint list supports keyboard selection, scrolling and an explicit
Jump action. Selection alone never seeks. Chapter starts are passed to the
existing media seek controller and retain the user's pause state. There is no
chapter clock, timer, background request or automatic skipping.

Chapter ranges and optional titles come from the resolved yt-dlp metadata.
The adapter accepts at most 200 ordered, non-overlapping ranges with finite,
nonnegative starts and ends, a positive interval, and ends within the reported
duration when present. Invalid ranges, malformed values, excessive count or
oversized titles reject the entire chapter set. Playback itself can continue
with an unavailable chapter explanation. Missing titles display a numbered
chapter label; ranges are never synthesized from descriptions or fixtures.

The metadata shape follows the [yt-dlp extractor contract](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/common.py)
(`chapters`, `start_time`, `end_time`, optional `title`), inspected on
2026-09-29. No SponsorBlock service or chapter-derived advertisement claim is
introduced.

Each installed list owns a non-reused UI epoch, typed video identity and exact
accepted native load. Jump checks those identities, current active playback,
the selected range and current guest/account authority. A replacement installs
a new list epoch; sign-out and explicit media clearing retire the old rows and
authority. Connected chapters remain in memory and never trigger a guest
metadata lookup. Local media chapters are not supported by this adapter.

Focused source regressions cover parser bounds and range rejection plus stale,
failed, stopped and not-yet-active native load admission. They were added but
not run in this implementation pass. The list's keyboard implementation was
inspected in the pinned Slint source; physical keyboard, screen-reader,
live-provider and native seek behavior remain to be qualified. No performance
tests were run.
