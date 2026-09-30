# mpv render-pass timing investigation

Status: **two isolated diagnostic builds compiled, loaded and functionally
checked; one unprofiled ON/OFF/OFF/ON series completed**, 2026-09-29. The CPU
ceiling did not pass repeatably. The production Homebrew keg and application dependency are
unchanged. No external maintainer message was sent. The proposed build option
below does not exist in released mpv 0.41.0.

## Observation and limits

The five-second `/usr/bin/sample` capture used frozen source `83a07b4` and C2
release `c2ff7d1c0f93145713e2de039e9aa6b574051afcba5d07ed331650c025568398`.
The app displayed the local 1080p60 fixture at 1320×860 with 30 labeled related
rows. The capture requested a 1 ms sample interval; its main-thread tree contains
1,515 samples. One inclusive 38-sample subtree goes through `timer_pool_stop` →
`gl_timer_stop` → `glEndQuery_Exec`; 37 of those descend through Apple's
`gldGetQueryInfo` → `GLDContextRec::flushContext` and command submission. Other
subtrees include UI traversal/text and further query calls. This is a concrete
call path, not 38 distinct calls, a CPU percentage, or the total timer cost.

Compilation by other agents was allowed. VO drops rose from 2 at ten seconds to
38 at 25 seconds and remained 38 at exit; decoder drops were zero. Both app and
sampler exited 0. This perturbed profile is **not a resource benchmark**. Removing
a query might shift submission work elsewhere rather than remove it. No expected
CPU saving, GPU saving or acceptance improvement follows from the sample counts.

Raw files remain local; their complete byte hashes bind this description without
publishing the large stack dump and its host paths:

| Relative artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `artifacts/cpu-profile-c2/sample.txt` | 926812 | `6dc3b4514e698c2af0e5da1706f02266fbe08fe78aeb08a70569516004b334e6` |
| `artifacts/cpu-profile-c2/app.log` | 5366 | `fb2215572c05a0e5e2a1213b83f5442a26dc0edd578355a4c8e0e96aaa3c1fca` |
| `artifacts/cpu-profile-c2/summary.json` | 269 | `a76c1003fa54c1f0a2c20bc357e50072e33045f7aaf98149bed3d48e0e3bbe0d` |

The [C2 source capture](../evidence/2026-09-29-motion-stable-source.json) has 115
independently verified input hashes. This profile is separate from the
[unprofiled stable-target ABBA](../stable-video-target.md).

## Existing option and source audit

Installed `mpv --no-config --list-options` exposes no per-pass GPU-timer switch.
`gpu-debug` defaults off and is separate from timer creation. Neither
`opengl-glfinish=no` nor `opengl-early-flush=no` removes driver submission inside
`EndQuery`. `gpu-dumb-mode` disables rendering features, including high-quality
scalers; it is not a quality-preserving timer control. These distinctions match
the exact [0.41 option definitions](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/context.c)
and [manual](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/options.rst).

In [shader_cache.c](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/gpu/shader_cache.c),
each new cached shader creates a timer; raster and compute dispatch bracket the
render pass regardless of a statistics consumer. In
[utils.c](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/gpu/utils.c),
creation skips an absent/failed RA timer; start/stop/measure already accept a null
pool, producing no timing samples. In
[ra_gl.c](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/ra_gl.c),
creation checks `GenQueries`, allocates eight queries and recycles them. Starting
a reused query reads `GL_QUERY_RESULT` without first testing availability;
stopping calls `EndQuery`. Eight slots are an intended delay, not a proof that a
driver cannot block or flush. Timer functions are loaded as
[core GL 3.3 capabilities](https://github.com/mpv-player/mpv/blob/v0.41.0/video/out/opengl/common.c).
Returning fabricated function pointers, hiding core support or changing context
version would not be an honest supported opt-out.

## Exact installed build and reconstruction inputs

The installed Homebrew keg is **mpv 0.41.0_10**, arm64, poured from a bottle. Its
receipt reports Clang, macOS 27/Xcode 27.0 and CLT 27.0.0.0.1788430756 for the
bottle build; this does not describe a new local rebuild. Local `.brew/mpv.rb`
hashes to `75ccdb2dcc87a829323f276466c902a1f97f836d0f09c7e7341957b9eb7db9e6`;
`INSTALL_RECEIPT.json` hashes to
`2b1f220752929a570683bf7edb80424eaa82777b6f318144b0de72f2c89a7569`.
The retained recipe specifies these exact inputs:

| Input | Expected SHA-256 |
| --- | --- |
| [mpv v0.41.0 archive](https://github.com/mpv-player/mpv/archive/refs/tags/v0.41.0.tar.gz) | `ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209` |
| [VapourSynth backport 75b2ccf](https://github.com/mpv-player/mpv/commit/75b2ccfeb1ce4ed5a40ac9860fa74f3d1265e13f.patch?full_index=1) | `3906b98b02071a0d5747a400406494ca69cef7afd8d3eee4a99fdbe40dc90c1f` |
| [macOS audio-device UAF backport c5d391a](https://github.com/mpv-player/mpv/commit/c5d391adba7bd024954d0df1e0405f5749f4d4ca.patch?full_index=1) | `769b218df220738cc1cf9f81cf696c16518c5dfe56a5ef028e33b22536e0e924` |

The recipe enables libmpv, HTML docs, JavaScript, LuaJIT, libarchive, uchardet and
Vulkan, disables the build date, and adds Homebrew standard Meson arguments.
Its declared build tools include docutils, Meson, Ninja and pkgconf. This review
initially found pkgconf/Clang but no Meson or Ninja on the ordinary PATH. The earlier
[offline source audit](../source-coverage.md) lacked the archive and both patch
payloads in its selected cache. The subsequent private preparation/builds below
record those bytes, the actual SDK/compiler and resolved dependency versions.
Both backports remain included; bottle-byte equivalence is not established.

## Local diagnostic build option

[mpv-pass-timers-diagnostic.patch](mpv-pass-timers-diagnostic.patch) adds one
**build-time** boolean, `gpu-pass-timers`, default `true`. Meson's existing
[feature/config generation](https://github.com/mpv-player/mpv/blob/v0.41.0/meson.build)
records it as `HAVE_GPU_PASS_TIMERS`; with `false`, `timer_pool_create` returns
null through the existing supported no-statistics path. With the default `true`,
the original path remains. This covers timer pools in the legacy GPU renderer,
not every timer in libplacebo or the application. The proposed flag is not a
runtime CLI option, and `-Dgpu-pass-timers=false` is invalid for unpatched mpv.

The patch changes neither shader dispatch, scaling/color options, decoding,
frame selection nor GL function discovery. Preserving rendering quality is the
design intent, not a completed validation claim; GPU-pass statistics become
unavailable in the disabled build. A future upstream interface could instead
make statistics explicitly opt-in at runtime while retaining current default
behavior. No such upstream change is assumed or requested here.

The initial textual `git apply --check` passed against the three exact upstream
files fetched from the v0.41.0 tag. That source-only check did not compile or run
anything; the later private paired-build result is recorded below.
Patch SHA-256: `1d4908ba5e481fe8cf7b7b22895c1cae5dec6ad71437df80740f2ab031ea1f8e`.
Unmodified input hashes:

| Path | SHA-256 |
| --- | --- |
| `meson.options` | `a20654c40ff836659bf081bc1080e5fd6344e09ed79e119dc60f938d5cbd19ba` |
| `meson.build` | `04c4798fb53f243880cf7c420ac26f019562361f05b2a09da82f7c716899741c` |
| `video/out/gpu/utils.c` | `79e35474e5d941a7e07c675aeb3b49ccacf043ab1a97d0126503e490e42df849` |

For a future controlled experiment, reconstruct both variants from the same
verified archive, two backports and diagnostic patch, with identical release
flags/dependencies and isolated prefixes/build directories. Record Meson options,
config, source/patch/library hashes and `otool` dependency closure. Verify the
actual loaded libmpv path in each isolated app build; do not replace Homebrew's
keg or rely on an unverified global loader override. Test both default-on and
explicit-off builds through motion, subtitles, transitions and cleanup before a
matched unprofiled ABBA. Compare rebuilt on versus rebuilt off, not only the
bottle versus a differently compiled library. A separate short profile can
confirm removal of the timer path; acceptance still requires unchanged quality,
verified hardware decoding, frame delivery and whole-process-tree budgets.

## Isolated preparation driver (synthetic checks passed)

[`scripts/mpv_timer_diagnostic.py`](../../scripts/mpv_timer_diagnostic.py) prepares
verified source and a private, explicit ON/OFF command plan. It does **not** run
Meson configure/compile/install or change an installed keg. The driver and its
[synthetic tests](../../scripts/test_mpv_timer_diagnostic.py) were authored during
the exclusive soak window. After the hold, all 20 focused tests passed, including
bounded probe output/timeout and actual patch application inside an enclosing
synthetic Git repository and the captured Homebrew system-library metadata
policy. Actual preparation and paired builds subsequently passed below;
loaded-library qualification has not run.

Future invocation, after prerequisites are available:

```sh
python3 scripts/mpv_timer_diagnostic.py \
  --output artifacts/mpv-timers-pair \
  --inputs /absolute/offline-input-directory \
  --tool-dir /absolute/private-build-tools/bin
```

Offline inputs use fixed names `mpv-0.41.0.tar.gz`, `75b2ccf.patch` and
`c5d391a.patch`. Explicit `--download` permits only those pinned HTTPS inputs;
there is no runtime media download or package-manager installation. Missing
Meson, Ninja, docutils or another required tool fails before preparation. Inputs
and the local diagnostic patch must match the reviewed hashes. Archive expansion,
member count/size, path traversal, links and special-file checks precede source
writes. Downloads run in a transient supervised process with a 60-second deadline
and byte cap. Probe/patch subprocesses retain their unreaped leader through group
termination, then require bounded cleanup confirmation; OS scheduling still
precludes hard real-time guarantees.

Initial central tests exposed Darwin returning `EPERM` for zombie-only process
groups. The corrected supervisor reaps the pinned leader and then requires
positive `ESRCH` group-absence confirmation; persistent permission failures remain
cleanup failures. Both outcomes have synthetic regression coverage in the Git
exporter's supervisor tests. No terminating signal is sent after reaping.

The driver checks the installed recipe and reads the actual Homebrew
`std_meson_args` body: prefix, `libdir=lib`, release build and `wrap-mode=nofallback`.
It discovers the selected Xcode compilers through xcrun, preserves invocation
names such as `clang++`, hashes their resolved binaries, and records SDK settings
plus bounded receipt-selected headers/libraries/pkg-config metadata. Both plans
use one patched source tree, the same environment and fixed build options; only
the isolated build/prefix/sysconf directories and timer boolean differ. An outer
Oxplay Git repository is excluded from patch discovery. No complete Homebrew
superenv or bottle reproduction is implied.

`provenance.json` uses source/package labels and hashes. `command-plan.json` and
raw tool-version outputs contain machine-local paths, are private, and must not
be published unreviewed. Failed preparation leaves `INCOMPLETE`; it is not a
buildable success. Executing the plan requires configure/compile/install timeouts,
source/tool/native checks before and after both builds, and comparison of effective
Meson options/dependencies; the local paired executor below performed those checks.
Actual loaded-library path and
hash verification in both mpv and Oxplay remains mandatory before functional or
ABBA runs. Prepared inputs alone are not successful builds or playback evidence.

## Actual isolated paired builds — 2026-09-29

The official archive and both exact backports above were downloaded through the
bounded preparer and matched their recorded hashes. The unchanged diagnostic
patch applied after both backports. The initial configure attempt failed before
compilation because the explicit dependency search omitted macOS `zlib.pc` and
`bzip2.pc`, required through freetype/libass. Its private log and stage metadata
remain under `artifacts/mpv-timers-pair-20260929`; this was a preparation defect,
not a playback result.

The corrected preparer captures Homebrew's actual
`extend/os/mac/extend/ENV/super.rb` system pkg-config path policy, the 13 metadata
files under `os/mac/pkgconfig/27`, and the inspected pkg-config shim's
`--define-variable=homebrew_sdkroot=...` override for the selected Xcode SDK.
Those paths/arguments are confined to the paired subprocess environment.
No global environment or installed keg was changed.

Build tools were installed only into an ignored private venv using hash-locked
PyPI wheels: [Meson 1.12.1](https://pypi.org/project/meson/1.12.1/),
[Ninja 1.13.2](https://pypi.org/project/ninja/1.13.2/) and
[docutils 0.23](https://pypi.org/project/docutils/0.23/). Matching source
distributions and their metadata/hash records were retained locally. This is
verified prerequisite-byte provenance, not a claim that those wheels were
independently rebuilt.

The fresh run at `artifacts/mpv-timers-pair-20260929-v2` used one patched source
tree, Apple Clang 21.0.0, the same Swift frontend and Xcode SDK 27.0, identical
release configuration/native inputs, and two compile jobs. Configure/compile/
install ran sequentially with 300/1800/120-second deadlines and bounded logs.
Before and after every stage the runner rechecked **832 source files, 2,775
native files (450,388,431 bytes), 13 system pkg-config files and 1,741 private
Python tool-module files**, plus selected compiler executables and SDK settings.
The linker executable has an additional post-build hash; both configure logs
record the same linker version. This is not a complete SDK/toolchain source audit.

| Variant | Configure / compile / install | libmpv SHA-256 |
| --- | --- | --- |
| Timers ON | 8.49 / 35.49 / 0.28 s, all exit 0 | `fc873c29f7f7cac23f419c7030c9351ee70a4662f4d257f974f87a7a1bca93c5` |
| Timers OFF | 5.85 / 30.60 / 0.26 s, all exit 0 | `895e653209230e86dd46f58ffe8a8ab983c8686308ef4383830ff65f41e4224e` |

These are build durations, not runtime performance measurements. Each private
`lib/libmpv.2.dylib` is 4,760,512 bytes, arm64, with a recorded minimum macOS 27.0.
Standalone mpv binaries were also compiled and hashed, but not launched.
Every configured install destination was checked to be inside its variant's
private prefix before installation.

After normalizing private build/prefix paths, the complete Meson option records
differ only in `gpu-pass-timers`. All **28 dependency records match**. Generated
config differences are limited to that macro and its configuration/feature-list
descriptions: `HAVE_GPU_PASS_TIMERS` is 1 versus 0. Linked-file inspection found
**48 non-system Mach-O nodes per variant**, including the root and 47 external
dependency nodes, with identical external closure hashes. The same checks cover
the CLI roots. System dyld-cache images are listed but not byte-hashed; runtime
`dlopen`/plugin/ICD loading is not established by this linked-file inventory.

Shareable evidence consists of the [build summary](../evidence/2026-09-29-mpv-timer-build-summary.json),
[source/tool/native provenance](../evidence/2026-09-29-mpv-timer-build-provenance.json),
[linked-file audit](../evidence/2026-09-29-mpv-timer-linked-audit.json),
[prerequisite records](../evidence/2026-09-29-mpv-timer-prerequisites.json) and
[normalized command plan](../evidence/2026-09-29-mpv-timer-command-plan-normalized.json).
Raw plans/logs and the bounded executor remain private local artifacts; public
evidence records their byte hashes and replaces private root paths with labels.

## Loaded-library checks and isolated ABBA

Two private copies of the validated `ca21de9` release were relinked to the exact
private libraries above and ad-hoc signed. `otool` and strict signature checks
passed; the ordinary release and installed Homebrew keg were unchanged.
`DYLD_PRINT_LIBRARIES=1` was used only for the functional launches: each loaded
exactly one libmpv at its intended private path. Private app SHA-256 values are
`e7ecb8a71ade690b996d4109fcd8b4069b03fcb68d48e523de8a2818afc23f8b` (ON) and
`9a3adccf774fb628d68b624b431b7f3056954caf44e5873eb3b65d511e6c76c5` (OFF).

The local lifecycle checks passed in 22.349 s and 22.229 s respectively, including
observed VideoToolbox H.264 1080p60 decoding, AAC audio, subtitles, mute, seek,
resize, fullscreen, minimize/restore, finite frame motion and clean shutdown.
Separate paused captures passed in 21.406 s and 21.357 s. Their 2640×1528 PNGs
were byte-identical (SHA-256
`2cd39c7fb1806bfcb374ca0f64134ebf786aa566c4a7aae7f7e5af1c652c4bd6`), and both
were visually inspected. This covers one paused frame, subtitle glyphs and UI;
it does not qualify every color conversion, HDR, motion frame or perceptual A/V
synchronization. Raw loader logs contain local paths and remain private.

The subsequent ON/OFF/OFF/ON series used these same copies and libraries, the
same local clip, dark theme, requested 1320×860 logical window and 30 explicitly
labeled related rows with empty thumbnails. The available display constrained
the actual window height. UI caches, clock staging, stable-target publication,
GPU queries, screenshots, loader logging and profilers were disabled. Each
85-second application lifetime included ten seconds of warm-up and 60 one-second
samples. All runs exited cleanly and stayed visible with active VideoToolbox.

| Run | CPU mean / p95 / peak, one-core % | Aggregate RSS mean / p95 / peak, MiB | Warm VO / decoder drops |
| --- | ---: | ---: | ---: |
| A1 ON | 52.168 / 62.998 / 64.001 | 190.026 / 190.750 / 190.859 | 0 / 0 |
| B1 OFF | 46.620 / 61.998 / 64.001 | 187.033 / 187.688 / 187.750 | 0 / 0 |
| B2 OFF | 50.100 / 60.999 / 63.001 | 187.301 / 187.953 / 188.031 | 0 / 0 |
| A2 ON | 50.943 / 63.265 / 64.002 | 189.722 / 190.391 / 190.469 | 2 / 0 |

Media notification deltas were 3600/3600/3600/3598 and UI draw deltas were
3838/3839/3838/3835. Catalog changes stayed zero and resets stayed at the single
fixture initialization. The sampler counted two processes per run: the app and
one newly launched, temporally associated VideoToolbox service. Two preexisting
decoder services were excluded. RSS can double-count shared pages; shared
WindowServer/audio work and energy are not attributed by this table.

OFF had a lower CPU mean in both adjacent comparisons, with a variable size of
change. One OFF run still exceeded the 50% release ceiling and all four missed
the 25% target. This is insufficient to adopt the patch or qualify optimized
playback. Neither a power saving nor a general causal estimate follows from this
single sequence. The patch remains diagnostic-only.

The public [runtime export manifest](../evidence/2026-09-29-mpv-timer-runtime-export.json)
retains all **240 measured samples** and their full per-process/host-work payloads:
[A1 ON](../evidence/2026-09-29-mpv-timer-a1-on.json),
[B1 OFF](../evidence/2026-09-29-mpv-timer-b1-off.json),
[B2 OFF](../evidence/2026-09-29-mpv-timer-b2-off.json), and
[A2 ON](../evidence/2026-09-29-mpv-timer-a2-on.json). Corresponding scalar native
and sampler logs are listed with original/export hashes in the manifest. No
samples, counters or metrics were dropped or changed during export. The
[complete summary](../evidence/2026-09-29-mpv-timer-runtime-summary.json) preserves
all four runs, including the two warm VO drops in A2 and the failed CPU budgets.

[Frozen source inputs](../evidence/2026-09-29-mpv-timer-runtime-source.json),
[runtime provenance](../evidence/2026-09-29-mpv-timer-runtime-provenance.json),
[exact sampler snapshot](../evidence/2026-09-29-mpv-timer-runtime-sampler.py) and
[normalized harness record](../evidence/2026-09-29-mpv-timer-runtime-harness-normalized.txt)
identify the experiment. The harness record replaces only the absolute workspace
root; its original and normalized byte hashes are separate. It is a provenance
record, not a newly relocated runnable harness. The source inventory identifies
the unmodified release; preparation records separately identify both relinked,
signed executable hashes and private libmpv hashes.

The manifest also includes all four successful functional loader-check records,
their full scalar native logs, and the normalized single libmpv loader line from
each private raw log. Raw loader logs stay local with byte hashes. Historical
preparation/ABBA provenance copied `runtime_loaded_library_verified=false`
before those functional checks; these original fields are preserved rather than
rewritten. The separate successful loader observations establish the functional
launches, while loader logging was intentionally off during measurement.
The [shared byte-identical paused PNG](../evidence/2026-09-29-mpv-timer-runtime-still.png)
and [comparison record](../evidence/2026-09-29-mpv-timer-runtime-still-comparison.json)
retain the previously inspected visual evidence with its limited scope.

Export manifest SHA-256:
`fb047e1e5c764d5202698640542b83c208057227df062f789cae27004aa0791e`.
This export adds auditability, not a new native run, adoption decision or passing
performance/energy/account/platform qualification.

## Native-child matched private pair: functional admission only

After the native-path profiles, two private copies of the already-profiled
`a45ff67692e0f25e22c188dc1583c3490d94a542` application were derived from unchanged
SHA256 `0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`.
Only the libmpv load command and ad-hoc diagnostic signature changed. ON uses
library `fc873c29f7f7cac23f419c7030c9351ee70a4662f4d257f974f87a7a1bca93c5`;
OFF uses `895e653209230e86dd46f58ffe8a8ab983c8686308ef4383830ff65f41e4224e`.
The resulting application hashes are respectively
`e32aea7ebe8b0c4b29f10859819f5585bb7ad5db3a026f462eb5caaf3b225e80` and
`bc6010464fb9312f10cc666def369303212bab3cb15905e07def77e463adad7f`.
This is matched source-built ON versus OFF, not OFF versus the Homebrew bottle.

Sequential native-child lifecycle runs passed all14 historical checkpoints in
47.959/47.967seconds, with normal exit, removed copied fixtures and no owned
process groups remaining. Each observed VideoToolbox1080p60, AAC48kHz through
AVFoundation, paused seek20seconds, fullscreen/pause, overlay hiding,
resize/minimize/restore and settled stop. Actual dyld records identify the
selected libmpv uniquely and all48 recorded non-system dependency images per
variant; byte hashes reverified. Other loaded image paths and UUIDs matched
across the runs after excluding the intentional app/libmpv differences. System
dyld-cache images are not byte-hashed.

Four finite owning-window captures per run were retained. The paused20-second
ON/OFF PNGs are byte-identical, as are their stopped captures. Offline decoding
of the logged1328×747video rectangle at window coordinate520,216 compared
992,016pixels: zero differences and maximum channel delta0. Inspected paused,
fullscreen and resized captures show upright video with shared controls; the
stopped video rectangle is black. These observations do not establish
continuous display cadence or perceptual synchronization.

[The17-payload functional evidence manifest](../evidence/2026-09-29-native-timer-functional-export.json),
SHA256 `98d049c2a16c681ba2a09fdd272ee4ca59d19b84cc71cd041ba3d4f86f44fed0`,
binds preparation, actual-image audit, exact sanitized app logs, signatures,
source inventory, capture hashes and documentary tools. Raw loader paths and
PNGs remain private. The initial offline parser rejected ordinary dyld
loaded/delayed status lines; it was tightened to accept only those known forms,
without changing either run or its original log.

No resource result follows from these functional runs, which included
screenshots and permitted other compilation. The patch remains a private
experiment: no installed library, original app or production default changed.
A separately scheduled unprofiled matched native-path comparison is the next
step, with complete process/footprint and shared-compositor accounting.

## Native-child matched ON/OFF/OFF/ON resource result

The same private a45-derived applications and paired libraries above completed
four85-second release runs in ON/OFF/OFF/ON order. Each retained60 samples after
a10-second warm-up, with complete separate RSS and OS physical-footprint
ledgers. All applications exited0, all anchored owned process groups were
absent, and exact application/library/native-dependency hashes were rechecked
before and after each run. No screenshot, profiler, DYLD image logging, build or
network workload was scheduled within this exclusive measurement window.
Actual loaded-image proof belongs to the preceding functional runs, not these
resource intervals.

The [34-payload evidence manifest](../evidence/2026-09-29-native-timer-abba-export.json),
SHA256 `db4bb3c7ca9672b806b60af5e6ddb727f01c47bd8cf07a54f7c4ad8243d654d6`,
retains all240 raw samples in deterministic gzip, full sanitized logs, exact
sampler, documentary harness/protocol, matched identities and independent
analysis. Every original manifest hash and recalculated mean/p95/peak matched.
The application workload remained the exact synthetic1080p60 clip and30
labeled related rows with empty thumbnail placeholders, not decoded rasters.

Values below are mean / p95 / peak. CPU is percent of one logical CPU, summed
across the sampled application-attributed processes. RSS and footprint are
MiB and must not be added together.

| Run | App-attributed CPU | Aggregate RSS | OS physical footprint |
| --- | ---: | ---: | ---: |
| A1 timers ON | 44.759 / 55.001 / 57.001 | 189.747 / 190.500 / 190.547 | 319.574 / 320.330 / 320.533 |
| B1 timers OFF | 36.665 / 45.000 / 46.009 | 188.136 / 188.875 / 188.922 | 317.865 / 318.549 / 319.033 |
| B2 timers OFF | 36.026 / 45.000 / 46.999 | 187.706 / 188.453 / 188.500 | 310.105 / 310.783 / 313.111 |
| A2 timers ON | 44.541 / 54.999 / 55.811 | 189.525 / 190.344 / 190.609 | 327.855 / 328.752 / 331.721 |

OFF is lower in both repeats. Mode-average CPU is44.650% ON and36.345% OFF:
8.304 percentage points lower, or18.599% relative, in this matched experiment.
This is an observed application-side reduction, not a prediction from profiler
weights. All mean values still miss the25% target. All means are below the50%
release ceiling; ON p95/peaks exceed50%, while both OFF runs' p95 and peaks
remain below50%. The observed RSS values are below the400MiB target and650MiB
ceiling. These narrowly scoped results do not qualify the full production
presenter, additional platforms, representative real-image browsing, account
work or all SPEC resource scenarios. Only two repeats of each mode were taken.

All warm10→70-second VO-drop and decoder-drop deltas were zero. Render
notifications advanced3600 in every run; Slint draw deltas were239/238/239/239.
Actual observations remained VideoToolbox H2641920×1080@60, AAC48kHz through
AVFoundation, speed1, volume100, unmuted, one load, no cache pause and no
occlusion. Geometry stayed window2640×1528backing at scale2, logical video
`(260,76,664,373.5)`, native backing1328×747. The final ON run had four startup
native geometry changes versus three in the others; each count was already
stable at5seconds and remained unchanged throughout the warm checkpoints.
This difference was retained, not normalized out of the log.

All240 samples contain one root and one newly observed temporal VT service,
with stable sampled PIDs/rusage-start identities, no omitted RSS entries, and
complete footprint entries. Two preexisting VT services were excluded in each
run. This does not prove exclusive decoder ownership or complete short-lived
helper accounting. Snapshot identity/read races, shared-page RSS double
counting and incomplete GPU/unified-memory accounting remain limitations.

WindowServer appeared in all60 host observations per run, at mean
30.755/32.229/32.066/30.704% CPU; coreaudiod was
7.151/7.219/7.220/7.151%. These shared services sit outside the application
aggregate and cannot be attributed exclusively to Oxplay. Their direction
also prevents equating the8.304-point app reduction with equal whole-system
or energy savings. Other-host CPU means were55.941/56.895/55.583/55.267%, with
peaks81.928/83.577/88.642/81.151%. No cargo/rustc entry appeared in the bounded
top-eight host lists; absence from that list is not proof of zero activity.

The patch remains private and unadopted. This result justifies further
production-path investigation and broader controlled qualification; it does
not authorize changing upstream dependencies or presenting the25% target as
met. Default borrowed-texture behavior and installed libmpv remain unchanged.
