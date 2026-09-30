# Search suggestions

The header search field shows a YouTube-style dropdown with up to ten rows:
your earlier searches (clock icon, removable with the trailing x) and, while you
type, completions from YouTube (search icon). The part you typed is shown in
normal weight and the completion in bold. Up/Down move the highlight and preview
that row in the field (previews are not edits and send nothing); Enter or a
click submits it; Escape closes the panel and keeps focus in the field; Down
reopens it. The panel closes on submission or when the field loses focus. It is a
late-drawn overlay in `app.slint` (component in `ui/search-suggestions.slint`),
not a `PopupWindow`, so it never takes keyboard focus. The field is disabled in
the native-child diagnostic, so the panel cannot open above that native view.

## Local search history

Only plain-text catalog searches are remembered. Video, channel and playlist
links and `@handles` open content and are never kept; text that looks like a URL
(`://`, `www.`, `youtube.com/`, `youtu.be/`, `http:`/`https:`) or contains
control characters is rejected by storage too, so pasted links with tokens are
not written. Whitespace is collapsed; duplicates are matched case-insensitively
and the newest spelling moves to the top. At most 50 are kept.

- **Local history off (default):** searches are remembered in memory for the
  current session only. Nothing is written to disk.
- **Local history on** (Settings → Keep watch history on this device): searches
  are also stored in SQLite (`local_search_history`, schema 8). The insert
  rechecks the opt-in inside SQL. They share watch history's retention period.
- Turning history off deletes stored searches in the same update (trigger) and
  clears the in-memory list. Turning it on starts from what is stored.
- **Clear history** and **Clear local data** delete stored searches and clear
  the in-memory list. Removing one row deletes its stored copy.
- The explicit SQLite backup copies the whole database, as for watch history.
  Library export excludes history.

## YouTube completions

**Show search suggestions from YouTube** in Settings is **on by default** (the
product owner asked for it) and is persisted with other preferences (schema 8).
While it is on, typed text is sent to YouTube as you type. When it is off, only
local history appears and no completion request is made.

Request, sent only when the field is focused, the text is 1–200 characters and
not URL-like, after 220 ms without further typing:

```
GET https://suggestqueries-clients6.youtube.com/complete/search?client=firefox&ds=yt&oe=utf-8&ie=utf-8&q=<text>
```

Observed on 2026-09-30 with curl: HTTP 200, `text/javascript; charset=UTF-8`,
body `["rust lang",["rust language","rust lang",…],[],{"google:suggestsubtypes":…}]`,
with no `Set-Cookie`. Without `oe=utf-8` the same body is ISO-8859-1 (handled
too). `client=youtube` returns JSONP `window.google.ac.h([...])`, which the parser
also accepts. `suggestqueries.google.com` set a Google cookie and is not used.

Client: a dedicated anonymous reqwest client. It is HTTPS-only with no cookie
store, ambient proxy, redirects or retries. It uses a 3 s connect / 5 s total
timeout, a 64 KiB response limit and this exact host/path only. Account cookies
are never attached. Only the newest query is kept. A newer keystroke, blur,
submission or disabling the setting drops the in-flight request. Stale answers
are discarded by serial and text. HTTP 429 pauses completions for 60 s.
Failures are silent; local history still works. Query text is not logged. Finite
smoke tests, fixtures and native diagnostics (`Options::finite_diagnostic`) start
no completion worker.

## Validation and limits

Unit tests cover parsing (JSON/JSONP/Latin-1, malformed input, entry and body
limits), request URL admission, history normalization/dedupe/bounds, persistence
gating, migration, retention and both clear flows. Mock-backend UI tests cover
panel placement, keyboard preview, Escape, removal and click submission. They
also cover the acknowledged setting and header centering.

Not validated: the dropdown was not viewed in a running native window, so
visuals, font metrics of the split bold text and hover were not seen. The live
completion request did not run from the app, only via manual curl. Also not
validated: IME composition while arrows are captured, and screen-reader output.
Clicking an empty, non-focusable area does not blur the field in Slint, so the
panel stays open until focus moves, Escape or submission.
