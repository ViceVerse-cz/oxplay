# Playback performance follow-up — 2026-10-01

The final local 1080p60 runs use 33.59% and 33.10% of one CPU core, down from
55.58% before these fixes: approximately 40% less CPU work in this comparison.
Chrome's matched bare-video run uses 20.23%. Oxplay's aggregate RSS is about
309 MiB versus Chrome's 1028 MiB, but its CPU still exceeds the 25% target in
[SPEC.md](../SPEC.md). These observations do not establish parity with Chrome
or qualify every playback configuration.

The final candidate includes pixel-alignment guards, retained GPU texture imports
and the Metal presentation correction described below. Moving 60 fps playback,
the finite functional checks, pixel comparisons and actual Core Animation
presentation cadence support this host's 60 fps result. The existing 30 fps
pacing issue remains open. Intermediate measurements retain their separate
build scope. CI packaging and publication of the planned
[v0.1.1 release](https://github.com/ViceVerse-cz/oxplay/releases/tag/v0.1.1)
require their own workflow receipts; publication is not asserted here.

The [earlier native-backend audit](performance-native-2026-10-01.md) preserves
its frozen build's 57.56% CPU result and platform limits. This follow-up records
subsequent work prompted by the playback CPU/GPU regression.

## Retained measurements

The reference host is an Apple M1 with 7 GPU cores and 16 GiB unified memory,
running macOS 27.0 on AC power with low-power mode off. Both players use the same
local 90-second H.264 1920×1080, 60 fps fixture with AAC audio. The displayed
video is 1392×783 physical pixels. Samples follow a 10-second warmup and contain
60 one-second CPU-time intervals. A CPU value of 100% means one logical core.

| Local playback run | Mean CPU, one-core % | Mean aggregate RSS, MiB | Scope |
| --- | ---: | ---: | --- |
| Before these fixes | 55.58 | 305.11 | `release-baseline-final-60.json` |
| Final candidate | 33.59 | 308.94 | `release-candidate-final-60.json` |
| Final candidate, repeat | 33.10 | 308.35 | `release-candidate-final-repeat-60.json` |
| Chrome for Testing 154.0.8037.57 | 20.23 | 1028.13 | Bare local video, isolated profile |

The final candidate's CPU p95 is 37.42% in the first run and 37.00% in the
repeat, versus 62.00% before these fixes. The browser p95 is 23.63%. Earlier
retained baseline runs use 54.83% and 56.81%; intermediate cached candidates
use 34.17% and 34.16%. They support the direction of the CPU change, but are
not measurements of the final binary. This is a comparison with a small local video
page in Chrome, not the YouTube website with its application, ads or network
workload. It does not establish an advantage over actual YouTube playback.

Oxplay's final logs report VideoToolbox hardware decoding, 60 fps source
video and captions off. During the measured playback, media position advances
from 9.43 to 69.43 seconds; the dropped-frame count stays at one and decoder
drops stay at zero. The UI produces 2700 further draws between the 25- and
70-second checkpoints, maintaining 60 draws per second. Two native render
targets remain stable at 8,719,488 bytes.
Chrome reports a hardware VideoToolbox decoder and zero dropped frames while
media time advances. These checks support moving playback at the intended
cadence. Geometry, advancing media time and cadence were inspected separately
from the resource sampler; its JSON media checkpoints remain null. UI draw
counts alone do not prove physical display scanout.

CPU and RSS aggregate the player process tree and newly appearing VideoToolbox
services under the sampler's temporal attribution rule. A pre-existing reused
decoder service may be omitted. RSS can count shared pages more than once;
unified GPU allocations can overlap RSS. The browser runs more processes, so
the approximately 309 versus 1028 MiB comparison is aggregate RSS, not exclusive
physical memory or total GPU residency. The final candidate uses about 3–4 MiB
more RSS than the old build; this is not a memory saving versus that build.
Bounded UI layers retain GPU resources while reused. The separate physical
footprint ledger records 350.59 MiB before these fixes and 344.41/343.51 MiB in
the final runs. Footprint must not be added to RSS or interpreted as exclusive
physical residency.

Other host work remains present and is uneven: the final baseline reports
81.09% of one core outside the measured process set, peaking at 282.81%, whereas
the final candidate runs average about 55%. Earlier baselines at 54.83–56.81%
app CPU had lower host activity, supporting the direction of the improvement.
The sequential final pair is still not a controlled causal estimate for every
machine or content type. Its sampler status remains
`completed_not_automatically_qualified`; resource completion alone does not
perform continuity or quality admission.

The separate matched 30 fps comparison (`release-final-30-comparison.json`)
records 40.81% CPU before these fixes and 24.36% in the final candidate. Over
the 10–70 second interval, the old build's drop counter rises from 10 to 75
(65 additional drops); the final candidate rises from nine to 48 (39 additional
drops). Decoder drops remain zero in both, with the same fixture/window,
approximately 60 Hz display clock, 50 ms timing lead and AvFoundation audio.
The baseline's published position becomes stale, so its counter deltas provide
the comparison rather than proof of advancing media time.

This matched pair establishes that the observed drop issue predates these
fixes, with fewer observed drops in the candidate. It does not identify the drop
path, prove audible A/V sync or qualify drop-free 30 fps playback. The candidate's
316.13 MiB RSS and sub-25% CPU result do not establish a passing CPU target under
that unqualified continuity condition. The existing 30 fps pacing issue remains
open. No scheduler or audio-smoothing alternative is adopted for this release;
the changes remove redundant work without intentionally lowering frame rate,
source resolution or playback speed.

## Work removed without lowering playback quality

The decoder, source resolution and 60 fps playback policy remain unchanged.
The native path continues to render into GPU textures shared with the UI;
it does not add full-frame CPU copies. The changes remove redundant work or
reuse resources while retaining normal invalidation and queue ordering.

The common gpu-next patch selects libplacebo's
`skip_caching_single_frame` policy for an ordinary fresh video frame. It avoids
an unnecessary full-size frame-cache intermediate when another presentation
does not need it. Multi-vsync display-synced frames, still frames, redraws and
repeats retain caching. Still frames continue to disable temporal mixing.
Scaling, color conversion, transfer functions, HDR handling and dithering keep
their existing configuration. Frame content signatures still distinguish new
content; allocation addresses are not used as content identity.

FemtoVG converts only an exact, initial full-target clear into an attachment
clear with the same premultiplied color. Partial or later clears, unusual
formats, nonfinite colors and other geometry keep the shader path; stencil
contents keep their existing load/store behavior. Texture and glyph bind groups
are reused within a flush, with a 64-entry bound and ownership released at flush
end. Existing pipeline and viewport caches remain bounded. The WGPU window
backend batches its background clear with the final scene when dimensions agree,
while preserving native video submission before the scene reads that texture.
Dimension transitions and callback-visible output textures retain the original
early flush.

Steady header, sidebar, watch actions and related-content layers can reuse
rendered content until their dependencies change. Optional caches require
translation-only transforms, finite integral physical origins and integral
physical child bounds. Fractional placement releases the cached image and
renders children normally. Mandatory opacity and group clipping keep their
original composition. This fallback matters: an earlier cache-on/off comparison
found antialiasing changes at fractional focus and image edges.

The direct rounded-video path avoids an intermediate clip layer only for a
single childless imported GPU image whose size and fitting exactly match the
clip. It requires uniform corners, translation-only transforms, integral
physical clip geometry and a containing rectangular ancestor. Fractional
ancestor edges need one physical pixel of clearance. Overlapping children,
partial viewport intersections, unsupported geometry and visible player overlays
use the original group-clip path. The source pixels still match the physical
display size; no lower-resolution render target is substituted.

An item's imported FemtoVG image now survives a source-property update only
when the owning actual WGPU texture handle and all image flags match. Different
allocations or sampler flags create a fresh import. The same import reads fresh
GPU pixels, and every video source-property notification still invalidates
dependent rounded/opacity layers. Reuse neither makes changing video images
equal nor suppresses frame publication. It introduces no global image cache or
extra ownership beyond the item that already holds the import.

The Metal HAL correction adds one `PRESENT` bit to its ordered-use mask. WGPU
may then omit a barrier for the identical `PRESENT -> PRESENT` state. This
removes a redundant zero-encoder pending-writes submission; render, copy and
initialization transitions into presentation remain tracked. Metal still submits
the actual presentation command buffer and schedules `presentDrawable`. Queue
order, resource ownership, fences and completion handling remain intact. The
change is confined to Metal.

See the retained provenance for [FemtoVG](../vendor/femtovg/OXPLAY-PROVENANCE.md),
[the Slint renderer](../vendor/slint-femtovg/OXPLAY-PROVENANCE.md), and
[WGPU HAL](../vendor/wgpu-hal/OXPLAY-PROVENANCE.md) for patch scope and tests.

## GPU and visual qualification

A separate final sequential whole-system power pair contains 30 samples per
build (`gpu-pair-final-summary.json`). Both runs use the same fixture and window,
default caches, ambient mode and unmuted audio. Binary identities are recorded
in `gpu-baseline-final.identity.json` and `gpu-candidate-final.identity.json`;
the latter matches the final candidate hash below.

| Whole-system metric | Before | Final candidate |
| --- | ---: | ---: |
| Estimated GPU power, mean mW | 3894.18 | 3328.23 |
| GPU active residency, mean % | 72.57 | 69.34 |

Thermal state is nominal in both. Per-process GPU time is unavailable. These
values include other host activity and are not app-attributable GPU energy or
GPU utilization. The final pair's estimated whole-system GPU power is about
14.53% lower; this does not isolate the Metal correction's contribution or
prove an exclusive app energy saving. An earlier intermediate pair also moved
in the same direction (3851.94 to 3302.17 mW).

The matched Metal trace comparison (`metal-final-submission-comparison.json`)
shows redundant zero-encoder pending-writes submissions falling from 757 to
zero. Actual Core Animation presentation requests retain approximately 60 Hz:
59.9156 Hz before and 59.9125 Hz after. The final trace contains 755 unique
presentation requests over 12.585 seconds, all matched to submission command
IDs. Its scene and native-video submission rates are about 59.91 and 59.92 Hz.
The separate auxiliary presentation transitions remain at 755; their labels do
not represent another 755 actual presentations. Nonempty pending-writes work
also remains. The trace demonstrates removal of the targeted redundant submits,
not elimination of all WGPU overhead.

Actual requests are counted through Core Animation command IDs, not command
buffer labels. Scene/native classification uses workload-specific heuristics;
submission counts and trace instrumentation are not GPU utilization, decoded
frame counts or per-frame physical-scanout proof. A finite Apple M1 Metal probe
also passes eight real WGPU
surface presentations, including fresh initialization and resize, without GPU
validation errors. It checks API ordering and resource lifetime; it does not
replace the app motion and presentation checks above.

The retained GPU import regression checks fresh pixels through the reused
ImageId, different texture allocations, sampler changes and release of the last
owning import. It passes on Apple M1 Metal and fails its identity assertion when
the production reuse branch is disabled. Renderer tests also compare optimized
and original clear output with blending, stencil, clipping and target switches.
These checks qualify their specific behavior, not the entire UI or every driver.

The final old/new full-player comparison still contains 72 rounded-video-corner
pixels with a channel difference above 2/255, including 57 above 12/255 and a
maximum of 44/255. These are the accepted outline/antialiasing differences
between the direct signed-distance-field rounded mask and the original layer;
the final output is not pixel-identical. The video interior differs by at most
1/255, and the header, sidebar, watch controls and related-content regions by at
most 2/255. This result supports retained video detail, not identical corner
coverage.

The pixel-alignment fallback addresses a separate issue: optional cached layers
previously shifted compact related-card, avatar and focus geometry by about
half a physical pixel. Final cache-on/off compact related-card and focus regions
are exact matches; compact avatar and action regions differ by at most 1/255.
Focus-mask XOR is zero across all recorded stages, including compact, expanded,
resize and fullscreen layouts. Wider-layout comparisons retain small channel
rounding differences. Raw transparent background regions can also include the
animated ambient effect; the comparison records foreground and focus masks
separately. These checks do not claim that entire sequential screenshots match.

The final cached and uncached related-focus exercises, moving-playback exercise
and PiP exercise all exit successfully (`functional-final-results.json`). They
cover their finite scripted lifecycle paths, not every playback interaction.
The final workspace test log records 676 passing tests and seven ignored tests.
Separate runs pass 13 Slint renderer CPU tests and explicitly run the normally
ignored GPU import test on Apple M1 Metal. Clippy, 260 Python tests and the
45-test release-tool suite also pass. Ignored hardware tests are not counted as
successful executions unless explicitly run. These checks do not resolve the
existing 30 fps pacing issue or qualify untested hardware.

## Evidence identity and platform limits

A [sanitized measurement summary](evidence/2026-10-01-playback-fix-summary.json)
is retained with this report. The full local aggregates are in
`artifacts/playback-fix-2026-10-01/`.
The browser result is `artifacts/browser-baseline/run-ccfgn4so/result.json`.
Raw traces and process inventories are local diagnostics, not published here.
Useful reproducibility identities are:

| Input | SHA-256 |
| --- | --- |
| Shared local video fixture | `d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0` |
| Chrome executable | `4f82263da1a7ed4b71be76d6501bb7edc13c6573b8d950197147ee5776edcf3b` |
| Final-pair GPU baseline binary | `ca4f04b7bd284a32a50f08969fb2ae58c5c4c9bd72ec5d7abbee5d41811ed656` |
| Earlier GPU candidate binary only | `c9bf010b42a497de33352bfc837c554e24e51531b7e8f6709263dce0972eed83` |
| Final app binary | `258668f58c7806a2e99b9bf86f811728c201349b8be7ec6cdc85eb022211dd8e` |
| Final before/after source inventory | `07a0f3d3290c9bce99b74256746c25983a4607e70c9bde75b576452c22057709` |
| Final private native build result | `f2cae1f8fca024532efe79d2e7bc2fda7d805121af78365146d095f7a678ffe6` |

The intermediate source inventories record base commit
`932c5fe95bb583fbb6d4721d2cffed47731ddcb4` and changed working inputs. Their
`binary_build_association_verified` field is false. They document observed
working-tree inputs, not a verified build-to-source binding or tagged release.
Files named `final` do not by themselves establish that binding.

The later `final-build-identity.json` records the final app binary hash above,
matching before/after source inventories and 427 source inputs unchanged during
its build, together with the private native build-result hash. This is the
receipt for the final measured candidate; a tagged release and its independently
packaged binary still require their own identity evidence.

Physical playback and the hardware regressions above are limited to the inspected
Apple M1/macOS Metal host. Linux Vulkan and Windows D3D11/DX12 source, build and
CI checks have a different scope: they do not qualify physical decoder import,
presentation, visual equivalence, power or performance on those machines.
See [native media platform qualification](native-media-platforms.md) and
[performance qualification](performance.md). Release packaging and CI results
must be recorded separately from final measured playback results.
