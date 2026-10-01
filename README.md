# Oxplay

An experimental, open-source native YouTube client: one compiled Slint UI and an
in-process Rust/libmpv core. This repository implements an early working slice of
[SPEC.md](SPEC.md), **not a completed or release-qualified product**.

Implemented: native shared UI, embedded local video via persistent native GPU textures,
play/pause, seek, fullscreen, explicit local subtitle loading/cycling, real guest
YouTube video/channel/playlist search, public channel and playlist pages, and
direct content stream resolution/playback. Genuine video descriptions and optional
metadata appear inline beneath the player; public read-only comments
load explicitly in bounded pages. Guest search, channel and playlist pages are
read by a native anonymous InnerTube client (yt-dlp only as a fallback); stream
resolution runs one cancellable bounded yt-dlp process group; each displayed
guest catalog page has at most 20 rows.
The shared UI now includes responsive thumbnail cards, themes, local collections,
local follows, opt-in history, import/export, and account connection screens.
Account code implements explicit session import, protected storage, identity
verification, remote reads and reconciled writes; real account qualification still
requires an authorized local human test. No credentials are imported automatically.
Source builds now default to Metal on macOS, Vulkan on Linux, and D3D11 media
shared with the DX12 UI on Windows. They require pinned, patched media libraries;
stock libmpv remains available for the explicit OpenGL comparison build.
The upstream native extensions are experimental. Hardware decoder observations
alone do not qualify presentation, performance or a release; see
[native rendering](docs/native-rendering.md) and [performance evidence](docs/performance.md).
The [2026-10-01 playback follow-up](docs/performance-playback-2026-10-01.md)
records the subsequent CPU reduction, browser comparison and remaining
qualification work.

## Install a release

[GitHub Releases](https://github.com/ViceVerse-cz/oxplay/releases) carry
**production** releases (`vX.Y.Z`, marked latest) and **nightly** prereleases
(`vX.Y.Z-nightly.YYYYMMDD.N`), built by CI from the tagged commit. They are
experimental builds; see [CI and releases](docs/ci-release.md#releases-nightly-and-production-channels).
The release recipes build the default native renderer and bundle its private
media stack on each platform. Historical stock-libmpv artifacts remain evidence
for the legacy comparison only. Physical Linux/Windows playback and the resource
targets remain unqualified.
Every release bundles pinned yt-dlp and Deno helpers and lists all assets in
`SHA256SUMS.txt`.

| Platform | Install |
| --- | --- |
| macOS (Apple Silicon) | `brew tap ViceVerse-cz/oxplay https://github.com/ViceVerse-cz/oxplay.git && brew install --cask oxplay`, or unzip `oxplay-<tag>-macOS-ARM64.zip` into `/Applications` |
| Windows (x86_64) | run `oxplay-<tag>-Windows-X64-Setup.exe` (per-user, no admin), or unzip `-Windows-X64.zip` |
| Ubuntu 24.04 | `sudo apt install ./oxplay-<tag>-Linux-ubuntu-24.04-oxplay_<version>_amd64.deb` |
| Fedora 44 | `sudo dnf install ./oxplay-<tag>-Linux-fedora-44-oxplay-<version>.x86_64.rpm` |
| Arch Linux | `sudo pacman -U ./oxplay-<tag>-Linux-arch-oxplay-<version>-x86_64.pkg.tar.zst` |
| Other Linux (x86_64, glibc ≥ 2.39) | `chmod +x oxplay-<tag>-Linux-X64.AppImage` and run it, or extract `-Linux-X64.tar.gz` and run `./oxplay` |

Unless Developer ID secrets are configured for the release, the macOS app is
ad-hoc signed only: run `xattr -dr com.apple.quarantine /Applications/Oxplay.app`
once. Windows and Linux binaries are unsigned. Signed apt/dnf/pacman
repositories are published to GitHub Pages only once a repository signing key
is configured ([details](docs/ci-release.md#optional-secrets-and-settings)).

## Build on the development Mac

Install Rust 1.98.1 (pinned in `rust-toolchain.toml`), Apple's Command Line Tools, and development packages
using Homebrew. A graphical macOS session is required to run the application:

```sh
brew install pkgconf cmake ffmpeg libass shaderc libarchive uchardet yt-dlp
python3 -m venv artifacts/native-media-tools
artifacts/native-media-tools/bin/python -m pip install meson==1.12.1 ninja==1.13.2
python3 scripts/native-media/macos.py --tools "$PWD/artifacts/native-media-tools/bin" --jobs 2
export OXPLAY_NATIVE_MPV_PREFIX="$PWD/artifacts/native-media/macos/prefix"
cargo build --locked --release --workspace
./target/release/oxplay
```

The script verifies pinned source archives and applies the reviewed patches,
installs into the private prefix, and records build provenance. The Rust build
requires `include/mpv/render_mtl.h` to declare `OXPLAY_NATIVE_RENDER_ABI 1` and
embeds the private library rpath. Installing Homebrew `mpv` alone cannot satisfy
this ABI. See [the complete Metal build and headless smoke](docs/native-media-macos.md).

The development host is Apple Silicon macOS 27.0. Linux Vulkan and Windows
D3D11/DX12 adapters have source and cross-target API checks, with native driver,
presentation, hardware-decoder and performance qualification still pending.
CI's Linux/Windows stock-libmpv jobs explicitly build the OpenGL comparison;
they do not validate those native adapters. See [platform matrix](docs/platform-matrix.md)
and [CI scope](docs/ci-release.md).

For the separate legacy OpenGL comparison:

```sh
brew install mpv
cargo build --locked --release --workspace --no-default-features
./target/release/oxplay --graphics-backend opengl
```

## Build on Linux (experimental)

The default requires a private Vulkan/VAAPI media build, matching FFmpeg/libass,
Vulkan and shader-compiler development dependencies, plus Winit/X11/Wayland
prerequisites. Follow [the native Linux recipe](docs/native-media-platforms.md#building),
set `OXPLAY_NATIVE_MPV_PREFIX` to its installed prefix, then run
`cargo build --locked --release --workspace`. The installed
`include/mpv/render_vk.h` must declare `OXPLAY_NATIVE_RENDER_ABI 1`.
Native X11 and Wayland playback remain unqualified on real Linux hosts.

## Build on Windows (experimental)

Target: `x86_64-pc-windows-msvc`. Install Rust 1.98.1 through rustup (the pinned
toolchain is selected automatically), the Visual Studio 2022 Build Tools with the
"Desktop development with C++" workload (MSVC linker and Windows SDK), and
[the native Windows media toolchain and dependencies](docs/native-media-platforms.md#building).
Build the pinned D3D11 media library with `scripts/native-media/windows.py`;
its SDK prefix must provide `include/mpv/render_d3d11.h` declaring
`OXPLAY_NATIVE_RENDER_ABI 1`, an import library, and the matching runtime DLLs.

```powershell
$env:OXPLAY_NATIVE_MPV_PREFIX = "C:\oxplay-native\sdk"
cargo build --locked --release --workspace
Copy-Item "$env:OXPLAY_NATIVE_MPV_PREFIX\*.dll" target\release\
.\target\release\oxplay.exe
```

The native renderer requires shared-texture/fence support and the same adapter
for D3D11 media and DX12 UI. It has not been runtime-qualified on a Windows host.
For the separate legacy OpenGL build, install [7-Zip](https://www.7-zip.org/),
download a stock libmpv development archive and point `MPV_DIR` at it. CI
uses the exact archive pinned in
[`scripts/ci/install-mpv-windows.sh`](scripts/ci/install-mpv-windows.sh)
(shinchiro's `mpv-dev-x86_64-…7z`; `bash scripts/ci/install-mpv-windows.sh DIR`
in Git Bash downloads and SHA-256-verifies it). It must contain
`include/mpv/client.h` (client API 2.5 or newer), `libmpv-2.dll` and either an
MSVC `mpv.lib` or `libmpv.dll.a`, which the MSVC linker accepts directly.

```powershell
$env:MPV_DIR = "C:\deps\mpv-dev"
cargo build --locked --release --workspace --no-default-features
Copy-Item "$env:MPV_DIR\libmpv-2.dll" target\release\
.\target\release\oxplay.exe --graphics-backend opengl
```

`cargo run` and `cargo test` find `libmpv-2.dll` automatically (the media build
script places a copy on Cargo's run/test path); a directly started `oxplay.exe`
needs the DLL beside it. Release builds are GUI applications without a console
window; command-line diagnostics started from a terminal still print there.
Helpers are taken from beside `oxplay.exe` first (`yt-dlp.exe`, `deno.exe`),
then from an absolute `PATH` entry (for example after `winget install yt-dlp.yt-dlp`
and `winget install DenoLand.Deno`), or from explicit `--yt-dlp`/`--deno` paths. yt-dlp runs in a
kill-on-close Job Object, including downloads; put `ffmpeg.exe` beside
`oxplay.exe` to merge qualities above the single-file format. Data lives under `%LOCALAPPDATA%\Oxplay`; remembered
account sessions use Credential Manager. The macOS `oxplay-dns` helper is not
used: the scoped media transport resolves in process, as on Linux. Browser
sign-in (use the session file import) and the Unix-only diagnostics are
unavailable on Windows. See the [platform matrix](docs/platform-matrix.md).

Slint runtime/build compiler use the identical upstream Git revision recorded in
[dependencies](docs/dependencies.md). Normal builds use `--locked`; upstream
updates must be reviewed and tested. No project API keys are needed. Native
libraries/helpers are installed development prerequisites, not bundled executables.
The legacy build requires **libmpv client API 2.5 or newer**, found through
`pkg-config` on Unix or `MPV_DIR` on Windows. Default native builds additionally
require the custom ABI-1 headers and matching private libraries. FFmpeg is
required for fixture generation. The inspected
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
launch unless you chose to remember an account session, which is verified once
at launch to sign you back in. History, autoplay, previews, telemetry, and background refresh are off.

```sh
./target/release/oxplay --local /absolute/path/video.mp4 --subtitle /absolute/path/subtitles.srt
./target/release/oxplay --yt-dlp /absolute/path/yt-dlp
./target/release/oxplay --url 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

The Mac default helper paths are `/opt/homebrew/bin/yt-dlp` and
`/opt/homebrew/bin/deno`. No helper, plugin, or runtime is downloaded at playback.
Explicit `--yt-dlp` and `--deno` paths support nonstandard installations.
The opt-in `--scoped-media` diagnostic on macOS requires the first-party
`oxplay-dns` executable beside `oxplay` (bundles place it in `Contents/Helpers`).
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
The account page explains session import and its risks before enabling it. On
macOS you can **Sign in with your browser**: pick one installed browser profile
(Chrome, Brave, Edge, Arc, Chromium, Vivaldi, Firefox or Safari) and Oxplay
imports only that profile's YouTube/Google sign-in cookies, then runs the same
identity verification and protected storage as the file import; Safari needs Full
Disk Access, and the manual Netscape file import stays available as a fallback.
Account playlist videos have a separate **Play with account**
action backed by revocable scoped media transport. This path is implemented but
real-account playback remains unverified; account captions/comments and private
local storage are unavailable. Guest playback never escalates automatically. See [privacy](docs/privacy.md),
[account status](docs/account-provider.md), and [provider limits](docs/provider.md).

Known ad/promoted metadata shapes are filtered, and native playback uses the
resolved content stream without an advertising web player. Live guest/authenticated
ad-suppression qualification is unfinished; this is not a universal ad-free promise.
No account session, Premium account, or fabricated blocked-ad count is used as proof.

## Validation

Keep the native prefix from the source-build instructions selected for the
default Cargo checks. The headless ABI/content check and its hardware requirements
are documented in [native Metal qualification](docs/native-media-macos.md#qualification).
For a stock-libmpv comparison, add `--no-default-features` to Cargo build,
check, Clippy and test commands.

```sh
cargo fmt --all -- --check
cargo check --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
scripts/generate-fixture.sh
./target/release/oxplay --local artifacts/local-1080p60.mp4 --subtitle artifacts/local.srt --smoke-test
python3 scripts/measure.py --output artifacts/playback.json -- ./target/release/oxplay --local artifacts/local-1080p60.mp4 --quit-after 85
```

`--smoke-test` is explicit fixture mode; its synthetic catalog rows are labeled.
It exercises pause, seek, resize, fullscreen, subtitles, minimize/restore, pointer movement, and
catalog notification assertions. It is not a perceptual A/V, screen-reader, or
full platform test. [Progress](docs/progress.md), [video integration](docs/video-integration.md)
and [performance](docs/performance.md) distinguish executed checks from pending gates.

Source license: GPL-3.0-or-later. Slint uses its GPL-3.0-only option. Combined binary
distribution and native dependencies require the audit in [licensing](docs/licensing.md).
UI icons are unmodified [Lucide assets](crates/app/ui/icons/SOURCE.md), with their
ISC and retained Feather MIT notices. Release [packages](docs/packaging.md) are
not yet portable-qualified: clean-machine testing, complete native notices and
corresponding source for bundled libmpv/FFmpeg builds remain release work.

[CI and releases](docs/ci-release.md) describe the macOS/Linux/Windows CI matrix and the manual
nightly/production release workflow, which publishes macOS ARM64, Windows
x86_64 and Linux x86_64 (deb, rpm, Arch, AppImage, tarball) builds with the
exact source archive. Signing is optional; portability and licensing gates
remain open.
