# Dependency adoption

Bootstrap on 2026-09-29, macOS 27.0 (26A428), Apple M1 / 16 GiB.
`git ls-remote --symref https://github.com/slint-ui/slint.git HEAD` returned
`refs/heads/master` and `cf3b07d4917e6759a63b0c03913a2594ec653414`.
Fetched `.upstream/slint` and verified HEAD. Local source cache is ignored.

Runtime and build compiler use that identical explicit Git revision. Reviewed
`api/rs/slint/Cargo.toml`, Rust integration, `examples/opengl_texture`, both
GStreamer sinks and their build configuration, borrowed-texture safety contract,
backend selector, rendering notifier, and Winit window accessor.

Selected: std, compat-1-18 (mandatory compatibility API at this revision),
backend-winit (enables X11/Wayland support when targeting Linux), renderer-femtovg (OpenGL),
accessibility, unstable-winit-030 (occlusion/window events). Defaults disabled.
No Qt, tray, software renderer, Skia, WGPU, runtime interpreter, live preview or
system-testing server is requested. slint-build defaults are also disabled, with
only compat-1-18 selected; compiler software-renderer asset embedding is unneeded.
Actual target graph must also be audited;
Cargo.lock includes packages for other targets and is not a shipped inventory.
The selected Linux X11 and Wayland feature graph now passes strict Clippy,
automated Rust tests and release compilation on Ubuntu 24.04 CI (source `036da68`). Neither native
presentation backend has been run; compilation is not runtime support evidence.

Toolchain: rustc 1.98.1 (48a229cea 2026-09-01), cargo 1.98.1
(797e8a9bc 2026-08-05), Homebrew. Build using `cargo build --locked` after initial
lock generation. Upstream advances require deliberate revision changes and tests.
The repository now pins this toolchain, including rustfmt and Clippy, in
`rust-toolchain.toml`; CI consumes that file. The Linux CI libmpv source and
native package inventory policy are described in [CI and releases](ci-release.md).

Native development prerequisites installed by Homebrew: mpv 0.41.0_10,
yt-dlp 2026.8.19_1, FFmpeg 9.0.2, Deno 2.9.7. These are development dependencies,
not an audited redistribution bundle. libmpv is linked in-process using pkg-config.
`crates/media/build.rs` requires the `mpv.pc` **client API version >=2.5** because
the FFI calls `mpv_get_time_ns`. The inspected package reports `2.5.0`; this is
distinct from the mpv executable version `0.41.0`. A source build needs the
`pkg-config` command (`brew install pkgconf` on this Mac), Apple Command Line
Tools, and the libmpv development headers/libraries. The helper's installed
Python version is 3.14.7 and `yt-dlp-ejs` is 0.8.0. Runtime Deno is explicitly
selected at `/opt/homebrew/bin/deno` on this development Mac; the UI currently
offers no override for that path. The yt-dlp executable path can be overridden.
See licensing.md for distribution limitations. Source build requires declared
native packages; application does not download or update executables.

The in-process `oxplay-storage` workspace crate uses rusqlite 0.40.2 with
defaults disabled and `bundled,backup` enabled. Its locked libsqlite3-sys 0.38.2
contains SQLite 3.53.2. No system SQLite prerequisite is needed for this crate.
The application now depends on it and owns SQLite on a dedicated bounded
worker; shared-UI integration evidence belongs in progress.md. Protected
session storage uses chacha20poly1305 0.10.1, zeroize 1.9.0, getrandom 0.4.3,
and macOS security-framework 3.7.0 / security-framework-sys 2.17.0. Other OS
credential stores remain unimplemented; a dependency declaration does not
establish platform support.

The account adapter uses reqwest 0.12 with rustls and a current-thread Tokio
runtime on its worker, sha1 for the provider's session-authorization protocol,
and httpdate 1.0.3 for server cooldown hints. No persistent JavaScript service
is part of account access. The app adds rfd 0.17.2 with defaults disabled and
`xdg-portal,wayland` for native file selection, plus webbrowser 1.2.4 for explicit
system-browser opening. These adapters do not replace the shared compiled UI.

The bootstrap `cargo tree --locked -p oxplay -e features` was inspected: no Qt, Skia, software
renderer, tray or runtime preview is in the selected macOS application graph.
Reaudit the full selected graph after account/file-dialog additions before
packaging; the bootstrap inspection is not an audit of those later additions.
Cargo.lock was committed in initial application commit e945b3e.

## Account, storage, and shared-UI integration (2026-09-29)

The Slint git revision and production feature selection above remain unchanged.
The lockfile was deliberately regenerated after adding these dependencies; subsequent
checks/builds use `--locked`. No existing locked package version changed during this
addition (new packages/features were added).

- Anonymous thumbnail HTTP and fixed-origin account HTTP: reqwest 0.12.28,
  default features disabled, Rustls TLS, bounded streaming responses; Tokio 1.53.1
  current-thread worker runtimes. No persistent service or JavaScript account stack.
- Thumbnail decoding: image 0.25.10, only JPEG/PNG/WebP. Video still uses libmpv GPU
  presentation; thumbnail pixel buffers do not carry media frames.
- Session protection: chacha20poly1305 0.10.1 (XChaCha20-Poly1305), zeroize 1.9.0,
  getrandom 0.4.3, macOS security-framework 3.7.0/security-framework-sys 2.17.0.
  OS Keychain stores a small encryption key, not an arbitrarily large cookie jar.
- Account protocol SHA-1 session proof: sha1 0.10.7; HTTP Retry-After parsing:
  httpdate 1.0.3. These do not introduce OAuth credentials or a token service.
- Native file selection only: rfd 0.17.2, default features disabled,
  `xdg-portal,wayland`; no GTK application frontend. macOS uses NSOpenPanel through
  its asynchronous API. Other native picker targets have not been run here.
- Explicit system-browser links: webbrowser 1.2.4. No embedded web content.

## Browser-session sign-in (2026-09-30)

The account adapter (`oxplay-youtube`) gained an explicit, user-selected
"Sign in with your browser" path that reads ONLY YouTube/Google session cookies
from one chosen installed browser profile on macOS. It adds, all permissively
licensed (MIT OR Apache-2.0 unless noted) and all already resolvable from the
locked registry:

- `rusqlite 0.40.2` (MIT) with defaults disabled and only `bundled` enabled, to
  read Chromium (`Cookies`) and Firefox (`cookies.sqlite`) databases. Feature
  unification with `oxplay-storage` means the single locked `libsqlite3-sys
  0.38.2` (SQLite 3.53.2) is built once; no system SQLite is required.
- `aes 0.9.3`, `cbc 0.2.1` (features `alloc,block-padding`) and `pbkdf2 0.12.2`
  (defaults off, `hmac`) to decrypt Chromium "v10" values: AES-128-CBC with a
  key from `PBKDF2-HMAC-SHA1(password, "saltysalt", 1003)`. `sha1 0.10` (already
  present) provides the PRF. Adopting aes 0.9 pulled transitive `cipher 0.5.2`,
  `block-padding 0.4.2`, `inout 0.2.2`, `crypto-common 0.2.2`,
  `hybrid-array 0.4.15`, `cpubits 0.1.1` and `cpufeatures 0.3.1`; pbkdf2 pulled
  `hmac 0.12.1` (and enabled `digest`'s optional `subtle` dependency, already
  locked). No previously locked version changed.
- `tempfile 3.27` (already locked; MIT OR Apache-2.0) to copy a locked cookie
  database and its `-wal`/`-shm` into a private temp directory before opening.
- macOS `security-framework 3.7` (already locked) to read the
  `<Vendor> Safe Storage` Keychain generic password. macOS shows its
  own access prompt; denial is a typed permission error, never a silent failure.

Decrypted values and derived keys are held in zeroizing buffers. Safari uses no
new dependency: its `Cookies.binarycookies` file is parsed directly (Full Disk
Access required; a permission error is surfaced with guidance). Non-macOS targets
compile and report the feature as not supported. The account/browser tests use
only synthetic fixture databases/files; no real browser, profile or Keychain is
touched. Reaudit the selected macOS application graph before packaging.

Local upstream Slint sources were used to verify compiled model APIs, ListView
`content-y`/`visible-height`, `spawn_local` cancellation, and Window::take_snapshot.
`--snapshot PATH` performs exactly one diagnostic pixel readback at 15 seconds and
encodes it on a worker. It is excluded from performance runs and is never a video
presentation mechanism.

## Guest catalog and upstream icons

Typed guest catalog browsing and its shared Slint adapter add no Cargo dependencies.
They use the same pinned Slint build/runtime and installed supervised yt-dlp described
above. Search filter serialization was reviewed against YouTube.js source revision
`bad89d2657e88f907011655f199fba9fb615c339`; that repository is a protocol reference,
not a runtime or bundled JavaScript service. Public channel avatars add two exact
anonymous thumbnail origins, `yt3.ggpht.com` and `yt3.googleusercontent.com`, under
the existing redirect-disabled, bounded Rust image fetcher.

All 22 SVG icons come unchanged from the official `lucide-icons/lucide` repository,
whose verified default branch was `main`, resolved to
`66d8f9fc394b8530377e5f6112f0b8908ba01280`. No Lucide package or runtime is installed;
Slint compiles the vendored SVG assets. Source URLs, the complete ISC and retained
Feather MIT notices, and SHA-256 hashes are in `crates/app/ui/icons/`. All 23
asset/license manifest entries were verified, and every SVG matches the upstream
file byte-for-byte. See [asset provenance](../crates/app/ui/icons/SOURCE.md).
The `e97ad3f` UI checkpoint reverified all 22 local asset/license manifest hashes;
the three navigation additions use the same pinned upstream revision. The layout
and palette change adds no framework or icon runtime dependency.

## Scoped media transport experiment

The new `oxplay-network` workspace crate carries compressed media ranges, never
decoded frames. It reuses reqwest 0.12.28 (Rustls/stream, defaults off) and Tokio
1.53.1. On non-macOS targets, Hickory resolver/net/proto are locked to 0.26.3; the
direct resolver dependency has only `system-config,tokio`, with defaults disabled. The published
0.26.3 source/manifests and its MIT/Apache-2.0 notices were inspected before use.
Its [resolver API](https://docs.rs/hickory-resolver/0.26.3/hickory_resolver/struct.Resolver.html)
and the installed reqwest DNS trait source were checked directly.

On macOS the implementation instead uses the installed SDK's native
`DNSServiceGetAddrInfo`/libSystem API, Tokio readiness and the already locked
libc crate. Hickory rejected a scoped IPv6 nameserver in this host's configuration;
the native path delegates scoped/VPN selection to the system daemon without
discarding that configuration. No Hickory resolver is compiled into the macOS
transport. Native reference ownership, descriptor teardown and callback types
were checked against macOS 27 `dns_sd.h`; its explicit resolver/cleanup smoke
passed. Native calls now run in the first-party `oxplay-dns` workspace executable,
with no new third-party dependency. It is built from the same source/lockfile and
included in the bundle's closure, signing, source archive and inventory. A bounded
supervisor and independent helper watchdog cover cancellation and userspace stalls
in synchronous initialization. System scheduling/reaping, native partial IPC and
scoped/per-app VPN behavior remain qualification limits described in
[media network integration](media-network.md).

This addition replaces a background OS getaddrinfo task with asynchronous DNS
waiting and a validated address set handed directly to reqwest's connector. It
does not silently switch to a public resolver or claim encrypted DNS, strict
proxy coverage, or confinement of unrelated helper traffic. The deliberate lock
update added 14 transitive packages; subsequent checks use `--locked`. The new
dependency/resource cost and complete distribution notices require re-audit
before this transport can qualify for release.

## 2026-09-29 motion/library/presenter checkpoint

Frozen source `83a07b4b493a7d411fc21fdbc55daf529596be7e` produced release SHA256
`c2ff7d1c0f93145713e2de039e9aa6b574051afcba5d07ed331650c025568398` (C2).
All 115 application build-input hashes were independently compared with the commit
after the locked release build. Slint revision, production features, toolchain,
Cargo.lock and native dependencies remain as recorded above; this is not an
independent rebuild or native dependency reproducibility proof. Formatting,
243 Rust tests (three external tests ignored), strict all-target workspace
Clippy and 63 Python tests passed. [Captured input hashes and native evidence](evidence/2026-09-29-motion-library-summary.json)
identify this checkpoint. The earlier 9c70e8e/541f4c corrected-clock comparison
has its own 114-input capture and is retained separately.

Native functional scope and the stable-target ABBA are recorded in
[progress.md](progress.md) and [stable-video-target.md](stable-video-target.md).
Neither texture nor clock experiment is enabled by default. Newer working-tree
settings/soak changes are outside this frozen validation. The historical bundle
source-coverage audit is separate and does not requalify that bundle against C2;
no new bundle or clean-machine result is asserted.


## 2026-09-29 preference/soak-source checkpoint

Source `f6dea1361cdd59d2f7a26315a6002e5c2a25fc85` produced release SHA256
`b6d39791e3535466abaeb9b7cce2a737577af6c2b11d6efc126c75129f8435e3`.
All 119 captured build-input hashes independently match the commit. Slint/native
dependencies and the locked source policy remain as recorded above. Central
validation passed 261 Rust tests with three ignored, 63 Python tests, formatting,
strict workspace all-target Clippy and the locked release build. No independent
native-dependency rebuild or new bundle qualification is asserted.

[Preference evidence](evidence/2026-09-29-preferences-native-audit.json) records two
successful offline native write/restart/clear phases, plus a failed local startup
focus assertion (exit 101, 3.210 seconds) after finite motion passed. This source
contains a soak harness but has no completed soak qualification. Later focus
fixes must not inherit this binary's hash or test counts. The 83a07b4/C2 performance
series remains historical evidence for its own source and geometry.


## 2026-09-29 local-startup correction

Frozen `ee59eaa47289382b6784547e8bece1b38e5e60a5` produced release SHA256
`464fdcf1432f1425698cf2892f2fd7f3e942c4e132dfb176c5c45bcb5582b0a8`.
All 119 captured source hashes independently match. Central validation passed
261 Rust tests, three ignored, formatting, strict workspace all-target Clippy
and the locked release build; no new dependency or framework revision is claimed.
The local startup/focus lifecycle passed in 21.071 seconds. Intermediate debug
binaries and the previous f6dea13 failure retain their separate identities in
[focus evidence](evidence/2026-09-29-startup-focus-summary.json). That hour subsequently completed; its RSS growth leaves leak qualification
open. See [local soak](soak-local.md). No bundle requalification is implied.

The `0f311dd` slice adds the unchanged upstream `volume-x.svg` for observed mute
state. Central verification checked all 23 asset/license hashes and byte-compared
all 22 SVGs with the exact official checkout above. No icon was redrawn and the
license route is unchanged. The source builds and first native results are
recorded in [UI validation](ui-validation.md); subsequent focus correction
`ca21de9` passed 114 app tests and strict workspace/all-target Clippy.

## Native-child diagnostic build glue

The macOS experiment adds direct build dependency `cc = "=1.5.1"` to
`oxplay-media`, and exact-fixture hashing adds app dependency `ring = "0.17"`
(resolved 0.17.14). Both versions were already locked; the only Cargo.lock changes
are these two direct dependency edges. macOS-filtered offline metadata and the
locked workspace tests compile successfully. An initial unfiltered offline
metadata attempt required uncached Windows-only `ipconfig` and failed; no
dependency version was advanced or substituted. Runtime qualification is separate.

On macOS only, `build.rs` compiles `native_child.m` with Objective-C ARC and links
Apple AppKit, QuartzCore and OpenGL frameworks. The glue contains only the native
video surface and context integration, not another application UI. Reviewed local
headers identify SDK `macosx27.0`; its OpenGL APIs remain present but deprecated.
The existing Slint revision, GPL distribution route and default presenter remain
unchanged. See the [unqualified native-child implementation](experiments/native-video-child.md).

The guest artwork cache uses the already locked `image`, `libc`, SQLite and
Tokio dependencies. No framework revision, production feature combination or
license route changed for this slice. Cache path/platform limitations and the
schema-v6 migration are recorded in [artwork-cache.md](artwork-cache.md).

Playlist keyboard behavior was checked against the same pinned Slint source:
`internal/compiler/widgets/common/lineedit-base.slint` exposes read-only, focus,
selection and key callbacks; `internal/core/api.rs` provides native-window key
dispatch. The finite diagnostic exercises Slint input directly and does not
qualify Winit/OS input or screen readers. The controlled `ConfirmedChoice` widget
uses an acknowledged-index change handler because `Property::set` in
`internal/core/properties.rs` removes ordinary bindings, and
`internal/compiler/widgets/common/combobox-base.slint` assigns its index before
emitting `selected`. A one-time rollback assignment is therefore insufficient
to follow later acknowledgments. Both the index and value must be restored
synchronously because the base widget reverse-resolves value changes to an index.
Confirmed history toggles likewise mirror asynchronous checked acknowledgments.
No dependency, feature or license selection
changed for this slice.

Inline transport and borderless PiP use the existing locked Slint/Winit stack.
The pinned Winit adapter reapplies native decoration state from `Window.no-frame`;
compact mode therefore binds that property as well as setting the native hint.
Winit's native inner/outer frame queries support the functional geometry checks.
No framework revision, Cargo feature, dependency or license route changed.

## Development UI regression backend

The app now has a **dev-only** `i-slint-backend-testing` dependency pinned to the
same `cf3b07d4917e6759a63b0c03913a2594ec653414` framework revision, with defaults
disabled and no optional features. Its actual manifest, testing API, mock backend,
and fluent ComboBox implementation were inspected in the pinned checkout.
The deliberate offline lock update added only this package; no existing package
version changed. Subsequent builds/tests use `--locked`.

The tests instantiate the shared compiled `App`, inject Slint keys and observe
accessible control values with synthetic callback acknowledgements. They use the
mock backend with no rasterizer, event-loop service, MCP server, network worker
or runtime UI interpreter. Development builds emit Slint element metadata for
these tests; release builds omit it, and this specific integration test target is
restricted to debug-assertion builds. Native release checks remain separate.

`cargo tree -p oxplay --locked -e normal,build` was inspected after the addition:
the testing backend is absent, as are Qt, Skia, the software renderer and the
interpreter. This is selected dependency-graph evidence, not a binary-size or
resource measurement. Runtime Winit/FemtoVG/accessibility features are unchanged.

## Local media and navigation feature batch

The picker, playlist organization, timestamp links, chapter list and time dialog
reuse the locked rfd, SQLite, Slint and media dependencies. No dependency,
framework revision, Cargo feature or lockfile change is introduced. The new
local per-entry options were source-reviewed against official mpv v0.41.0 and
FFmpeg n8.0; see [local media policy](local-media-policy.md). This does not advance
or replace the installed native build inventory or its pending qualifications.

The custom-window-chrome slice uses existing locked Slint/Winit APIs, including
WindowMoveArea, resize-border-width, window attributes and the Winit blur request.
No Cargo dependency, feature, revision or lockfile changes. The exact source
contracts and platform restrictions are recorded in [window appearance](window-appearance.md).

The watch-layout correction keeps the same locked framework/runtime revision and
features. Native system decorations replace the custom titlebar. Theatre mode
adds unchanged `rectangle-horizontal.svg` from the already-pinned Lucide
revision, with its checksum and existing license retained. Channel artwork uses
the existing yt-dlp/HTTP/image dependencies; no dependency or lockfile update.

The integrated macOS header adds direct target-only references to already-locked
`objc2 = 0.6.4` and `objc2-app-kit = 0.3.2` (default features off; std,
NSResponder, NSView, NSWindow). Public AppKit methods restore full-size content
styling after Winit restores PiP decorations. The lockfile adds these two edges
to the application; no package version, Slint revision or renderer feature moves.
No native titlebar subview hierarchy is retained or rewritten.

The subsequent 48px header/whole-window translucency change adds no dependency,
Cargo feature, framework revision or lockfile update. It reuses the same locked
Slint/Winit and limited macOS AppKit integration. Winit 0.30.13's macOS native
blur request calls private `CGSSetWindowBackgroundBlurRadius` with radius 80
when enabled; it exposes no success/active-state getter. Default requests and
alpha shared tokens are capability-gated appearance choices, not new runtime
qualification. See [window appearance](window-appearance.md).

## Header/comments/history presentation correction

Slint's exact revision, Rust toolchain and Cargo.lock are unchanged. macOS-only
objc2-app-kit 0.3.2 now additionally enables `NSButton,NSControl` to access the real
standard traffic lights; no new crate/version or native UI framework is added.
The refreshed comments action uses unchanged official Lucide `refresh-cw.svg`
from revision `66d8f9fc394b8530377e5f6112f0b8908ba01280`; its hash/source are added
alongside the existing ISC/MIT notices. Shared English count/calendar formatting
adds no runtime dependency. See [comments](comments.md), [portraits](channel-avatars.md),
[history](local-library-ui.md) and [native header](window-appearance.md).

## Video-card clipboard (2026-10-01)

The card context menu writes text and thumbnail pixels to the system clipboard
through a direct application dependency on `arboard = "=3.6.1"`, default features
off, `image-data` on. arboard 3.6.1 was already locked through Slint's Winit
backend (which uses it for text with default features off); the exact version is
unchanged. `image-data` unifies onto that one package and adds: on macOS,
`objc2-core-graphics`/`objc2-core-foundation` edges (already locked) and image's
`tiff` codec; on Linux the `png` codec; on Windows `png`/`bmp`. The deliberate
lock update added only `tiff 0.11.3` and `fax 0.2.7` (both from the local
registry cache) plus these edges; no existing package version, Slint revision or
renderer feature moved. An offline unfiltered `cargo metadata` still stops at the
uncached Windows-only `ipconfig`, as recorded above. See
[sharing](sharing.md#video-card-menu) for behavior and limits.
