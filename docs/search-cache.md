# Search-field cache experiment

`--search-cache` is implemented as an opt-in `cache-rendering-hint` on the shared
search `LineEdit` only. It defaults off and cannot be combined with `--ui-cache`.
The candidate compiles in the locked release and passed the20-second native
lifecycle with search focus/text shortcut checks, resize and minimize/restore.
A same-source ABBA resource experiment is recorded below; full input and native
accessibility/IME qualification remain pending. It retains the existing
Fluent input, selection, context menu, IME and accessibility implementation; this
source observation does not qualify native screen-reader or IME behavior.

## Observed source mechanism

The inspected Slint checkout is clean at
`cf3b07d4917e6759a63b0c03913a2594ec653414`. `crates/app/build.rs` selects Fluent.
The search input uses a three-stop border gradient:
[Fluent LineEdit, line 44](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/compiler/widgets/fluent/lineedit.slint#L44)
and [palette, line 59](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/compiler/widgets/fluent/styling.slint#L59).
The application itself declares no gradients or shadows. Normal watch-page
slider borders have two stops at 0% and 100%; they are not the primary target.
Popup menus have shadows, but are absent from ordinary closed-popup playback.

`Cargo.lock` selects FemtoVG 0.27.0, checksum
`31eebf3ab1b76359ccd5f850c07a7c6b2722c64d9127c40e02358e83f3ff9b1a`.
Its installed crate records upstream revision
`0eb55780894a9200e9a70ca3d16bd1c641281a44` in `.cargo_vcs_info.json`.
Inspected source explains a concrete allocation mechanism:

- Slint [flushes before the rendering notifier and after UI drawing](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs#L242).
- Each FemtoVG [flush retires old gradient textures](https://github.com/femtovg/femtovg/blob/0eb55780894a9200e9a70ca3d16bd1c641281a44/src/lib.rs#L488).
- Its [gradient store](https://github.com/femtovg/femtovg/blob/0eb55780894a9200e9a70ca3d16bd1c641281a44/src/gradient_store.rs#L29)
  allocates a 256×1 RGBA texture for a missing multi-stop gradient. The intervening
  pre-notifier flush contains no search gradient and retires the preceding UI
  gradient, so the next uncached border draw recreates it.
- [Exactly two endpoint stops use a different paint path](https://github.com/femtovg/femtovg/blob/0eb55780894a9200e9a70ca3d16bd1c641281a44/src/paint.rs#L231).

The older `artifacts/display-clock-profile.sample.txt` contains a border-stroke
stack with 49 gradient-lookup and 40 `glTexImage2D` samples. Those are sampled
stack counts, not call counts, allocation rates, or percentages of execution
time. The mechanism above supports an experiment; it does not quantify savings.

## Cache behavior and cost

The compiler lowers the hint into a `Layer` wrapper (`internal/compiler/passes.rs:189`).
The wrapper forwards input and ignores focus itself (`internal/core/items.rs:990`).
`ItemCache` tracks drawing dependencies (`internal/core/item_rendering.rs:57`);
`render_layer` uses that cache at line 794. On a hit, FemtoVG blends the cached
image and skips child rendering (`internal/renderers/femtovg/itemrenderer.rs:1147`).
The image is a separate GPU resource: retiring a gradient lookup texture does
not erase the cached field pixels. On invalidation, a same-size layer texture is
reused (`itemrenderer.rs:986`), with RGBA8 allocation defined in `images.rs:76`.

Expected invalidations include caret blinking, text/preedit/selection changes,
focus, enabled state, clear-button hover, theme/accent, size and display scale.
The caret follows the platform's configured flash cycle; no new application
polling timer is added. The input's existing clipping bounds long text. Media
position and unrelated sidebar changes are not field drawing dependencies.

At the current maximum nominal field size of 568×40 logical pixels and 2× scale,
RGBA storage alone is 363,520 bytes (about 0.347 MiB), plus FBO/driver overhead.
Actual bounds and allocations still require measurement. There is one image
blend per window draw and offscreen rendering whenever the field invalidates.
Caching its parent would also cache the search button and add hover dependencies;
the direct `LineEdit` is the smaller experiment. This is neither partial-window
GPU redraw nor a video copy-path change.

## Qualification plan

Build one frozen release with both modes and keep whole-chrome caching off.
Compare baseline and `--search-cache` in coordinated ABBA runs with identical
clip, geometry, display, quality, controls and search-focus state. Record frame
loss, CPU, full process-tree memory, idle/paused behavior and separate graphics
allocation evidence. Profile separately to check whether the gradient-allocation
stack disappears between input changes; do not sample during benchmark runs.

Then exercise typing, long pasted text, selection/copy/paste, caret, clear button,
focus navigation, context menu, themes, resizing and minimize/restore. Native
IME and screen-reader qualification remain explicit pending gates. Retain the
candidate only if resource and frame-pacing results are repeatable and interaction
behavior remains correct. If it fails, a solid-border shared search component is
a possible later experiment, but would require full input/accessibility parity;
rewriting sliders or changing the renderer is not justified by this evidence.


## Measured candidate result

One ABBA series used frozen `e1725df` release
`ac87f2c3ad53ff066fcb5596ec9ffc614c5f0e81c737700ed25f1a7f8628ea80`,
normal/`--search-cache`/`--search-cache`/normal. Whole-chrome caching stayed off.
Each run sampled60 seconds after10 seconds warm-up using the same generated
1080p60H.264/AAC clip, actual VideoToolbox/AVFoundation,1384×778 video target and
30 labeled related rows with empty thumbnails. Other team native tests/builds
and downloads were held. GPU queries, profiling and screenshot capture were off.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB |
|---|---:|---:|
| a1-off | 51.458 / 60.921 / 62.000 | 184.909 / 185.516 / 185.578 |
| b1-search | 52.065 / 62.012 / 64.001 | 185.335 / 185.922 / 186.000 |
| b2-search | 51.773 / 61.996 / 63.988 | 185.987 / 186.672 / 186.734 |
| a2-off | 51.530 / 61.000 / 61.358 | 184.448 / 185.125 / 185.172 |

All warm VO/decoder drop deltas were zero, no occlusion was observed, and every
run exited cleanly. Catalog changes stayed zero with one initial fixture reset.
Each allocated two persistent video targets totaling8,614,016bytes. The search
cache showed **no repeatable CPU improvement** and every mean exceeded the50%
release ceiling. It remains an opt-in diagnostic and is not enabled by default.
The separate [native stack profile](search-cache-profile.md) supports the
predicted gradient-path change, but does not establish a resource benefit.

[Summary/provenance](evidence/2026-09-29-search-cache-summary.json) and the four
`2026-09-29-search-cache-*.json`/`.log` pairs retain complete measurements, host
audits and decoder/model counters. No active-account data was used.
