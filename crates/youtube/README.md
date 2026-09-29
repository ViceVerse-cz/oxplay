# YouTube provider

Blocking guest APIs run on the application's single owned worker. No helper survives an idle operation. `YtDlp::catalog` returns typed video/channel/playlist results, canonical channel tabs, and public playlist contents. Pages contain at most 20 records, with a 200-record search cap and 10,000-record browse cap. Opaque cursors bind the exact request and session generation; a safety cap is reported distinctly from remote exhaustion. The older video-only `search` API remains available, while the application uses the typed catalog adapter.

The constructor requires an absolute helper path. `with_deno` explicitly allows one installed runtime; discovery is disabled. The subprocess receives a clean environment and argument array, ignores all inherited configuration and plugins, does not persist extractor caches or mark videos watched, and cannot fetch remote challenge components. Guest operations supply no credentials and never escalate automatically when an account is connected. Proxy inheritance is deliberately disabled; this is direct guest mode, not strict proxy protection.

The Unix supervisor owns a process group, bounds stdout to 8 MiB and stderr to 64 KiB, enforces a 45-second deadline, and checks cancellation at most 20 ms apart during waits. It kills the group and reaps the direct child on every exit path. Orphaned descendants are reaped by the operating system; helpers deliberately escaping their group are outside this mechanism. Windows extraction fails closed until a job-object implementation is validated. There is no idle polling thread.

Only canonical YouTube video IDs enter resolution. Direct HTTPS formats at up to 1080p are selected, preferring H.264 only after resolution and frame rate; an explicit `ResolutionPolicy` can change the ceiling/preference; video and audio are returned separately. Initial media addresses must be HTTPS on a `googlevideo.com` subdomain, without credentials, nonstandard ports, or fragments. Signed addresses have redacted Debug/Display and are never persisted here. Expiry is parsed, and the domain provides a generation-scoped one-attempt refresh budget for known expiry; automatic application refresh/resume is not yet connected. The adapter rejects an unexpected returned video ID.

The separate `account` module implements explicit bounded session import, identity verification, capability reporting, protected-session handoff, remote reads, and reconciled writes. No real account credentials were supplied or tested by the agent; see [account adapter evidence](src/account/README.md). Remaining release blockers include transport-level redirect and DNS confinement, origin-specific header forwarding, subtitle retrieval, hardware-aware format selection, real human account qualification, and authenticated ad-suppression tests. Empty subtitle lists currently mean the adapter has not implemented subtitle discovery, not that the video has no captions. No strict proxy or universal suppression claim is made. Known ad/promoted metadata shapes are filtered before normalization; direct content playback does not run YouTube's advertising player. Unknown or inseparable stream advertising has not been qualified.

## Validation observed on 2026-09-29

The installed Homebrew yt-dlp reports `2026.08.19` (formula revision `2026.8.19_1`), with Deno `2.9.7`. Flags were verified using that executable's `--help` as well as upstream [CLI documentation](https://github.com/yt-dlp/yt-dlp) and [EJS guidance](https://github.com/yt-dlp/yt-dlp/wiki/EJS). Homebrew includes the `yt_dlp_ejs` Python package. Downloads of remote components remain disabled.

The explicit `guest_smoke` example prints only counts and stream characteristics. Real public search returned two pages of 20 videos. Resolution of the public Big Buck Bunny video `aqz-KE-bpKQ` returned 1920×1080 video, separate audio, and signed-URL expiry. This proves extraction, not actual player delivery, advertising behavior, or hardware decoding.

```
cargo run --locked -p serein-youtube --example guest_smoke -- /opt/homebrew/bin/yt-dlp /opt/homebrew/bin/deno search 'Blender Big Buck Bunny'
cargo run --locked -p serein-youtube --example guest_smoke -- /opt/homebrew/bin/yt-dlp /opt/homebrew/bin/deno resolve 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

Routine tests are deterministic and network-independent. The live example is opt-in only and never reads browser profiles or imports account credentials.

## Guest catalog validation

The typed catalog interface was verified against the installed yt-dlp source and the pinned YouTube.js protocol reference. Guest-only live checks returned two 20-record pages each for channel and playlist searches, 20 videos from a selected public channel and playlist, and a mixed page containing videos, a channel and playlists. Provider fixtures cover identity checks, cursor scoping, lookahead, caps, unknown shapes and promoted metadata filtering. The new shared Slint UI routes genuine typed results through the same supervised worker; it does not invent VideoSummary records for channel/playlist cards. Shorts, streams, channel-playlist tabs, deep paging and native interaction still require additional qualification. See [provider evidence](../../docs/provider.md) and [guest UI evidence](../../docs/guest-catalog-ui.md).

The helper slice cursor is not an exposed InnerTube continuation token. Later pages may revisit earlier remote pages; the 45-second supervisor deadline and output bounds remain in force. Media headers fail closed on unknown fields or credentials. The inspected defaults User-Agent/Accept/Accept-Language/Sec-Fetch-Mode are retained, with Sec-Fetch-Mode restricted to `navigate`; normalized Origin/Referer values still require safe native transport handling.

## Public comments and video details

Playback results now retain bounded optional descriptions/counts/upload dates from the existing extraction response. Explicit guest `comments` requests expose 20 top-level records per page, at most 200, with scope-bound opaque cursors and changed-order detection. Later pages replay the bounded prefix; they are not remote snapshot/continuation tokens. A real public probe returned 20+20 comments; no author/comment contents were logged. See [comments evidence and limitations](../../docs/comments.md).
