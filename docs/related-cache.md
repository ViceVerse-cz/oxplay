# Related-card cache experiment

`--related-cache` enables Slint's `cache-rendering-hint` on individual compact
`VideoCard` instances in the watch page's related list. It defaults off and is
mutually exclusive with `--ui-cache` and `--search-cache`. The browsing grid,
search field, sidebar, player surface, controls and account/library views keep
their existing rendering behavior. All screens still use the shared compiled
Slint UI. The measured comparison below did not establish a repeatable CPU benefit; the
experiment remains off. No power benefit is claimed.

## Why investigate this scope

The existing [search-cache stack profile](search-cache-profile.md) contains
`draw_text`, shared-Parley layout drawing, glyph-run drawing and generated
`VideoCard` geometry stacks while displaying 30 explicitly labeled fixture rows.
These are inclusive sampled stacks, not call counts or exclusive related-card
costs. They do not establish that every frame reshapes each title. The pinned
[FemtoVG text renderer](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/itemrenderer.rs#L230)
already passes Slint's text-layout cache into its text drawing function.
This experiment instead asks whether bypassing repeated drawing of unchanged
cards helps enough to justify additional graphics memory and invalidation work.
The earlier search-only experiment did not show repeatable CPU savings.

## Pinned cache contract

Source inspected: Slint `cf3b07d4917e6759a63b0c03913a2594ec653414`, the unchanged
workspace runtime/compiler revision.

- The [compiler](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/compiler/passes.rs#L189)
  lowers the hint to a `Layer`. The [Layer item](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/core/items.rs#L989)
  forwards pointer input and ignores keyboard/focus itself; existing card
  callbacks, focus scopes and accessible button labels remain in place.
- [ItemCache](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/core/item_rendering.rs#L57)
  tracks properties read while drawing. [render_layer](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/core/item_rendering.rs#L794)
  renders children on invalidation; a valid cache hit reuses the cached image.
- [FemtoVG](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/itemrenderer.rs#L1147)
  blends the layer and skips child drawing on a hit. It [reuses a same-size layer target](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/itemrenderer.rs#L986)
  after invalidation. Size changes may allocate replacement GPU targets.
- The [renderer clears layer caches on display-scale changes](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs#L264)
  and [releases component graphics resources](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs#L378)
  when item trees are destroyed. Visibility alone is not a promise of immediate
  GPU-cache release; retained invisible components may retain their cache.

Each card is its own cache scope. Its title, channel, duration, kind,
thumbnail-ready flag, image, theme colors, geometry, hover and focus participate
in normal drawing dependencies. Thumbnail completion still updates one stable
model row on the UI thread; it invalidates that card's layer. A stale thumbnail
cleared from a row likewise invalidates the placeholder/image choice. No catalog
reset, progress timer, animation, worker or additional network request is added.
There are no thumbnail video previews; decoded static thumbnail images remain
separate from the borrowed GPU video texture, which is outside every card layer.

The existing `ListView` virtualization and bounded backing model are unchanged.
Cache count is tied to instantiated/retained row components, not a new cache entry
for every catalog record. Native scroll/page-replacement tests must still verify
retention, focus, thumbnail changes and graphics-memory release.

## Expected costs and required validation

At the current wide-layout nominal card size of 344×106 logical pixels and 2×
scale, RGBA8 layer storage alone is 583,424 bytes, about 0.556 MiB per card;
six retained cards would require about 3.34 MiB. This is an arithmetic estimate,
not a graphics-allocation measurement. Responsive widths, scale, actual layer
bounds, retained rows, FBO/driver allocations and original thumbnail textures
change the real cost. Cached cards still require image blends during a window
redraw. This is not partial-window GPU redraw or a change to video transfers.

Compare baseline/related-cache/related-cache/baseline using one frozen release,
the same local clip, display, geometry, visible controls, related rows and settings.
Keep other cache flags off. Record whole-process-tree CPU/RAM, host context,
actual decoder, video drops, idle/paused behavior and separate graphics evidence.
Use profiling separately from timing runs. Exercise hover, keyboard focus/activate,
scrolling, row/image replacement, light/dark themes, resize, DPI and minimize/restore.
Test real asynchronous thumbnails as well as the explicitly labeled static demo.
The candidate stays opt-in until repeatable gains and interaction/resource gates
are demonstrated; the static playback comparison below does not complete those gates.

## Checks

On 2026-09-29, `cargo test --locked -p serein cli::tests` passed all five CLI
tests and compiled the shared Slint UI. `cargo clippy --locked -p serein
--all-targets -- -D warnings` and `cargo fmt --all -- --check` passed. The CLI
regression verifies the default, explicit activation and rejection of mixed or
duplicate experiment flags. The later native performance comparison is recorded below. Screenshots,
accessibility and the full interaction/cache-retention matrix remain unqualified.


## Native ABBA comparison — 2026-09-29

The frozen source was `5094c3902645976def9de5b827b9f42fc9aaaa59`, release SHA256
`662c0b7789cf013409da7df82cca2b9694b45f538fe1b24fe598797a7a4b3342`.
Four runs used off/related/related/off order, the same local 90-second H.264
1920×1080 SDR60/AAC48kHz clip, 1320×860 logical dark window and 30 explicitly
labeled related text rows with empty thumbnails. VideoToolbox and AVFoundation
were observed in every run. Only the related-card hint differed; no other cache,
profiler, screenshot or GPU query was enabled. Each run supplied ten seconds of
warm-up and 60 one-second process-tree samples. A bounded external
`caffeinate -d -u` fixture kept the display awake during each valid run; this
series provides no power measurement or application sleep-prevention validation.

| Run | CPU mean / p95 / peak, one-core % | RSS mean / p95 / peak, MiB | Warm VO / decoder drops |
|---|---:|---:|---:|
| A1 off | 53.987 / 62.999 / 64.001 | 188.939 / 189.672 / 189.781 | 0 / 0 |
| B1 related | 48.010 / 64.000 / 65.312 | 193.285 / 193.969 / 194.078 | 0 / 0 |
| B2 related | 53.281 / 63.000 / 64.182 | 193.080 / 193.875 / 194.062 | 0 / 0 |
| A2 off | 51.202 / 63.000 / 64.001 | 189.059 / 189.844 / 189.906 | 0 / 0 |

All four exited cleanly, without forced termination or an observed occlusion.
The 10-second, 70-second and final counters agree: zero additional warm VO drops
and zero decoder drops. Startup VO totals were 3/3/2/2 in run order; zero warm
drops does not mean zero lifetime drops. Catalog changes stayed zero, and resets
stayed at the single explicit fixture initialization. Lifetime UI draws were
5,543/5,568/5,588/5,574, while video draws were 5,076/5,076/5,077/5,077; these are
separate drawing counters, not evidence of partial-window GPU redraw. Each run
allocated two media targets totaling 8,614,016 bytes. Slint card-layer and driver
allocations were not independently measured and are not included in that count.

The first cached run fell below the 50% mean CPU release ceiling, but the second
did not. Every run exceeded the 25% target. This is **not a repeatable ceiling
pass** and does not establish CPU savings. Related-card caching stays off.
Pair-mean aggregate RSS increased by about 4.18 MiB with caching; shared-page
accounting and unmeasured graphics allocations prevent attributing that entire
difference to layer storage. Aggregate RSS is not macOS physical footprint and
must not be added blindly to unified GPU memory.

Host-other CPU means were 55.31/71.29/61.62/58.09% of one core. WindowServer and
coreaudiod were prominent; B1 also had more recorded `zeron` activity. These
shared/background processes are reported separately, not charged as exclusively
application-owned work. Their varying activity is a limitation, not an established
explanation for the differing cached means. Finite snapshots can miss short-lived
processes; newly observed VideoToolbox-service attribution is temporal.

An earlier A1 attempt is retained and excluded. Its short initial wake fixture
expired, the window reported occlusion at 8,928 ms, and playback was paused by the
10-second checkpoint and stayed paused. Its roughly 0.048% CPU mean is therefore
not a playback measurement. The later complete ABBA series uses the bounded
per-run awake fixture instead. The fixture is not evidence that application
sleep prevention alone would keep this environment visible.

[Summary, exported-file hashes and provenance](evidence/2026-09-29-related-cache-summary.json)
link all four full JSON sample sets and application/sampler logs, plus the invalid
attempt and its original provenance. The recorded binary, source, clip and sampler
hashes identify this series; later UI changes are outside its scope. Exported
bytes passed a bounded URL/path/credential-marker scan and were unchanged. The
static text-row fixture does not qualify real thumbnail churn, focus/hover,
scrolling, resize/DPI, minimize/restore, retained-layer release, energy, perceptual
A/V synchronization or the soak gate.
