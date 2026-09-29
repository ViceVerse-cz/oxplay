# Cosmetic clock staging experiment

`--stage-progress` is an explicit, default-off scheduling experiment. It keeps
one pending position/duration sample and applies its fixed-width elapsed,
remaining and seek-position properties during an already scheduled
`BeforeRendering` callback. It does not request a frame, add a timer, change the
4 Hz sampling cadence, delay transport commands, or mutate a catalog model.
Duration/range, transport state and all non-clock UI updates remain immediate.
No resource benefit is established yet.

## Source contract

The pinned Slint revision is `cf3b07d4917e6759a63b0c03913a2594ec653414`.
Its [OpenGL example](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/examples/opengl_texture/main.rs#L442)
sets a generated UI property inside `BeforeRendering`.
[FemtoVG](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs#L248)
invokes that callback before item traversal, inside the
[window draw dependency tracker](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/core/window.rs#L1868).
The tracker clears its old dependencies before evaluating the draw. This makes
cosmetic clock assignment a candidate for avoiding a separate dirty redraw;
it is not proof of lower draw counts or partial GPU redraw.

Only visible progress on an actively playing, loaded, presentation-ready native
display clock may stage a sample. Paused, buffering, seeking, hidden and unloaded
states apply immediately and discard deferred work. Load request identity and a
transport epoch are checked again at consumption. Accepted pause/seek operations
advance the epoch; pending absolute/coalesced/relative seek, stop and not-yet-
restarted loads close admission. Epoch saturation disables staging. Account
clear and rendering teardown explicitly discard pending values.

The pending scalar slot is removed before identity lookup or any generated
setter, preventing reentrant publication of the same sample. An empty slot
performs no snapshot clone or native property lookup during a video frame.
Shutdown counters distinguish samples staged, overwritten, consumed and
invalidated; consumed samples are not a count of changed UI properties. Existing
UI-assignment, catalog-change, video-render and presentation counters retain
separate meanings. No new raw media positions or account data are logged.

## Validation status

The initial staging workspace test run passed 231 Rust tests with three explicitly ignored
external tests. Strict workspace/all-target Clippy passed. Pure tests cover
bounded overwrite, default-off immediate application, empty-slot lookup,
reentrant consumption, transport/load invalidation and immediate transitions.
The native headless media regression covers accepted pause, absolute/coalesced
and relative seeks, volume, and source replacement identities.

The initial release native lifecycle/source-handoff checks and frozen off/on/on/off
comparison are recorded below. The subsequent position-cache correction has its own functional and resource
comparison below; the initial measurements do not qualify that changed version. Other cache experiments must remain off. Record
decoder, frame quality, CPU/RAM, UI draw callbacks and media frame notifications
separately. This candidate remains disabled by default.

## Initial native comparison — 2026-09-29

Source `7a4c9bf460eb72cbbc697c1853b6aef5a84395ba`, release SHA256
`914e39501233981b20c932af323c9ac65b6f95452c191f5c82473546991265e6`.
The staged native lifecycle (21.120 seconds) and video/audio/stop handoff
(30.239 seconds) passed. Staging was actually consumed, with zero unrelated
catalog changes. Lifecycle transitions are functional checks, not steady-state
frame-drop qualification.

Four same-binary off/on/on/off runs used ten seconds of warm-up and 60
one-second samples, the generated 90-second H.264 SDR1080p60/AAC48k clip,
1320×860 logical dark window, visible controls and 30 labeled related rows with
empty thumbnails. All cache flags were off. VideoToolbox and AVFoundation were
observed. Two persistent video targets totaled 7,936,128 bytes; this is explicit
target storage, not total graphics memory and must not be added blindly to RSS.
The new scrollable watch layout changes target geometry from the older related-cache
series, so its standalone/browser references are not an exact overhead baseline
for this series.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB | Warm UI draws / media notifications |
|---|---:|---:|---:|
| A1 off | 47.062 / 61.995 / 63.002 | 189.441 / 190.172 / 190.375 | 3959 / 3600 |
| B1 staged | 53.733 / 62.003 / 64.002 | 189.483 / 190.234 / 190.422 | 3945 / 3600 |
| B2 staged | 52.498 / 62.003 / 64.969 | 189.523 / 190.328 / 190.391 | 3895 / 3600 |
| A2 off | 51.912 / 62.998 / 63.275 | 189.554 / 190.297 / 190.406 | 3959 / 3600 |

All warm intervals added zero VO and decoder drops. Startup VO totals were
2/2/4/2; all runs exited cleanly without observed occlusion. CPU did not improve;
three means exceeded the 50% release ceiling, and all missed the 25% target.
Host-other load varied and is preserved in each raw sample. Bounded external
`caffeinate -d -u` was used, so this is not power or app sleep-prevention evidence.
The source manifest, functional logs, all samples and log hashes are indexed in
[the evidence summary](evidence/2026-09-29-clock-staging-initial-summary.json).

Source review found `apply_clock` read the generated position property immediately
before assigning it inside the draw callback. Slint's property read registers
the active tracker; changing it can invoke the tracker's dirty handler and
request another draw. The corrected candidate uses a Rust `Cell<f32>` for the same
position deduplication already used for text. Account clearing updates the
same caches, preserving exact zero at terminal clear. This source-backed
correction is distinct from the frozen measurements above; its separate results
follow below.


An independent read-only audit matched all 14 exported files byte-for-byte to
the original artifacts and verified their summary hashes. All 113 captured
source-input hashes match the recorded `7a4c9bf` commit. Sample means, the table's
10-to-70-second UI-draw/media-notification deltas and zero warm drop deltas were
rechecked from raw data. Each run has 60 samples, exit 0, no forced termination,
zero catalog changes and one fixture initialization reset. This verifies evidence
consistency, not independent compilation or resource qualification.

Host-other mean CPU was 74.60/58.91/60.15/57.79% of one logical core in ABBA order;
that varying shared/background activity is not an established explanation for
the differences. Staged lifetime counters were 5,381/5,371 submissions,
302/275 overwrites and 5,079/5,096 render consumptions for B1/B2. These count
requests to stage cached clock values across UI updates, not distinct native
position samples or changed properties, and do not imply a higher sampling timer.


## Corrected position-cache comparison — 2026-09-29

Source `9c70e8e3cd077a56c1b08fc3cd738cebdebcce3c`, release SHA256
`541f4cef58a9efb3e7f366a1b1b44938df9ea1cbcb05d4b7ee2ac0449575f84b`.
All 114 captured source-input hashes match that commit, and the recorded sampler
hash matches its committed `scripts/measure.py`. This is source association,
not an independent rebuild of native dependencies.

With staging enabled and stable-video-target disabled, the native local
lifecycle passed in 21.122 seconds and the video/audio/stop handoff passed in
30.301 seconds. Clock values were consumed in the render callback, with two
invalidated pending values recorded during the local lifecycle. Both runs kept
catalog changes at zero. The handoff log records blank images during audio-only
playback and after stop, followed by visible paused-video images after the
appropriate new loads. These are synthetic local functional checks, not human
account, A/V-perception or steady-state pacing qualification.

Four same-binary off/on/on/off runs then used the same local generated
90-second H.264 SDR1080p60/AAC48k clip, 1320×860 logical dark window, visible
controls and 30 labeled related text rows with empty thumbnails. All other
cache flags and stable-video-target were off. Each run supplied 60 one-second
process-tree samples after a ten-second warm-up, with actual VideoToolbox and
AVFoundation observed. Two persistent video targets totaled 7,936,128 bytes;
that is not total graphics memory. No profiler, screenshot or GPU query ran.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB | Warm UI draws / media notifications | Warm VO / decoder drops |
|---|---:|---:|---:|---:|
| A1 off | 53.106 / 62.408 / 64.211 | 189.391 / 190.125 / 190.141 | 3949 / 3600 | 0 / 0 |
| B1 staged | 45.250 / 62.000 / 63.000 | 189.301 / 190.000 / 190.078 | 3720 / 3600 | 0 / 0 |
| B2 staged | 51.548 / 61.689 / 64.001 | 189.215 / 189.859 / 190.000 | 3720 / 3600 | 0 / 0 |
| A2 off | 53.132 / 62.998 / 64.000 | 189.159 / 189.828 / 189.938 | 3946 / 3600 | 0 / 0 |

The warm counters are differences between the 10- and 70-second checkpoints.
Startup VO totals were 3/3/2/2 and did not increase afterward, including final
counters. All four exited 0, with no forced termination or observed occlusion.
Catalog changes stayed zero and resets remained at the one fixture initialization.
Staged lifetime submissions were 5,392/5,389, overwrites 313/308, and render
consumptions 5,079/5,081. These counters do not count distinct position samples
or changed UI properties.

The correction reduced observed warm UI draw callbacks in both staged runs,
while media notifications stayed constant. That does not establish partial GPU
redraw, lower GPU execution or lower energy. One staged mean CPU value passed the
50% release ceiling and the other failed; every run exceeded the 25% target.
The CPU result therefore remains **unqualified and default off**. Aggregate RSS
has shared-page limitations and must not be added blindly to target storage.
Host-other mean CPU was 58.61/77.08/57.52/57.19% of one core; varying background
activity is recorded without assigning it an unproven causal explanation.
The bounded external display-awake fixture again excludes a power or application
sleep-prevention claim.

[Corrected-series evidence summary](evidence/2026-09-29-clock-staging-corrected-summary.json)
indexes the exact source/provenance files, all samples, application/sampler logs
and functional metadata with SHA-256 values. Exported bytes were unchanged after
a bounded URL/path/header-marker scan; private-profile-path files were excluded.
The empty-thumbnail fixture, limited duration and developer-host setting do not
qualify real thumbnail churn, the library/soak gates or another platform. Older
standalone/browser video-target geometry still prevents an exact overhead
comparison with this series.

### Separate stable-target functional checks

The same `541f4cef…` binary also ran with stable-video-target enabled and clock
staging disabled. Local lifecycle and handoff tests exited 0 after 20.501 and
30.309 seconds respectively. The local run recorded 674 stable-target draws and
four target publications; the handoff recorded 276 stable-target draws, 24 private
target draws and three publications. Its audio-only/stop checkpoints remained
blank and subsequent paused video appeared, with four total file loads and no
catalog changes or resets. The local fixture had its single initialization reset.

These are separate flag selections, not a combined optimization or comparative
resource test. The local lifecycle deliberately resized and changed visibility
and recorded 88 final VO drops; no steady-playback conclusion follows. Stable-
target functional logs/metadata are also indexed by the corrected-series summary.
No stable-target CPU, power, long-running texture-lifetime or authenticated-media
qualification is claimed by this evidence.
