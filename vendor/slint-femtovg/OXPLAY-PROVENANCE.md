# Vendored Slint FemtoVG renderer

This directory copies `internal/renderers/femtovg` from Slint revision
`cf3b07d4917e6759a63b0c03913a2594ec653414` (package `i-slint-renderer-femtovg`
1.19.0). Source: https://github.com/slint-ui/slint/tree/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg

The original Rust files and README retain upstream copyright and license
headers. The root license summary and GPL, Royalty-free, Software and MIT
license texts are copied alongside them. The application uses the GPL option.
`LICENSE-GPL-3.0` also retains the upstream GPL text at the package root so
package notice collectors include the complete selected license.

The manifest replaces inherited workspace metadata and dependencies with their
exact upstream values. Internal Slint dependencies retain the same git revision;
only the FemtoVG renderer package is patched locally.

The renderer change bypasses the intermediate rounded-clip layer only for one
childless ordinary Image with an imported GPU texture, with uniform radius and
a containing rectangular ancestor under translation-only transforms. The image
geometry must exactly fill the clip, its source pixels must match the physical
size, and image fitting must produce unit scale with zero offset and no tiling.
The physical clip origin and extent must be integral pixels under the actual
Canvas transform. Fractional ancestor edges require at least one physical pixel
of clearance; integral containing edges may coincide with the clip. These guards
retain the original sampling and reject ancestor antialiasing intersections.
Other clips keep upstream layer
composition, including partial viewport intersections and overlapping children.
This avoids changing alpha composition when multiple children overlap rounded
antialiased edges. The app confines its steady video clip to that Image; visible
overlays restore the original outer group clip.

`OXPLAY_ROUNDED_IMAGE_CLIP=0` forces the original layer path for differential
pixel and resource checks. The environment is read once; the default enables
the guarded optimization. `OXPLAY_RENDER_DIAGNOSTICS=1` reports the first
eligible direct clip once, including its logical geometry and scale, to verify
the compiled item tree reaches this path. Ordinary runs do not log this event.
These controls do not establish quality or resource improvement on their own.

Optional `cache-rendering-hint` layers now require exact translation-only Canvas
transforms with finite integral physical origins. On each cache update, the
upstream child-bounds computation also checks that the physical layer origin and
extent are integral; negative child bounds and device scale are included in that
check. Bounds are traversed once on an update, with no added traversal for clean
cached layers. Rejected hints release their cached image and render children
normally. Mandatory opacity and rounded group-clip layers retain their original
composition. This guard addresses the compact watch layout's fractional-origin
AA differences: a cached focus line snapped about half a physical pixel, while
thumbnail/creator edges changed despite unchanged titles and focus state.
CPU policy tests cover transforms, local bounds, scaling and nonfinite inputs;
the app's finite related-focus snapshots provide the cache-on/off pixel check.

The WGPU window backend also batches its queued background clear with the final
scene submission when window and surface dimensions agree. Its public notifier
API exposes the instance, device and queue, without the window backbuffer. Native
video commands submitted by the notifier precede the scene's video texture reads
on that queue. OpenGL keeps its clear-before-callback contract. The caller-owned
WGPU texture backend and differing window/surface dimensions keep the original
early flush, because callbacks may hold the output texture and dimensional
transitions must preserve command interpretation. Setup, after-render callbacks,
surface acquisition, presentation and resource lifetimes keep their order.

Native WGPU source publication also reuses an item's existing imported FemtoVG
image when its owning actual `wgpu::Texture` handle and all image flags match.
WGPU texture dimensions and format are immutable for that allocation. The old
item cache value remains alive while its dependency update closure runs, so the
reuse check neither retains global ImageIds nor extends residency past the
owning item import. A different allocation or nearest/repeat sampler flags
imports a fresh image. Colorized images reuse only their original import and
recompute colorization as before. Source-property updates remain unchanged:
they still invalidate rounded and opacity layer pixels on each video frame.
This deliberately avoids changing Slint's WGPU Image equality or skipping
publication notifications, either of which could freeze cached layer contents.

An explicitly invoked, ignored hardware-GPU test checks changing pixels through
one reused ImageId against a fresh-import control, rejects different same-size
and resized allocations and sampler flags, and releases the last owning import.
Run it only after CPU/GPU resource measurements have stopped:
`cargo test --offline -j 4 -p oxplay -p i-slint-renderer-femtovg --lib same_wgpu_import_reads_fresh_pixels_and_rejects_new_handles_or_flags -- --ignored --test-threads=1 --nocapture`.
Selecting the app also enables its pinned WGPU texture-import feature on this
dependency package; Cargo cannot set dependency-package features directly.
On 2026-10-01 this regression passed on Apple M1 Metal, failed at the ImageId
equality assertion when its production reuse branch was temporarily disabled,
and passed again after byte-for-byte restoration. This proves the test detects
reimport churn and verifies fresh pixels for the retained import. It does not
establish a measured performance improvement or replace full app motion tests.

Original-file SHA-256 values (before local modifications):

- `Cargo.toml`: `0066cd7627b81096f963b37e80a86fe5a104e9f5922dcf91ed2df1049c9c9f5f`
- `README.md`: `d517affd6b3d811a95384d094217d9bae9c77615b41026121717e67e6a774fc7`
- `font_cache.rs`: `e07c6979a2ca41743d2df94a99f1574df693d941d3ea94f1c92287ced899e5c5`
- `images.rs`: `e7c6a42fc9f1b0aac5f3f527670e7ff8f2f24a8b3894f1c282e93630640d3767`
- `itemrenderer.rs`: `1f350a624cf35e2943a0281106b4fdb8625ed0eb55f62032f4c756abd5f8e0a4`
- `lib.rs`: `5a9b8acdcbb663073394546f6211319cd0daf40a9b80fa0169cf0bacb41cd44d`
- `opengl.rs`: `8a8393cbe71dcaf83e6665731807f7edb741e4e0b65066866e3162b4d6e43135`
- `wgpu.rs`: `a6440febe22ca8b4273011178de43a096c65a4079922d8f9d9206cbaa83df2a4`
