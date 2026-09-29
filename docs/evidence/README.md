# Local measurement evidence

`2026-09-29-empty-idle.json`: macOS 27 / M1 release binary SHA256
`992f684ca0dd31b08c83101f1e67c5a16e819928001440c3d517eca8364cf37c`,
initial commit e945b3e plus the subsequently committed media/scheduling changes.
This run preceded the idle-target-allocation and paused-seek follow-ups.
Ten-second warm-up, 60 samples at one-second intervals. No thumbnails, no media,
one process, no new VideoToolbox service. Scope is an empty shell, not AC-12's
settled library. Three rendering callbacks occurred over the 85-second application
lifetime, with zero explicit media redraw requests. No claim about partial GPU redraw.

Selected safe diagnostic logs are retained here. Other raw local logs, sample
traces and screenshots remain in ignored `artifacts/`.
The current benchmark script records app-owned descendants and, optionally,
newly launched VideoToolbox service PIDs. The latter uses timing-based attribution,
not OS-proven exclusive ownership. Shared/unified memory is not blindly summed as
an additional GPU cost. Transient helpers need higher-resolution measurement.

`2026-09-29-playback-failing.json`: commit 463e677, binary SHA256
`982a0050b1fd2259a289c70467475599aa447b44300cf4c85a30d2fb0a7d0cd2`.
Input SHA256 `d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0`.
Includes the application and one new decoder service. It fails the 60 fps gate:
2,118 video draws and 2,961 VO drops across 85 seconds despite zero decoder drops.

The native-lifecycle, end-of-file, and null-audio-diagnostic logs come from commit
`3846416`, binary SHA256
`ebf7d8525fa8cadd6a46b4546c4d90f8846c6010095c7ff4ce4168d2fe2cd5c3`.
The null-audio run is a silent clock-isolation experiment, not an audio or resource
acceptance run. It still lost 237 of 900 nominal frames between 10 and 25 seconds.
The lifecycle run uses explicitly labeled fixture rows; its only catalog reset is
the initial insertion. The EOF clip is a three-second stream-copy trim of the
same local fixture, with an observed duration of 3.008 seconds.

The callback-latency log comes from commit `7e4e81b`, release SHA256
`d3253b95619de9b015699c3312ea6f24ec9a153bac656fe323eb336e67ff7200`.
It records engineering timing diagnostics, not a 60-second resource gate.

The minimized-idle JSON/log come from commit `9bada8a`, binary SHA256
`7b1e68569a66e881910a0966126ed2d65293e46987fb41c3d9fd63223469bbf5`.
State was verified while the native event loop was active, before sampling and
immediately before quit. The application exited cleanly; the resource sample
covers 60 seconds after ten seconds of warm-up. Mean RSS 111.610 MiB and mean
CPU 0.0154% are empty minimized-shell results, not library/playback qualification.


`2026-09-29-display-clock-playback.json` records the working-tree macOS display
clock candidate: application SHA256
`a36b84a0f86d0ef133ddf671996b6fc220a427d60a7dfc9be3d2d30dca9f4761`,
base commit `319fa36251de8d1b97cab964f8906796db03f817` plus uncommitted changes.
Media source hashes were recorded locally in
`artifacts/display-clock-source.sha256`. Hardware playback at 1080p60 with
1384×778 presentation had zero additional warm VO drops, but averaged
52.79% of one logical CPU and 185.77 MiB RSS, so the CPU release ceiling fails.
This run used the redesigned watch page with an empty related list.

`2026-09-29-standalone-matched-playback.json` records the diagnostic-only SDL
baseline, same local clip, renderer target dimensions, engine options, hardware
decoder/audio output, native display-clock mode and 10+60-second methodology.
Harness source SHA256 `9862b37bd523170f9eeb6d9605f7090802b1f4da7f5efc820c06bb0dda5496ea`;
harness binary SHA256 `4c17a09726bfbd1ab6a399c74a583ec0469b34f88e6958f4534511f95104baa0`.
CPU averaged 44.59%, RSS 163.46 MiB, zero additional warm VO drops (three startup
drops unchanged through 85 seconds). Both samples include a new temporally
attributed VideoToolbox service. The difference of independent run means is
22.31 MiB and 8.20 CPU percentage points; it is an estimate of application
overhead, not a controlled per-stage allocation. SDL omits the application's
window composition and uses a smaller window with the same video target.
The display was explicitly woken for five seconds before the standalone run;
SDL then prevented idle display sleep. No global power settings were changed.
Both runs report 1440×900 at 60 Hz, Retina scale, M1/macOS 27, VideoToolbox,
AVFoundation, H264 1920×1080 at 60 fps, and AAC 48 kHz. Perceptual A/V sync,
energy and GPU execution time remain unqualified.


`2026-09-29-chrome-cache-off.json` and `-on.json` are the same-binary c77ebe91
comparison (SHA256 6b3c97f2a2a610aab267d0bb6dce4b806b714b195efcc4c92e8627cd30637ede).
They use 30 labeled synthetic rows, with empty thumbnails. Cache-on fails frame
throughput after 25 seconds (299 VO drops at 70s, 592 at 85s), so its lower mean CPU
must not be reported as an optimized playback pass. Cache-off had one extra
warm drop and 47.73% mean CPU. See performance.md for full context and flags.

- `2026-09-29-guest-description.png`, `2026-09-29-guest-comments.png`, and `2026-09-29-guest-comments.log`: release guest details/comments native harness, 760×600 logical light UI, real public 20+20 comments and cancellation. Intentionally captured public content; not fixture data. Screenshot readbacks mean this is not performance evidence. See [comments evidence](../comments.md).

`2026-09-29-continuous-clock-cache-off.json` and `-on.json` retain the revised
phase-continuous clock follow-up and exact source/binary hashes. Both are
uncontrolled observations: another project's compilation was observed around the
measurement interval. Warm VO drops were352 and257 respectively; lower CPU
numbers are not an efficiency result. See performance.md for timestamps and
full limitations. No foreign process was stopped.

`2026-09-29-media-tls.json` records synthetic loopback HTTPS qualification using
the production Player TLS configuration: untrusted CA rejection, explicit-CA
acceptance and hostname-mismatch rejection. No credentials or external media
were used. See media-tls.md and scripts/test_media_tls.py; the ignored native
test was explicitly executed, not counted as a default-suite pass.


- `2026-09-29-guest-captions.png` and `.log`: genuine public caption selector at 760×600 logical pixels, explicitly inspected after the compact popup/exact-load/fresh-position update, with passing native selection/Off/cache reuse/quality reattachment. Mostly paused and includes screenshot readback; no performance claim. Popup obscures caption glyphs, so visual subtitle rendering remains unqualified. See [caption evidence](../captions.md).

- `2026-09-29-scoped-media-native.log`: experimental in-process HTTP transport in the native player, 60-second debug run. Hardware video/audio and clean exit observed; slow startup and 1,469 VO drops under competing builds remain unqualified.

- `2026-09-29-stream-refresh.log`: 70-second direct TLS guest expiry diagnostic; paused deferral, fresh hidden-control position and single-refresh budget passed. Simulated metadata expiry, genuine re-resolution; not a resource benchmark.

- `2026-09-29-scoped-media-release-timing.log`: b1f4074 release functional range
  timing, followed by occlusion/pause; not a steady-playback performance sample.
- `2026-09-29-packaged-caption-validation.json`: source-associated b1f4074 bundle
  caption lifecycle, cleanup and loader checks, including retained failed probes.
- `2026-09-29-local-subtitle-glyphs.{png,log,vtt}`: explicit synthetic local VTT
  glyph-composition diagnostic. One-shot screenshot readback and debug drops are
  excluded from performance claims; no public catalog content is fabricated.

- `2026-09-29-clear-local-native.{log,json}`: isolated 85-second release diagnostic
  covering real captions, quality replacement, catalog admission blocking and
  confirmed local deletion. Post-exit SQLite/caption counts are included; occluded
  playback and GPU capability queries are not performance measurements.

- `2026-09-29-dns-backed-scoped-playback.{log,json}`: paired `bc2e53f` release
  executables, actual guest VideoToolbox/AVFoundation playback through the new DNS
  helper. Forty seconds, no occlusion, clean exit; overlapping builds, 19 VO drops
  and no CPU/RSS sampling make this functional evidence only.

- `2026-09-29-abba-summary.json` and four `2026-09-29-abba-{a1-off,b1-on,b2-on,a2-off}.{json,log}` pairs:
  frozen `bc2e53f` release, repeated whole-chrome cache comparison. All warm frame
  counters pass this clip's loss threshold; all mean CPU results exceed the
  release ceiling. See performance.md for measured values and scope.

- `2026-09-29-soak-local-*`: frozen `ee59eaa` local 60-minute mixed-use run;
  3,600 complete resource samples in deterministic gzip, full scalar native log,
  source hashes, external helper audit, analysis and SHA-256 export manifest.
  Local lifecycle assertions passed; RSS growth and untested remote/account/
  fallback workloads leave the full gate open. See [soak-local.md](../soak-local.md)
  for sanitization, exact hashes, shutdown ordering and measurement limits.

- `2026-09-29-raster-corrected-*`: source `ca21de92`, identical 125-input debug
  and release inventories, six clean finite native diagnostics and every file's
  SHA-256 in the summary. Both profiles passed corrected feed keyboard and local
  mute/keyboard/video lifecycle; release also passed public guest refresh and
  caption-quality reattachment. Prior failures remain in `raster-first-summary`;
  the earlier screenshot is not reassigned to this source. No resource, account,
  screen-reader, IME or audible-sync qualification follows.

- `2026-09-29-raster-first-*`: frozen `0f311dd` source and release, first native
  raster-library and mute/keyboard regressions. Page/hover and mute checks passed;
  both full lifecycle attempts failed and remain explicitly failed. Separate
  synthetic-only native PNG was inspected. See [UI validation](../ui-validation.md).


`2026-09-29-soak-attribution-*` retains the completed second sixty-minute local
soak on frozen `ee59eaa47289382b6784547e8bece1b38e5e60a5`, executable SHA-256
`464fdcf1432f1425698cf2892f2fd7f3e942c4e132dfb176c5c45bcb5582b0a8`.
All 3,600 resource samples are preserved in deterministic gzip; the manifest
records original/export hashes and privacy checks, and its own SHA-256 is
`e618c7b63f305e742a8f1a10f011c879e997b59bfa2135254298ed90addf50c5`.
The strict local functional analyzer accepts 120 checkpoints/60 cycles/12 loads
and normal cleanup. Aggregate mean RSS was 234.158 MiB; matched-minute median
growth was 9.306%, with two of twenty pairs above 10%. Most absolute growth was
in application RSS (matched median18.551 MiB), with 2.773 MiB in the temporally
attributed decoder service. This is not a leak-free, current-build or full-SPEC
qualification. See [process attribution and limitations](../soak-attribution.md).

- `2026-09-29-mpv-timer-runtime-*` and four
  `2026-09-29-mpv-timer-{a1-on,b1-off,b2-off,a2-on}.*` sets: isolated legacy GPU
  pass-timer ON/OFF/OFF/ON experiment on frozen `ca21de92`. All 240 resource
  samples and complete scalar native/sampler logs are retained, with source,
  executable, private native-library, sampler and harness provenance. The export
  manifest SHA-256 is
  `fb047e1e5c764d5202698640542b83c208057227df062f789cae27004aa0791e`.
  Separate functional loader observations and the previously inspected,
  byte-identical paused image remain distinct from performance runs. CPU means
  were 52.168/46.620/50.100/50.943% of one core; the patch was not adopted and
  optimized playback remains unqualified. Original pre-functional false loader
  flags are preserved and explained, not silently replaced. See
  [experiment and public evidence links](../experiments/mpv-pass-timers.md).

- `2026-09-29-raster-idle-*`: all 240 resource samples and full native/sampler
  logs from the old `ca21de92` and neutral-focus `d7c58acd` settled/minimized
  pairs. Old settled CPU failed at 1.248202%; old minimized is invalid because
  of restoration, workload changes and exit 1. New settled/minimized mean CPU
  was 0.016667%/zero sampled deltas, mean RSS 129.933/130.073 MiB, and separate OS
  footprint 147.606/147.731 MiB. New counters stayed stable after readiness and
  minimized remained observed through quit. Only six thumbnail rectangles
  intersected the layout; the thirty-visible gate remains open. The manifest
  SHA-256 is
  `562b9825759a60b571b285895fc2f15c645a32e406a49e454c4afbfb210b5ffd`.
  See [scope and limitations](../performance.md#raster-library-idle-retained-failures-and-neutral-startup-focus).

### First restricted native-child attempt and d7 default regressions

`2026-09-29-native-child-first-export.json` indexes eleven exact payloads for
source d7c58acd/release6d982bb8: successful default local and10,000-item library
regressions, the failed native-child lifecycle(stage23 pause/fullscreen race),
and two successful external owning-window captures with finite pixel analysis.
The before/after133-input inventory matched the source commit. CapturePNG hashes
are retained; private PNGs show synthetic local content only. Moving pixels and
upright watch placement do not qualify cadence, other overlays, A/V or resource
budgets. The later pause fix requires separate build/runtime evidence.

### Corrected native-child lifecycle and matched ABBA

`2026-09-29-native-child-corrected-export.json` retains the a45ff676 release's
corrected default-local, two14-stage native lifecycles, separate finite native
motion captures, library keyboard/paging regression and local-library UI
capture metadata. All runs exited normally. The library UI capture uses Slint
snapshotting; its generic original summary's external-capture wording is
explicitly corrected in a separate retained analysis. The first d7 failure is
not removed. Manifest SHA-256:
`5ac8763e8e3eaae12130ba32eb241c21f406e33ece54e0d74f0016c94738f8d6`.

`2026-09-29-native-child-abba-export.json` retains all240 complete resource
samples (four deterministic gzip JSONs), native/sampler logs, independently
recomputed statistics, source inventory and binary/library/sampler/harness
provenance for default/native/native/default on that same frozen release.
Private filesystem operands and kernel hostname are labeled; exact original
and exported hashes distinguish those transformations. Original profile contents
and unrelated process arguments are excluded. Manifest SHA-256:
`29c136b025e0f8ba7feb9225b9823b9021bbf3bb97bec735c41fa9cb09176660`.

App CPU means were50.658/46.437/43.568/48.784% of one core, with zero warm VO
and decoder drops. Native Slint draws fell to240/239 per nominal minute versus
3838/3839 default, but WindowServer mean CPU increased in the two native runs.
There is no25% target, system-energy or production-native qualification.
See [full comparison and limits](../experiments/native-video-child.md#matched-abba-resource-comparison-on-a45ff676).

`2026-09-29-native-child-profiles-export.json` records two separate five-second
thread-stack diagnostics on the same a45ff676 app: default and native, each with
its own28-second lifetime. Both exited0 and owned groups were absent. Raw private
samples are represented by hashes and bounded useful stack aggregates; clean
app logs and supervision are retained. Loaded libmpv UUID matches the installed
0.41.0_10 library. Substantial VO drops occurred during these
sampled runs, which permitted concurrent compilation; no individual cause is
attributed, and these are not benchmark results. Wait/IPC observations are explicitly
separated from other leaves and never reported as CPU percentages. Manifest SHA:
`91c71b18d8bab8f8f80e0e927001fe156ca5a73ceb683f6d0a93adff0551fcde`.
See [profile interpretation](../experiments/native-child-profiles.md).

`2026-09-29-native-child-cpu-export.json` records subsequent targeted Xcode Time
Profiler diagnostics, native then default, on frozen0734. Both apps/recorders
completed cleanly. Whitelisted weighted-stack/thread summaries, exact raw trace
manifest/XML/tool hashes, app logs, analyzer and sampling audit are public; raw
traces/TOCs with automatic environment/device metadata remain private. All rows
resolve to the owned target and tool stateRunning, but1809/native versus676/default
statistical weights differ substantially without an established cause. No mode
CPU percentage, speedup, exact kernel attribution or benchmark result is claimed.
Manifest SHA-256:
`58051c7c8585772c49ffb9050078a2d570d4f7746d0834039d849aaa2feefbac`.
See [scope, audit and candidate review](../experiments/native-child-profiles.md#subsequent-targeted-time-profiler-recordings).

The d78332c UI workflow release is associated with executable SHA256
`e32cfa128282c781a80eeba06a8259207b07f54da946056b77185fa04efb88b9`
and138 unchanged committed build inputs. Its separate manifests record:

- [Local Save](2026-09-29-local-save-manifest.json): eight stages with genuine
  guest metadata, committed/idempotent SQLite assertions and an inspected compact
  popup screenshot. No private database or account credentials are exported.
- [Related keyboard focus](2026-09-29-related-focus-release-export.json): all16
  release checkpoints, including compact scrolling and fullscreen retirement.
  The earlier debug failures remain in their separate export.
- [Native-child lifecycle](2026-09-29-ui-workflows-native-export.json): all16
  checkpoints and hashes/visual analysis of14 private owning-window captures,
  including readable-message suppression and identical paused-frame restoration.

All three runs exited cleanly and verified owned-process cleanup. They are finite
functional evidence, not performance, account, accessibility or platform support
qualification. Documentary harness path substitutions are explicit and retain
the hashes of the exact privately executed originals.

The matched native-child mpv timer investigation has separate
[functional](2026-09-29-native-timer-functional-export.json) and
[resource](2026-09-29-native-timer-abba-export.json) manifests. Both derived
a45 apps passed14 lifecycle stages and exact loaded-library checks; their paused
images were identical. All240 ON/OFF/OFF/ON samples are retained. OFF lowered
application-attributed CPU in both repeats but missed the25% target; no production
patch or energy benefit is claimed. These historical app copies exclude the
later Retry/library source slice. See [scope](../experiments/mpv-pass-timers.md).
