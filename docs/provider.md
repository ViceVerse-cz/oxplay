# Guest provider evidence

Recorded 2026-09-29 on the available macOS Apple Silicon host. The working adapter is `crates/youtube`; domain values are in `crates/core`. The public catalog/resolver remains guest-only by default. The separate account adapter now implements identity, remote collections/writes, explicit cookie import, and protected-storage integration; real human account and connected-account ad qualification remain unpassed gates. See [account evidence](account-provider.md).

## Runtime and source verification

Homebrew yt-dlp `2026.8.19_1` reports `2026.08.19`. Deno reports `2.9.7`. The actual installed executable's `--help` was read to verify configuration/plugin/runtime isolation, remote-component denial, JSON output, and format sorting. The current [yt-dlp CLI documentation](https://github.com/yt-dlp/yt-dlp) and [EJS guide](https://github.com/yt-dlp/yt-dlp/wiki/EJS) were consulted. A packaged `yt_dlp_ejs` module is installed; the application never enables remote EJS downloads.

Observed SHA-256:

| Component | SHA-256 | Scope |
|---|---|---|
| yt-dlp Homebrew entrypoint | `ef6b151cf04c6882b6889aa4fb73bcbcb046a5f9d9e9c3fb56dcaea35f6399c1` | `/opt/homebrew/Cellar/yt-dlp/2026.8.19_1/libexec/bin/yt-dlp`; Python launcher only, not the entire dependency closure |
| yt-dlp Python source tree | `86bb02e3863faf344ec7a2dc3e366cbdc0d6f2adf1e40687aa19c840b0a046ca` | 1,049 non-bytecode package files; aggregate method below |
| yt-dlp EJS package tree | `67854fd94c5fa2e111901fa315045a50773c058e6b92c3543143a4eb295b1ecc` | 6 non-bytecode package files; aggregate method below |
| Deno | `0443b58059a74c547f942d9a9e96939fd673008186c812c8a7dc0b5aced8e23e` | `/opt/homebrew/bin/deno`, resolved installed executable |

The package-tree SHA-256 feeds sorted relative UTF-8 paths, a NUL, file contents, then another NUL into one digest. `__pycache__` directories and `.pyc` files are excluded. Trees are rooted at the corresponding module below Homebrew’s `libexec/lib/python*/site-packages`. This does not hash the Python interpreter or every installed dependency.

These observations are a development inventory, not an application update trust root. Release packaging still needs the complete helper/runtime dependency inventory, license audit, reproducible integrity check, and update/rollback policy.

## Working behavior and exact checks

```
cargo test --locked -p serein-core -p serein-youtube
cargo clippy --locked -p serein-core -p serein-youtube --all-targets -- -D warnings
cargo run --locked -p serein-youtube --example guest_smoke -- /opt/homebrew/bin/yt-dlp /opt/homebrew/bin/deno search 'Blender Big Buck Bunny'
cargo run --locked -p serein-youtube --example guest_smoke -- /opt/homebrew/bin/yt-dlp /opt/homebrew/bin/deno resolve 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

Initial video-only slice observed: 14 deterministic unit tests passed (4 core, 10 provider/supervisor); clippy passed with warnings denied. Public search returned 20 videos on page one and 20 on page two. Public URL resolution returned 1920×1080 video and a separate audio stream with expiry. The example emits counts and non-sensitive stream characteristics, never signed URLs or raw provider JSON. Extraction alone does not prove delivered playback, audio synchronization, hardware decode, or live ad suppression.

The current default resolution policy caps video at 1080 pixels high. Explicit `ResolutionPolicy` supports a height ceiling from 144 through 2160 and an H.264 preference. Resolution and frame rate sort before codec preference (`height,fps,vcodec:h264`), so preference alone does not reduce those qualities to obtain H.264. This is a codec preference; only the media engine can confirm hardware decode. The application must disclose the ceiling and actual fallback decoder. No 4K or HDR capability is implied by accepting a policy value.

The compatibility video-search API keeps at most 20 normalized video results per page and limits a query to 200 results. The application now uses the typed video/channel/playlist catalog API below, with the same 20-row page bound. Cursors are scoped and provider-owned. Reissuing paged searches can observe a changing upstream result order; stable remote snapshot semantics are not promised.

## Ownership, safety, and unfinished qualification

Each adapter permits one concurrent operation. Calls are blocking and belong on the application's dedicated worker. The worker has a latest-job slot and a single result slot, cancels superseded work, and compares request IDs before applying results. The cancellation check/publication race cannot apply a stale result because `take()` checks the current ID. The public-catalog worker uses guest session generation zero. Account work uses a separate generation-controlled client; authenticated extraction explicitly checks its session generation and cancellation, rather than reusing this guest worker implicitly.

Helpers receive argument arrays, a cleared environment, explicit minimal PATH, no stdin, disabled inherited configs/plugins/caches, disabled remote challenge components, zero retries, and no mark-watched action. The only opt-in runtime is an explicit Deno path. Ambient proxy variables are not inherited and direct guest mode is explicit. This is not strict proxy support.

On Unix, a process group is created at spawn. stdout is limited to 8 MiB, stderr to 64 KiB; the deadline is 45 seconds and cancellation is observed during waits within roughly 20 ms. Drain batches are bounded so a chatty child cannot starve deadline checks. Every exit path terminates the group and reaps the direct child. The operating system reaps orphaned descendants. A regression test verifies a successful parent cannot leave a background child running. Deliberately escaping the process group is outside this mechanism. Windows extraction fails closed pending a job-object supervisor. The supervisor has no idle polling loop.

Input video URLs normalize to validated YouTube IDs. Initial resolved media must use HTTPS on a `googlevideo.com` subdomain, with no credentials, nonstandard port, or fragment. Signed URL Debug/Display is redacted. Expiry and a one-attempt generation-scoped refresh budget are implemented; application refresh/resume integration remains unfinished. Normalized headers carry their exact media origin; only bounded User-Agent, Accept, Accept-Language, Sec-Fetch-Mode, Referer, and Origin values are retained. Credential headers, control-character injection, and cross-origin referers are rejected. Actual forwarding must preserve origin scope. DNS, redirects, FFmpeg segment requests, and extractor egress have not passed whole-process confinement tests; this remains a release blocker.

Known ad/promoted metadata renderer fixtures are excluded before normalization. Content streams are passed to the native player without running the YouTube advertising player. No ad counter is fabricated. Guest live ad behavior, account ad behavior, unknown renderers, and inseparable in-stream advertisements are not qualified by the extraction checks above. Universal suppression is not claimed.

Manual and automatic/translated VTT tracks are now discovered from real resolver metadata, with selected-only bounded guest download ([caption evidence](captions.md)); missing returned tracks do not assert that a video has no captions. Shared native selection, Off, cached reselection and quality-change reattachment passed the finite macOS diagnostic; visual glyph appearance and other platforms remain unqualified. Other unfinished qualification includes authorized human account verification and mutations, hardware-aware format availability/selection UI, application stream refresh, live rate-limit/challenge behavior, and complete transport confinement. Local cooldown handling and protected account import/verification code are implemented, but synthetic checks do not establish live account capability.

Media request headers fail closed: credential headers and every unknown extractor header are rejected, rather than silently dropped. The reviewed yt-dlp `utils/networking.py:161–166` declares User-Agent, Accept, Accept-Language and `Sec-Fetch-Mode: navigate` as its default browser headers. The adapter retains these fields, accepting only `navigate` for Sec-Fetch-Mode, plus validated YouTube Origin/Referer values. Retaining a field does not authorize forwarding it across media redirects; the presenter must independently qualify required headers. Unknown-header and non-default Sec-Fetch-Mode fixtures are covered by the header policy test.

## Typed public catalog pages

`YtDlp::catalog(&CatalogRequest, Option<&CatalogCursor>, &OperationContext)` adds guest search kinds (all, videos, channels, playlists), channel tabs (videos, shorts, streams, playlists), and public playlist contents. `serein_core::CatalogItem` normalizes video, channel, and playlist summaries before they reach an application adapter. Counts, descriptions, and avatars remain optional when the provider does not supply them. The compatibility video-only API remains available. The application now integrates the typed interface through `guest_ui.rs`, kind filters, shared cards, and channel/playlist pages; see [UI evidence](guest-catalog-ui.md).

The implementation was checked against the installed yt-dlp 2026.08.19 primary source: `extractor/youtube/_search.py` SearchURLIE forwards query/filter parameters and includes a channel-result fixture; `_tab.py` normalizes channel/playlist renderers and tab metadata; `options.py` defines flat/lazy playlist slicing; `YoutubeDL.py` bounds resolved entries to the selected slice. Type-filter bytes are independently derived from YouTube.js revision `bad89d2657e88f907011655f199fba9fb615c339`, `src/Innertube.ts:201–269` and `protos/generated/misc/params.ts`: nested filter field2/type field2 with video1, channel2, playlist3, followed by the reference's URI-encoded base64 convention. No JavaScript service is introduced.

Each job asks for 20 records plus one lookahead. Only 20 normalized records leave the adapter; 21 raw entries is the maximum accepted shape. Cursors bind the exact request and session generation. Limits are explicit: 200 search records, 10,000 browse records; `limit_reached` distinguishes the safety cap from remote exhaustion. A page containing unrecognized records is marked partial, and an entirely unsupported nonempty response is an error, not a successful empty page. Known promoted entries are excluded before normalization. This does not establish universal ad suppression.

The cursor records a supervised-helper slice offset. It is **not** a captured InnerTube continuation token. Each next-page helper may revisit earlier remote pages; lazy extraction avoids intentionally requesting the entire catalog, but this approach does not prove deep-page latency or helper memory budgets. Jobs retain the existing 45-second timeout, process-group cancellation/reaping, bounded output, clean environment and guest-only configuration. No account session is automatically used if public browsing fails. Canonical channel IDs/URLs are supported; resolving arbitrary channel handles is not yet implemented.

Channel avatar metadata permits only exact `yt3.ggpht.com` and `yt3.googleusercontent.com` hosts in addition to existing ytimg hosts. A thumbnail consumer must independently enforce the matching redirect/egress policy; normalized metadata is not permission to bypass that policy.

Synthetic coverage now includes all three result kinds, actual filter serialization, page lookahead/final-page/cap behavior, cross-query and cross-generation cursor rejection, channel/playlist identity mismatches, promoted and unknown entries, and unsafe targets/avatars. `cargo test --locked -p serein-core -p serein-youtube` passed 5 core + 41 provider tests, and strict Clippy passed. `examples/catalog_smoke.rs` is an explicit public-network check that reports counts only. On the available macOS host, the live guest check completed eight sequential supervised calls: channel search returned 20 records then 20 on page two, and the selected real channel's videos page returned 20; playlist search returned 20 then 20, and its selected public playlist returned 20 videos; mixed search returned 12 videos, 1 channel, and 7 playlists, followed by 20 results on page two. Every observed page had `partial=false`; full first pages reported continuation. The helper exited after the check. No account session, cookies, playback, or account mutation was involved. Shorts, streams, and channel-playlist tabs still need their own live validation, and application UI integration remains separate from this provider proof.

## Public details/comments and Deno update isolation

Optional video details now come from the already returned playback JSON. The shared UI exposes explicit guest-only paginated comments through the same bounded worker; two genuine public pages returned 20 comments each. See [comments implementation, primary sources and limits](comments.md).

The supervisor explicitly sets `DENO_NO_UPDATE_CHECK=1` after clearing its environment. The installed Deno 2.9.7 `--help` and [that exact version’s update implementation](https://github.com/denoland/deno/blob/v2.9.7/cli/tools/upgrade.rs#L294) confirm this disables its background update check before the non-TTY prompt condition. A subprocess test verifies the actual child environment. This closes that specific automatic network request; it does not establish whole-helper egress confinement.
