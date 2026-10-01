# Platform qualification

macOS integrates its real native traffic lights into the shared 48px custom
header through a transparent titlebar/full-size content view. Windows/Linux
retain native-frame fallback; their integrated-header adapters are unvalidated.
PiP temporarily removes decorations. Theatre mode and watch/library/loading
layouts remain shared Slint UI. At the user's explicit request, whole-window
translucency and blur are requested by default, gated by backend capabilities
and with session-local opt-out. The canvas/shared surfaces use neutral alpha;
decoded video stays opaque. Blur is exposed only on macOS, as a public AppKit
`NSVisualEffectView` clipped to the rounded system frame (no success getter);
Windows/X11/native Wayland blur remains unavailable. These changes have no new
native visual qualification. Historical capture attempts encountered native
display-clock error `-6661`; older results below retain their exact source scope.
See [appearance capabilities](window-appearance.md).

The latest local-file picker, playlist organization, timestamp links, chapter
navigation and Jump to time additions have source/compile validation only. No
new native picker, popup/keyboard, malformed-file or account test has run on any
platform for this batch; older evidence below retains its exact source scope.
See [progress](progress.md).

All targets are experimental. A native compile/run is not release support.

## Windows port (x86_64-pc-windows-msvc)

Windows now compiles, lints and runs the automated test suite in a hosted
`windows-latest` CI job (see [CI](ci-release.md) and the evidence below). It
links libmpv from `MPV_DIR`, an extracted, SHA-256-pinned shinchiro
`mpv-dev-x86_64` archive (self-contained `libmpv-2.dll` with static FFmpeg;
[build inputs](build-inputs.md#windows-libmpv-input)). The Winit/FemtoVG shell
uses a WGL OpenGL context and the same shared OpenGL presenter as Linux;
macOS-only pieces (display clock, native child, AppKit header integration) stay
behind `cfg(target_os = "macos")`, so frames are scheduled by mpv's render
update callback. Windows keeps native window decorations: the OS frame owns
dragging, resizing and the minimize/maximize/close buttons.

Platform adapters implemented for Windows:

- yt-dlp supervision uses one Job Object per run with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The helper starts suspended, joins the
  job, then resumes, so all descendants are born inside it. Timeout,
  cancellation, session change, output bounds and success terminate the whole
  tree (`TerminateJobObject`); an Oxplay crash closes the job and kills it too.
  CI tests cover bounded capture, timeout, cancellation and a detached
  grandchild that must not survive. The former fail-closed gate is removed.
  Each run gets a private TEMP, removed once the job reports no live process:
  the one-file `yt-dlp.exe` (PyInstaller) unpacks itself there on every run and
  would otherwise leak `_MEI*` directories when a run is cancelled. The
  one-directory `yt-dlp_win.zip` distribution avoids that per-run unpacking
  and is the better choice for packages.
- Helpers resolve to `yt-dlp.exe`/`deno.exe` beside `oxplay.exe`, then one
  absolute `PATH` entry (development), or explicit `--yt-dlp`/`--deno`.
- Remembered account sessions: the per-profile key is a Credential Manager
  generic credential (DPAPI-protected, local machine persistence), separate
  from the XChaCha20Poly1305 envelope. New private directories get a protected
  owner-only DACL; reparse points (symlinks/junctions) are rejected. Session
  file import, ephemeral extractor cookie jars and the caption cache use the
  same checks. An ignored synthetic Credential Manager roundtrip exists
  (`windows_credential_manager_synthetic_roundtrip`).
- Display sleep is prevented only during observed video playback
  (`SetThreadExecutionState`), mirroring the macOS assertion.
- Media DNS uses the in-process system resolver as on Linux. The macOS
  `oxplay-dns` helper is not used; its Windows build is an inert stub that
  exits with status 2 and need not be packaged.
- Release builds use the GUI subsystem (no console window); diagnostics
  reattach to a launching terminal, and startup errors show a message box.

Not available on Windows: browser sign-in (Chromium on Windows uses DPAPI and
app-bound encryption that Oxplay does not implement; the UI shows the existing
"not supported on this platform yet" message and the file import remains),
the optional guest artwork disk cache (fails closed), the Unix-only smoke
diagnostics, and blur. Not verified on any Windows machine: opening the native
window, WGL presentation, hardware decoding (`hwdec=auto-safe` may select a
copy-back decoder), audio output, HTTPS media trust (FFmpeg uses OpenSSL with
the Windows certificate store), DPI/multi-monitor behavior, translucency,
real yt-dlp/Deno execution, Credential Manager on a user desktop, and
accessibility. CI never opens a window.

Evidence: [CI run 36793555368](https://github.com/ViceVerse-cz/oxplay/actions/runs/36793555368)
for source `2d3a68e` passed all three jobs. On `windows-latest` (MSVC) it passed
formatting, strict all-target Clippy, 577 Rust tests (five explicitly ignored),
the synthetic Credential Manager roundtrip and the locked release workspace
build, linking the pinned archive's `libmpv-2.dll` (SHA-256
`0d5b9dbe…7254e2e`). The 64 media tests initialized the real libmpv with the
production option set and null outputs; the Job Object, private-directory,
caption-cache and cookie-jar tests ran against the real Windows APIs.

The local-first Home slice passed debug/release functional checks on the same
macOS host: bounded local pagination, unchanged refresh, navigation and committed
collection mutations. Dark1000×720 and light760×600 captures were inspected.
This is local UI/storage evidence, not a new platform or resource qualification.
See [Home evidence](local-home.md).

The current [PiP slice](picture-in-picture.md) passed debug and release native
functional tests on Apple M1/macOS 27.0: P entry, pause, seek, resume, resize,
Escape and close-request restoration preserved the same window/media/presenter.
An owned-window CoreGraphics query verified floating layer 3 and restored layer 0.
Windows and X11 PiP remain unvalidated; native Wayland and the experimental
native-child presenter explicitly disable PiP. The account page was visually
checked in guest mode; this does not validate a live Google/YouTube connection.
No resource measurements were added.

The [CI run for source 42c937e](https://github.com/ViceVerse-cz/yt/actions/runs/36585092789)
passed macOS ARM64 formatting, strict Clippy, all 395 Rust tests, 163 Python tests
and release compilation on a hosted macOS 26 runner. The Ubuntu 24.04 job passed
formatting, strict Clippy, 377 Rust tests, 163 Python tests and release compilation
with both Linux backend features enabled. CI uses null audio/video outputs
in media unit tests and never opens a native desktop window. It provides no
new X11/Wayland/macOS presentation or hardware-decoder qualification.

The latest d78332c macOS release passed genuine guest local Save (eight stages),
related-video keyboard navigation (16 stages) and the restricted native-child
lifecycle (16 stages). Compact Save and full-message dialogs, fullscreen controls
and resizing were visually checked. These functional results do not change the
support status below. The a45ff676 matched default/native/native/default resource
comparison measured50.658/46.437/43.568/48.784% CPU means: every run missed the25%
target and the first default run exceeded the50% ceiling. See
[current progress](progress.md) and [resource scope](performance.md).

The macOS [local hour-long soak](soak-local.md) completed 60 cycles, 12 loads,
120 observed playing checkpoints, and clean bounded shutdown. RSS still grew
(four of twenty matched-minute comparisons exceeded 10%), so this is a local
functional result and does not pass the complete soak or resource gate. No
equivalent Windows, X11 or native Wayland result exists.

| Target | Build / native shell | Embedded media | Hardware evidence | Release status |
|---|---|---|---|---|
| macOS 27.0 arm64, Apple M1 | Debug/release compiled and native Winit/FemtoVG window run | Local and public YouTube H.264 video advance in shared Slint window; scripted pause/seek/resize/fullscreen and teardown exercised | `hwdec-current=videotoolbox` for H.264 input at 1920×1080/60 fps; OpenGL 4.1 Metal - 91.7 observed; bounded native display-clock path has zero warm VO drops in 60-second sample; A/V timing unqualified | Unqualified: repeatable playback CPU qualification remains open; default presenter still misses the ceiling in repeated measurements, and the restricted native-child candidate misses the target; sound/sync perception, accessibility, real-account and security gates pending |
| Windows x86_64 (MSVC) | Debug/release compilation, strict Clippy and automated tests pass in `windows-latest` CI; native shell not run | No native presentation test | None | Experimental; Job Object helper supervision implemented and tested with synthetic helpers |
| Linux / X11 | Release compilation and automated tests pass in Ubuntu 24.04 CI; native shell not run | No native presentation test | None | Experimental |
| Linux / native Wayland | Same CI build enables Wayland; native shell not run | No native presentation test | None | Experimental; XWayland is not a substitute |

The same compiled `.slint` UI is used. Only one initial desktop OpenGL presenter
exists, with a macOS-only native display-clock adapter for this context. No platform-specific app frontend, CPU video upload fallback, or browser
player is bundled. Missing graphics capability returns a visible error.

macOS display: built-in 2560×1600 Retina; screenshot shows an approximately
1320-pixel-wide redesigned logical window; initial baseline video target was 1384×778 physical pixels; the later C2
layout used 1328×747. Geometry must be associated with each individual run. The SDL baseline reported mode 1440×900 at 60 Hz, separately from
System Information’s panel resolution. Energy and screen-reader qualification remain open. Resource runs used AC power with low-power mode off. Sound output and A/V
synchronization require an actual
listening test; progressing mpv audio timing does not prove audible quality.

Actual public playback also observed Opus 48 kHz audio decoding. The new local 1080p60 run retained zero warm VO drops but exceeded the CPU
ceiling; the public path needs fresh qualification with this adapter.
API/screen-reader/A-V-perception and account tests remain open.

The H.264 hardware claim also uses the linked FFmpeg 9.0.2 decoder's mandatory
hardware-session flag. It is not generalized to codecs whose VideoToolbox paths
permit internal software fallback; see video-integration.md. Verified minimized
idle measured 111.6 MiB mean RSS and 0.0154% mean CPU on this host, while the
optimized playback gate remains open on resource/pacing repeatability and A/V sync.
The initial display-clock playback sample averaged 185.77 MiB RSS and 52.79% CPU, including
a newly launched decoder service; the same-engine standalone baseline averaged
163.46 MiB and 44.59%. A newer uncached control with 30 labeled related text rows measured 186.97 MiB
and 47.73% mean CPU, with one warm VO drop. Its cached comparison became unstable
late in the run and is rejected despite lower mean CPU. Caching is opt-in.
The historical `e1725df` phase-preserving clock/search-cache comparison completed
four warm 60-second intervals with zero additional VO/decoder drops, but CPU
means were 51.46/52.07/51.77/51.53% of one logical core. Every run exceeded the
50% release ceiling and 25% target. Mean RSS was 184.91/185.34/185.99/184.45 MiB.
The matching phase-continuous standalone reference measured 39.42% mean CPU;
run/host differences prevent interpreting subtraction as exact UI cost. The
30 related rows had labeled empty thumbnails, not a settled thumbnail library.
Both search-only and whole-chrome caching remain off by default. See
[search-cache.md](search-cache.md), [standalone-baseline.md](standalone-baseline.md)
and [performance.md](performance.md) for hashes and measurement limits.

Native clock creation requires an active display. An initial asleep display was
observed to fail setup (-6661); a bounded diagnostic wake made the same lifecycle
pass. The current media assertion prevents idle display sleep only during
observed playback and releases when paused/hidden/ended or destroyed. Native pmset checks confirmed its presence during Playing and absence while
hidden/paused and after teardown. A controlled initially-asleep launch recovered
on a native wake event and began VideoToolbox playback without restarting; see
video-integration.md for the exact scope. Multi-display moves, forced sleep,
GPU/context loss, HDR/4K and long-duration stability are not qualified.

Recent macOS functional additions include genuine selected-only guest captions
with exact-file observation, Off/cached reselection and paused quality reattachment;
a once-only guest expiry diagnostic preserved a fresh position with controls
hidden. The experimental in-process HTTP transport also reached native hardware
video/audio and clean exit, but its startup latency and whole-process network
qualification remain open. The opt-in OpenGL timestamp diagnostic reports unsupported capability on this
setup. The separately requested UI-only TIME_ELAPSED diagnostic did collect
1,523 intervals averaging 1.392 ms in a mixed 20-second lifecycle run. It excludes
mpv rendering and presentation, is not GPU utilization or steady-playback
acceptance, and does not supply the unsupported timestamp measurements.
See [captions](captions.md), [stream replacement](stream-refresh.md),
[media transport](media-network.md) and [GPU timing](gpu-timing.md).

The `b1f4074` macOS development bundle was built with tool-verified source
association and bundled Python/yt-dlp/EJS/Deno, native dependencies and the
immutable public CA resource. Its 70-second guest caption lifecycle passed
without helper overrides under an isolated environment: native selection,
Off/cached reselection and paused quality reattachment, followed by private
caption cleanup and no surviving tracked processes. A direct-app loader repeat
observed hardware H.264/audio and only bundle/system library paths. An initially
sleeping display still prevented a separate probe from creating its clock;
bounded wake made the repeat pass. These are development-host functional
checks, not clean-machine, performance, accessibility, subtitle-composition or
account qualification. The bundle remains experimental; see
[packaging evidence](packaging.md) for hashes, retained failed probes and limits.

The newer `bc2e53f` bundle includes the first-party transient `oxplay-dns`
executable. Both binaries were built from a clean detached checkpoint with
`--build`; their original hashes matched the main-checkout release inputs.
All five offline helper probes, deep strict ad-hoc signature verification and
5,149-file integrity verification passed. The DNS executable links only system
libSystem/libiconv. A subsequent 40-second packaged `--scoped-media` guest run
with no helper overrides passed: 1080p60 VideoToolbox H.264/Opus output,
37.23 seconds of advancement, 21 successful HTTP ranges, visible video in the
shared UI, only bundle/system loaded libraries and no tracked process survivors.
An app-owned DNS helper child was sampled. Seven presentation drops and zero
decoder drops were recorded; this is functional evidence, not a resource/pacing,
audible-output or clean-machine qualification. Minimum macOS remains 27.0,
and all release/platform limits above still apply. See
[the exact artifact evidence](evidence/2026-09-29-packaged-dns-helper-validation.json).


The new shared account-playback coordinator is implemented and compiled on this
host, but has no real-account native result on any platform. An explicit account
playlist action uses the guarded HTTP path; quality/expiry preserve that access
scope, and sign-out/known session expiry clear installed account media and stale
private UI results. Provider/worker/network/UI tests use synthetic fixtures only.
Account captions/comments and profile-scoped private local storage are
unsupported; anonymous local history and Save/Follow-current do not accept
account playback metadata. macOS protected persistence has separate synthetic
Keychain evidence, not proof of account login. Windows now has implemented
Credential Manager, file-import and Job Object adapters, but no account login
has run there; Linux/X11 and native Wayland remain unrun.
See [account-media.md](account-media.md). Existing packaged guest artifacts above
predate this coordinator and do not qualify it.

Guest artwork disk caching uses Unix private directory descriptors and locking.
The Windows implementation fails closed for this optional cache; ordinary guest
browsing remains available. macOS debug/release native functional checks passed twelve offline Home/cache/clear
stages; screenshots were inspected. Linux CI evidence is tracked separately in
[artwork-cache.md](artwork-cache.md). This does not
change any platform support or optimized-video qualification.

The shared inline player controls and borderless 480×270 PiP passed finite macOS
debug/release functional exercises. Three own-PID CoreGraphics probes confirmed
the same native window, floating layer, zero compact frame extents and restored
normal frame/position. Inspected captures retain controls and local subtitles.
The bounded collection-window diagnostic also passed fourteen stages in both
builds. Physical pointer dragging, multiple displays, OS keyboard/IME and
accessibility remain unqualified, as do Windows, Linux/X11 and native Wayland.
These checks do not change platform support or resource acceptance status. See
[PiP evidence](picture-in-picture.md#inline-controls-and-borderless-update).

The new headless shared-widget tests check Slint control state with synthetic
acknowledgements. They render no pixels and do not exercise OS keyboard delivery,
IME or assistive technologies. The PiP monitor-identity restoration algorithm has
regressions, but actual multiple-monitor/disconnect behavior is not qualified on
any platform. No new platform support claim follows from either test category.

The 2026-09-29 header/comments/history correction compiles and passes focused
shared-UI/provider/storage checks on macOS. A fresh isolated native shell capture
shows shared header alignment, but excludes AppKit decorations; real traffic-light
centering/mouse routing is not visually qualified. Presenter setup again reports
macOS display-clock `-6661`. There are no new Windows, X11 or native Wayland runs,
resource measurements or account capability qualifications in this slice.
