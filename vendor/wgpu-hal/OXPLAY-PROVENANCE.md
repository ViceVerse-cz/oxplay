# Vendored WGPU HAL 30.0.1

This directory was extracted from the original cached crates.io archive
`wgpu-hal-30.0.1.crate`, not copied from potentially modified registry source.
Its SHA-256 was verified against the existing workspace `Cargo.lock` checksum
before extraction:

`b6b7fb58561a792bc237628ba0792e332de418fefe145f13b5ed8201e6d52f58`

Archive: https://crates.io/api/v1/crates/wgpu-hal/30.0.1/download

The archive's `.cargo_vcs_info.json` records upstream Git revision
`40f4a34ebaf56f9a046231f54125ad046239d3f3`, path `wgpu-hal`, from
https://github.com/gfx-rs/wgpu/tree/40f4a34ebaf56f9a046231f54125ad046239d3f3/wgpu-hal

All 86 archive files are retained, including the generated standalone manifest,
original manifest, original nested lockfile and complete MIT/Apache licenses.
The original `src/metal/adapter.rs` SHA-256 is
`50ae470024e8d0aa4f031f4d503aa73bfa8ecac69c4496f1080a07fdfc8a10c3`.
The root Cargo crates.io patch selects this exact package version; other WGPU
packages retain their original versions and registry sources.

The only production behavior change adds `TextureUses::PRESENT` to the Metal
adapter's ordered-texture-use mask. A helper supplies the mask to the existing
adapter trait method, with a policy regression covering its scope. HAL's ordered
use contract permits skipping a barrier only when both states are identical and
the usage is in that mask. WGPU core's `skip_barrier` implements precisely that
test. Thus `PRESENT -> PRESENT` stops creating a redundant pending-writes
submission before presentation; rendering, copying or initialization transitions
to `PRESENT` remain tracked. Metal's texture transition encoder already records
no GPU operations. No Vulkan, Direct3D, GLES or other backend capability changes.

WGPU's actual Metal `Queue::present` still creates its presentation command
buffer and schedules `presentDrawable`. The preceding rendered-scene submission,
queue order, tracked resource ownership, fence and completion handling remain
unchanged. Presenting an uninitialized surface must still execute WGPU core's
initialization clear and submit it; the clear changes the state, so the identical
state optimization cannot bypass it.

The policy unit test uses the compiled helper and checks every pair of individual
texture uses plus inclusive combinations. Only identical `PRESENT` may differ
from the original mask. Cargo will not test this dependency package with its
archived development dependencies outside the workspace. No dependency cycle or
new development dependency is introduced to work around that restriction.

The finite workspace example `crates/app/examples/metal_present_probe.rs`
instead exercises the actual compiled Metal adapter helper through its public
HAL trait, checks the ordered-use contract, and performs eight actual WGPU
surface presentations on Apple M1 Metal. Its real standalone CAMetalLayer
owns hardware drawables on the main thread; the owned Winit window stays hidden.
Hosted hidden windows are rejected as occluded by upstream Metal acquisition,
so this probe uses WGPU's supported CoreAnimationLayer surface target. The layer
outlives the surface and the probe creates no persistent playback diagnostic.

On 2026-10-01 it passed: a fresh 32px drawable presented without any scene
submission, three rendered presentations, a resize/reconfigure to 48px, a fresh
no-scene presentation and three further rendered presentations. Bounded GPU
polls completed with no validation or uncaptured GPU errors. Build with
`cargo build --offline --locked -j 4 -p oxplay --example metal_present_probe`,
then explicitly run `target/debug/examples/metal_present_probe` with an outer
15-second process timeout. Unsupported platforms report that fact; macOS
adapter mismatch, acquisition failure or GPU errors fail instead of skipping.

This probe qualifies initialization/presentation API ordering and resource
lifecycle, not the black pixel contents of the initialization clear or physical
scanout. Qualification still requires application surface lifecycle/motion
tests and a matched Metal trace: actual Core Animation requests and moving
frames must continue at the original cadence while the steady zero-encoder
`PendingWrites` submissions disappear. Offscreen image readback alone cannot
provide that presentation evidence.

The existing trace exported on 2026-10-01 showed 757 unique Core Animation
requests over 12.61775 seconds, rather than duplicate actual presentation. Its
757 redundant zero-encoder pending-writes submissions motivate this correction.
These counts do not establish a measured CPU/GPU improvement for the change.
