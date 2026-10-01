# Native rendering performance audit — 2026-10-01

This report preserves its original frozen-build evidence. Subsequent playback
fixes and measurements are recorded in the
[2026-10-01 playback follow-up](performance-playback-2026-10-01.md); its later
results have a separate build and qualification scope.

The native rewrite and this performance audit are complete; the CPU goal is
not met. The frozen release plays through native Metal on the inspected Apple
M1/macOS 27 development host, with bounded renderer cache improvements and
passing functional checks. Its local playback CPU mean is 57.56%, exceeding both
the 25% target and 50% ceiling in [SPEC.md](../SPEC.md). The observations do not
establish a CPU advantage over OpenGL or Chrome. Both actual-website intervals
fail continuity admission, so that comparison remains unqualified. The completed
browser comparison below uses the local fixture. This audit does not qualify a
portable native release or close the remaining platform/performance gates.

Evidence remains in local `artifacts/performance-2026-10-01/`,
`artifacts/native-media/macos/`, and the named browser-run directories. File
names containing `final` or `qualified` do not independently establish
qualification. The frozen clock-fixed release binary has SHA-256
`ac3b3fe5616448e30fcc1d2d537b6f33650fa64e4bfaa55ce2a9a0e20f853983`.

## Renderer and dependency changes

Source builds default to `native-rendering`; stock libmpv is used only by the
explicit `--no-default-features` OpenGL comparison. Both retain the shared Slint
UI and libmpv demuxing, audio, seek/load correlation, controls and subtitles.
The native adapters import GPU output into persistent WGPU textures rather than
transporting full decoded video frames through CPU buffers.

| Platform | Media output and decoder | Shared UI and transfer | Evidence available |
| --- | --- | --- | --- |
| macOS | Experimental libplacebo Metal renderer; VideoToolbox texture import | Metal WGPU device and command queue supplied by Slint | Private dependency build, unit tests, headless moving-content check and development-host UI exercises |
| Linux | libplacebo Vulkan; direct VAAPI DMA-BUF import from a matched DRM node | Same Vulkan device/queue; timeline semaphore transfer and agreed sampled layout | Patch application, C API syntax and Rust cross-target checks; no native Linux runtime qualification |
| Windows | mpv D3D11 renderer; D3D11VA | Same-adapter D3D11 shared NT-handle textures opened by DX12; producer/consumer GPU fences | Patch/ABI checks and real Windows Rust API cross-check; no Windows native C build or runtime qualification |

mpv is pinned to `97179bce7ed980c53647d6344916f632fe689e9e`. macOS uses
the experimental Metal libplacebo branch at
`a96296875a067887fc75d7b74a0a4292422c204d`; Linux/Windows use libplacebo
v7.360.1. The custom installed headers require `OXPLAY_NATIVE_RENDER_ABI 1`.
Archive checksums, applied patches and installed-file inventories are recorded
by the private builders. The inspected Mac build used the macOS 27 SDK and a
12.0 deployment target; this does not establish support on macOS 12, because
the external Homebrew dependencies have their own deployment requirements.
See [Metal dependencies](native-media-macos.md),
[Linux/Windows recipes](native-media-platforms.md) and
[the integration contract](native-rendering.md).

Normal GPU work uses queue ordering or GPU fences. Capacity recovery uses
bounded completion waits and retains resource leases; it does not block the UI
on every frame. Teardown still synchronizes native work to protect callbacks
and imported resources. Software decoding remains an explicitly observable
fallback for unsupported codecs/devices; it is not hardware-decoder proof.

Ambient sampling stays on the GPU and reads back only a 16×9 RGBA8 summary:
576 bytes, at most four samples per second on an unchanged enabled load. One
readback owns the buffer until retirement. A bounded worker advances its map,
checks load/generation cancellation and wakes paused first-frame sampling.

Two empty per-frame WGPU submissions were removed from the Mac adapter. Metal
has no image layouts and the pinned HAL texture-transition methods emit no
barriers; the shared Metal command queue already orders native writes and UI
reads. The initial WGPU allocation clear remains necessary to prevent lazy
initialization from erasing native output. Vulkan/DX12 synchronization remains.

## Resource measurements

All completed local runs below used ten seconds of warmup and 60 one-second
samples. The playback fixture was 1920×1080 H.264 at 60 fps with AAC audio.
Application logs record VideoToolbox and AVFoundation audio output. App and
browser video geometry was 696×391.5 logical pixels at scale 2, or 1392×783
physical pixels; observed window geometry was 1320×796 logical pixels. These
are individual development-host runs, not a repeated final acceptance series.

CPU percentages use one logical core as 100%. RSS includes the selected process
tree and temporally attributed newly appearing VideoToolbox services. Physical
footprint is a separate macOS OS ledger and must not be added to RSS or GPU
allocation estimates.

| Workload / artifact | CPU mean / p95 | RSS mean / peak MiB | Physical footprint mean MiB |
| --- | ---: | ---: | ---: |
| Final native Metal playback, `native-metal-final-playback.json` | 57.56% / 63.11% | 303.84 / 304.33 | 350.95 |
| Final paused video, `native-final-paused.json` | 0.03% / 0.00% | 293.69 / 293.69 | 291.44 |
| Final minimized empty library, `native-final-minimized.json` | 0.03% / 0.00% | 156.11 / 156.11 | 116.06 |
| Final raster library, `native-final-library.json` | 0.02% / 0.00% | 172.78 / 172.78 | 142.33 |
| Native Metal playback, `native-metal-playback-1.json` | 59.04% / 67.00% | 303.44 / 303.75 | 351.04 |
| OpenGL comparison, `opengl-playback-1.json` | 59.33% / 69.00% | 206.10 / 206.47 | 347.99 |
| Native, chrome-cache experiment, `native-metal-chrome-cache.json` | 54.26% / 60.73% | 305.04 / 305.39 | 361.14 |
| Native, related-cache experiment, `native-metal-related-cache.json` | 56.03% / 63.47% | 307.52 / 307.89 | 364.05 |
| Native settled library, `native-idle-library.json` | 0.00% / 0.00% | 156.83 / 156.83 | 116.44 |
| Isolated Chrome local fixture, `browser-baseline/run-2octggkx/result.json` | 26.27% / 30.13% | 1024.58 / 1129.16 | Not recorded |

The first four rows use the frozen clock-fixed release. Playback peak CPU is 74.46%,
RSS p95 is 304.23 MiB and physical footprint p95/peak is 355.55/355.72 MiB,
with all 60 footprint samples complete. CPU mean is 1.48 percentage points below
the earlier native baseline; one run does not establish a causal improvement.

The two cache rows are optional experiments; product chrome/related caching
remains disabled by default. No causal improvement or repeatability follows
from comparing these single runs. The native playback RSS is below the browser
tree's reported RSS, but shared-page counting, service ownership, browser
process scope and different UI workloads prevent a general memory or browser
victory claim. The OpenGL RSS is also substantially lower than native in this
series, while their separately measured physical footprints are close.

The idle library run made five UI draws, allocated no video targets and started
no display clock. It is useful quiescence evidence for that empty workload; it
does not qualify the SPEC workload with about thirty visible cached thumbnails.
Sampled zero CPU is limited by measurement granularity, not proof of zero work.

The final paused-video run holds UI draws at 47 from five seconds through seventy
seconds, with no display-clock starts, one video target and three delivered
ambient samples. The minimized run has no loaded media: four UI draws, no video
targets and no clock starts. It covers that empty minimized workload, not
minimized playback. Both have 60 samples and approximately 1% peak CPU.

The final raster fixture has 100 model rows. Before sampling, it reports 16 ready
near-viewport thumbnails, of which 12 intersect the viewport, no pending/inflight
work, 16 local decodes and no remote starts. The sampler waits for this readiness
event; the run ends with six UI draws and no video targets or display clock.
Its 12 visible thumbnails do not establish the SPEC's approximately thirty
visible-thumbnail workload. Model readiness does not prove compositor visibility
or GPU completion. See `native-final-suite.log` for the three successful resource
runs and their precise scope.

The frozen release's separate 44-second
`native-final-library-traversal.log` passes five 100-row pages from a 10,000-row
fixture, keyboard End/Home/PageDown identity checks, resize retention of item 99
and search-focus cancellation of a delayed handoff. It records no remote starts,
100 decoded thumbnails, 96 publications, four stale discards, no failures and
a ready-queue peak of eight; model rows remain 100 across five resets. This is
functional traversal and memory-bound evidence, not a resource benchmark or
screen-reader qualification.

Host-other mean CPU was 45.21–58.27% of one core across these playback/browser
runs. That separate ledger includes system services and the harness and is not
charged to the app. It remains a comparability limit. Preexisting decoder
services are excluded even if reused; newly attributed services are not proven
exclusive to the app. RSS may double-count shared pages, and short-lived or
reparented helpers can escape discovery. Unified GPU memory overlaps other
ledgers. Target-byte counters omit decoder/libplacebo intermediates, Slint's
rounded-clip layer, swapchain storage and GPU command leases.

The first `baseline-playback.json` attempt is incomplete, with zero samples and
exit code 1, and is excluded. Chrome 154.0.8037.57's completed local-fixture
run recorded `VideoToolboxVideoDecoder`, 3,609 submitted frames over 60.1513
seconds and zero additional browser dropped frames. Those are browser media
counters, not scanout measurements. The online browser attempt
`youtube-browser-baseline/run-8rm509wu/result.json` has zero samples and failed
`initial_codec_hardware_geometry_admission`; its browser cleanup exit code 0
does not make it a successful measurement. The app's online preflight also
encountered buffering and is excluded from a steady-playback comparison.

The actual-website run `youtube-browser-baseline/run-s9jyfj5k/result.json`
collects 60 raw resource samples, but fails `interval_continuity_admission`.
At the end its video element has reset to time zero, unknown duration, zero
decoded width/height, zero frame counters and paused state. No aggregate resource
comparison can be admitted from that interval. At admission YouTube selects AV1
through the software `Dav1dVideoDecoder`, while the app selects H.264 through
VideoToolbox; this measures default-format policy, not renderer isolation.
The browser's 1392×784 physical video differs from the app's 1392×783 by the
explicit one-pixel tolerance.

The repeat `youtube-browser-baseline/run-4fy9jmtl/result.json` also fails
`interval_continuity_admission` after 60 samples with the same emptied video
state. Its initial preflight advances 121 frames over 2.024 seconds with zero
drops; that short admission does not validate the later interval. It retains
AV1/software Dav1d, Opus 48 kHz stereo and the same geometry tolerance. A bounded
passive CDP event drain runs on all 60 sampler ticks, consuming 12 messages and
2,829 bytes, with a maximum 0.203 ms drain and no budget exhaustion or pending
input. Pipe backpressure is not demonstrated; the reset's cause remains unknown.
Neither failed website interval yields admitted CPU or memory averages, and
neither supports a website victory claim.

`native-online-final.json` also collects 60 resource samples and exits normally,
but the log records six display-clock starts, five stops, cache interruptions
and a dropped-frame counter increasing from nine at warmup to 35 at seventy
seconds. It is excluded from a steady-playback comparison. A private-prefix
OpenGL control for the same video, H.264 and Opus similarly reports
`Buffering` with `paused_for_cache=true` at 25 seconds, and six clock starts/five
stops; its dropped-frame counter increases from three at warmup to 28 at seventy
seconds. The interruption is therefore not unique to Metal. These observations do not
identify a cause in the broader network, resolver, cache or media path.

A separate `native-online-settled.json` run uses **70 seconds of warmup**, then
60 one-second samples, with exit at 145 seconds. It does not replace or
reclassify the interrupted ten-second-warmup runs:

| Extended-warmup online workload | CPU mean / p95 / peak | RSS mean / p95 / peak MiB | Physical footprint mean / p95 / peak MiB |
| --- | ---: | ---: | ---: |
| Native, 70-second warmup | 63.25% / 70.48% / 72.07% | 374.35 / 381.28 / 382.64 | 425.01 / 450.14 / 453.91 |

At 70 seconds the log records four display-clock starts, three stops and 41
dropped frames from earlier startup/cache interruptions. Those counters remain
unchanged at 145 seconds, with decoder drops zero. Render notifications advance
3,410→7,910 and UI draws 4,164→8,665 over those 75 seconds, consistent with
continued 60 Hz callbacks. No further cache interruption or added drop is
observed after warmup. The result still exceeds the CPU ceiling, and null media
endpoints, callback counts and sparse diagnostic snapshots do not prove fresh
media-time progression, physical scanout or audible A/V synchronization. It is
not a matched successful website comparison.

App sampler JSON files have null `media_start`/`media_end`. Their diagnostic
snapshots cache progress when controls hide: reported playback position does
not advance between the ten- and seventy-second checkpoints. Display-clock
ticks and UI draws do advance by approximately 3,600, and recorded decoder
drops are zero, but these are not fresh media-time endpoints or physical frame
presentations. Actual moving-content checks below provide separate correctness
evidence. In the final native playback log, UI draws advance from 581 at ten
seconds to 4,181 at seventy seconds: exactly 60 callbacks per second. The dropped
frame counter stays at one from five seconds through exit, so the recorded warm
interval adds no drops; decoder drops remain zero. Two persistent targets total
8,719,488 bytes, and all 327 ambient samples are delivered with no recorded
failure. These counters improve the observed warm-interval evidence, while
fresh media endpoints, physical cadence and audible A/V synchronization still
require stronger matched observations.

## CPU diagnosis and bounded changes

The initial `native-release-profile.sample.txt` showed repeated WGPU render
pipeline creation under FemtoVG flushes. This wall-stack sample identified a
candidate; its counts are not CPU percentages. Slint flushes its clear before
the render notifier and later flushes the scene. The upstream per-flush cache
retired each other's pipeline keys, causing repeated creation.

The vendored FemtoVG 0.27.0 patch retains a pipeline through one unused nonempty
flush and expires it on the second. Its map is capped at 256 entries, with
older idle entries evicted first. An eight-entry exact-dimension viewport cache
similarly retains immutable 16-byte uniform buffers and their bind groups.
It contains no video/image/glyph texture references. Both preserve pipeline
keys, draw ordering and synchronization. Excessive key churn remains correct
but can recreate evicted entries. Source/archive provenance and licenses are
in [the vendor record](../vendor/femtovg/OXPLAY-PROVENANCE.md).

The subsequent `native-cpu-summary.json` resolves 4,675 Running-state Time
Profiler rows, or 4,675 ms weighted CPU over an 8.581-second sample span.
It covers only the target process, excludes waiting threads and is statistical
sampling. Inclusive categories overlap and must not be added:

| Inclusive activity | Share of sampled CPU |
| --- | ---: |
| FemtoVG flush | 24.53% |
| Scene traversal | 14.93% |
| WGPU render encoding, within flush | 9.28% |
| Native video render | 6.40% |
| Text drawing, within scene traversal | 6.05% |
| GPU submission | 5.90% |
| Binding creation | 3.70% |
| Viewport buffer/group creation | 2.42% |
| App update | 0.68% |
| Ambient sampling | 0.53% |
| Pipeline creation | 0.02% |

The main thread held 58.59% of sampled CPU. These findings support investigating
scene traversal, text and command encoding before redesigning app observers.
Pipeline creation fell to one sampled millisecond after retention, but that
alone does not establish the final patch's resource benefit. The viewport
cache's final release benefit still needs a matched measurement.

Slint's rounded video clipping renders the video subtree into a cached layer.
The per-frame image setter must remain to invalidate that layer; a full window
redraw alone does not suffice. Skipping publication to improve CPU can freeze
ordinary rounded playback while fullscreen/theatre appears to work. The shared
UI, rounding, controls, captions and overlays were preserved instead.

## Verified checks and remaining gates

The actual `test-native-final-clock.log` totals **672 passed, seven ignored and
zero failed**. `clippy-native-final-clock.log` finishes successfully in 15.69
seconds. Earlier native workspace logs include 22 media-test failures and are
retained as superseded evidence; the later passing run is the relevant check.
`build-native-final-clock.log` records a successful optimized build in 4m 09s.
Earlier successful builds precede the stopped-clock fix and do not certify the
frozen release.

Eight GPU-free FemtoVG pipeline/viewport policy tests pass in
`test-femtovg-cache-final.log`. The retained negative controls deliberately
fail when original pipeline expiry or uncached viewport behavior is restored;
the viewport validation runner reports `Validation PASS`. Fifteen macOS
packaging-tool tests pass in `test-package-final.log`, and 35 shared/offline
browser-baseline harness tests pass in `test-browser-harness-final-35.log`,
including the online zero-drop continuity helper and bounded passive event drain.
Five PiP geometry tests pass
in `test-pip-restore.log`.
These checks have distinct scope and are not native driver/performance tests.

The headless Metal smoke reports ABI 1, exactly one deferred capacity wake,
15 rendered VideoToolbox frames with 15 distinct nonblack RGB hashes and no
post-teardown callbacks. The retained stale-signature negative control renders
15 callbacks but only one distinct frame. Preserving libplacebo's monotonic
queue signatures fixes that failure; allocator addresses are not frame identity.
Diagnostic readback and GPU waits in this smoke are excluded from measurement.

`native-final-smoke-runner.log` exercises pause/seek, mute, subtitle Off, focus/editor
Escape, resize/fullscreen and minimize/restore through eight ordinary stages.
Its finite compositor snapshots detect 793 changed pixels out of a 128×72 grid
between samples at 1.4 and 2.4 seconds, providing actual moving-content evidence
under the ordinary rounded video clipping. Snapshot readback is excluded from
performance. `native-final-pip-runner.log` passes
all nine stages: borderless compact entry, paused seek/resume, resize to 360×260,
Escape restoration to the original decorated 1320×796 client, remembered compact
re-entry and close-request restoration. The earlier PiP run failed stage seven;
restoring integrated native titlebar appearance before applying saved client
geometry resolves that sequence. Its functional run reports
`performance=not_measured` and does not satisfy a dropped-frame budget. The final
window suite preserves one preliminary PiP exit 1 followed by the successful
exit-0 rerun; this is not evidence of repeated reliable passes.

The earlier paused handoff failed to publish a replacement frame. A stopped
display clock must not gate pending work on a display tick it cannot deliver.
The new unit test protects that condition. The debug
`native-handoff-clock-fix-debug.log` reaches all eight checkpoints, including
ready paused replacements at 15 and 28 seconds. The frozen release repeats all
eight in `native-final-handoff-runner.log` and exits 0. The successful release
handoff, PiP and moving-content smoke share the frozen binary hash recorded in
`native-final-window-suite.log`. Developer-host checks do
not establish perceptual color/orientation, subtitle placement across codecs,
audible A/V synchronization, HDR/4K, accessibility or multiple-monitor behavior.

Windows/Linux require actual native dependency builds and real-host playback,
driver/fence validation, failure/resize/paused recovery tests, resource/power
measurement and packaging qualification. Cross-target Rust checks and host C
syntax checks cannot substitute for those runs. macOS measurements cannot be
extrapolated to those platforms.

Higher-resolution/HDR coverage, power comparisons, long playback/resize/soak
tests and portable-release validation remain open. Successful unit, functional
and local resource runs do not convert the failed CPU target or unqualified
website comparison into a passed performance gate.

CI now separates the default pinned-native macOS job from explicit stock-libmpv
OpenGL comparison jobs. Hosted hardware may skip the headless runtime smoke
with an explicit availability result; compilation is not a playback pass.
Existing historical releases use the legacy stack and do not certify native
output. Native bundles need the patched media dependency closure, exact sources
and notices, relocation/rpath and clean-machine checks, plus signing validation
where claimed. No browser/CPU win, power win, portable-release approval or full
platform feature-parity gate is closed by this audit.
