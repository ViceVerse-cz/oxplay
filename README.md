# Serein

An experimental, open-source native YouTube client: one compiled Slint UI and an
in-process Rust/libmpv core. This repository implements an early working slice of
[SPEC.md](SPEC.md), **not a completed or release-qualified product**.

Implemented: native shared UI, embedded local video via persistent OpenGL targets,
play/pause, seek, fullscreen, explicit local subtitle loading/cycling, real guest
YouTube video/channel/playlist search, public channel and playlist pages, and
direct content stream resolution/playback. Genuine video descriptions and optional
metadata appear inline beneath the player; public read-only comments
load explicitly in bounded pages. The provider runs one cancellable
bounded yt-dlp process group; each displayed guest catalog page has at most 20 rows.
The shared UI now includes responsive thumbnail cards, themes, local collections,
local follows, opt-in history, import/export, and account connection screens.
Account code implements explicit session import, protected storage, identity
verification, remote reads and reconciled writes; real account qualification still
requires an authorized local human test. No credentials are imported automatically.
Hardware decoder observations do not establish the optimized playback gate:
the default macOS presenter still exceeds the CPU release ceiling in some
measured repeats. A restricted native-presenter experiment reduces UI drawing
but remains unqualified and misses the CPU target.
See [performance evidence](docs/performance.md).

## Build on the development Mac

Install Rust 1.98.1 (pinned in `rust-toolchain.toml`), Apple's Command Line Tools, and development packages
using Homebrew. A graphical macOS session is required to run the application:

```sh
brew install pkgconf mpv yt-dlp
cargo build --locked --release
./target/release/serein
```

The inspected development environment is Apple Silicon macOS 27.0. Other targets
remain experimental; see [platform matrix](docs/platform-matrix.md). Linux builds
need libmpv development files and Winit/X11/Wayland development prerequisites;
the [CI workflow](docs/ci-release.md) compiles and tests the selected features
on Ubuntu. Native X11 and Wayland playback remain unvalidated. Windows extraction currently fails
closed until process-tree supervision is implemented.

Slint runtime/build compiler use the identical upstream Git revision recorded in
[dependencies](docs/dependencies.md). Normal builds use `--locked`; upstream
updates must be reviewed and tested. No project API keys are needed. Native
libraries/helpers are installed development prerequisites, not bundled executables.
The media build requires `pkg-config` to find **libmpv client API 2.5 or newer**
(the tested mpv executable is 0.41.0). FFmpeg is required for fixture generation;
the inspected Homebrew mpv package installs it as a dependency. The inspected
yt-dlp package supplies Python, Deno and packaged EJS challenge scripts.

## Use

Description and comments sit beneath the watch-page title and actions. Expand
long descriptions with **Show more** and load real public comments explicitly;
Next/Previous keeps each page bounded to 20 comments in the same page scroll.
The shell has a rounded search field and shared custom window controls.
**Settings → Appearance** offers session-local translucent title/header/sidebar
surfaces and optional experimental macOS blur. Video and main content stay
opaque. Other backends report their appearance limitations. See
[window appearance](docs/window-appearance.md).

Search or paste a supported HTTPS YouTube URL in the centered field. Filter real
results by videos, channels, or playlists. Select a video to play, a channel to
browse its public tabs, or a playlist to browse its videos. Canonical `/channel/UC…`
and `/playlist?list=…` URLs, standalone `@handles`, and channel-handle URLs are supported. The initial resolver selects up to 1080p,
preferring H.264 at equal resolution/frame rate. No network work occurs on clean
launch. History, autoplay, previews, telemetry, and background refresh are off.

```sh
./target/release/serein --local /absolute/path/video.mp4 --subtitle /absolute/path/subtitles.srt
./target/release/serein --yt-dlp /absolute/path/yt-dlp
./target/release/serein --url 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

The Mac default helper paths are `/opt/homebrew/bin/yt-dlp` and
`/opt/homebrew/bin/deno`. No helper, plugin, or runtime is downloaded at playback.
Explicit `--yt-dlp` and `--deno` paths support nonstandard installations.
The opt-in `--scoped-media` diagnostic on macOS requires the first-party
`serein-dns` executable beside `serein` (bundles place it in `Contents/Helpers`).
`cargo build --locked --release --workspace` builds both. This short-lived DNS
helper is supervised and bounded; the experimental transport is not a proxy or
production-network qualification claim.
The shared watch controls offer a quality ceiling and selected-only guest VTT captions.
The search field opens standalone `@handles` and HTTPS channel-handle links,
then uses the returned stable channel ID for paging, tabs and local following.
See [channel handles](docs/channel-handles.md) for input and validation limits.
Public paging uses bounded helper
slices; deep-page efficiency is not yet qualified. See the
[guest catalog UI evidence](docs/guest-catalog-ui.md).
The CC button opens the [guest caption selector](docs/captions.md) for online videos
and cycles available tracks for local files. Info shows observed decoder and application counters. Local collections and YouTube account data remain separate.
Home opens your recently saved local videos, deduplicated across playlists, with
bounded Next/Previous pages. It starts without fetching thumbnails or remote
recommendations. Save videos to a local playlist to populate it; local collections
stay separate from your YouTube account. See [local Home](docs/local-home.md).
Local playlists also offer **Search this playlist**, with Apply/Enter and Clear,
to find saved titles and channel names beyond the visible page without network
access. **Duplicate** creates a named metadata-only copy of a playlist; each saved
video offers **Copy / Move** to another explicitly selected local playlist.
See [local library](docs/local-library-ui.md).

**Share** on the watch page offers a public video link or a link at the current
playback time. It copies neither signed media URLs nor account credentials, and
sends nothing automatically. See [sharing](docs/sharing.md).

**Open video file** (Ctrl/Cmd+O) selects a local MP4/MOV or Matroska/WebM file.
**Load subtitle file** attaches a selected UTF-8 SRT/WebVTT file to the current
local video. These paths are not saved to history. See [local media](docs/local-media-policy.md).

Timestamped YouTube links retain their requested initial position. Click the
elapsed time or press **G** for **Jump to time**. Resolved online videos with
chapter metadata expose a bounded list through **Video chapters**. These additions
have compile/lint validation; native interaction qualification remains pending.
See [time navigation](docs/time-navigation.md) and [timestamp links](docs/timestamp-links.md).

The picture-in-picture button or **P** switches the same window into a borderless floating
player with controls inside the video; **P**, **Escape** or closing that compact window restores browsing.
It has been functionally exercised on the development Mac; native Wayland and
the experimental child presenter keep it disabled. See [PiP details](docs/picture-in-picture.md).
The account page explains session import and its risks before enabling an explicit
file selection. Account playlist videos have a separate **Play with account**
action backed by revocable scoped media transport. This path is implemented but
real-account playback remains unverified; account captions/comments and private
local storage are unavailable. Guest playback never escalates automatically. See [privacy](docs/privacy.md),
[account status](docs/account-provider.md), and [provider limits](docs/provider.md).

Known ad/promoted metadata shapes are filtered, and native playback uses the
resolved content stream without an advertising web player. Live guest/authenticated
ad-suppression qualification is unfinished; this is not a universal ad-free promise.
No account session, Premium account, or fabricated blocked-ad count is used as proof.

## Validation

```sh
cargo fmt --all -- --check
cargo check --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
scripts/generate-fixture.sh
./target/release/serein --local artifacts/local-1080p60.mp4 --subtitle artifacts/local.srt --smoke-test
python3 scripts/measure.py --output artifacts/playback.json -- ./target/release/serein --local artifacts/local-1080p60.mp4 --quit-after 85
```

`--smoke-test` is explicit fixture mode; its synthetic catalog rows are labeled.
It exercises pause, seek, resize, fullscreen, subtitles, minimize/restore, pointer movement, and
catalog notification assertions. It is not a perceptual A/V, screen-reader, or
full platform test. [Progress](docs/progress.md), [video integration](docs/video-integration.md)
and [performance](docs/performance.md) distinguish executed checks from pending gates.

Source license: GPL-3.0-or-later. Slint uses its GPL-3.0-only option. Combined binary
distribution and native dependencies require the audit in [licensing](docs/licensing.md).
UI icons are unmodified [Lucide assets](crates/app/ui/icons/SOURCE.md), with their
ISC and retained Feather MIT notices. The current [developer macOS bundle](docs/packaging.md)
is not a portable, signed release: helper packaging, source association and notices
remain release work.

[CI and manual source previews](docs/ci-release.md) describe the macOS/Linux
build matrix and draft-only source-release workflow. Binary publishing remains
blocked on the documented platform, packaging and licensing gates.
