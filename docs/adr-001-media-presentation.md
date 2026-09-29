# ADR 001: first media presenter

Status: implemented experimentally; qualification open.

Use one Slint Winit/FemtoVG desktop OpenGL context and libmpv's render API.
The application retains an alternating pair of RGBA8 targets, sampled by the
same Slint window through the borrowed-texture contract. This avoids application
CPU frame readback while retaining shared Slint controls. Source/API review and
local VideoToolbox observation are documented in video-integration.md.

Keep GStreamer as a fallback research candidate, not a second runtime dependency.
Do not use the GStreamer software sample or an external mpv window as completion.
Do not claim that OpenGL selection proves hardware decoding or all-platform support.

A subsequent native run exhibited hundreds of media redraw requests but only two
Slint rendering callbacks. The adopted source routes macOS requests through an
NSView CADisplayLink throttle. The precise OS/throttle failure cause is unresolved.
An event-driven macOS media wake now calls the documented Winit window accessor's
`request_redraw()` directly; it introduces no frame timer and no render-callback
self-loop. Other targets use Slint's request_redraw. This workaround requires
release frame-pacing/idle validation and re-review with Slint updates. It does
not patch, substitute or advance the locked framework source.

Window occlusion pauses both audio and video and stops progress UI work. Restore
resumes only if playback was active before occlusion. Individual native platform
visibility behavior remains a qualification item. No screenshot/capture permission
is required by the application; OS screenshot tools used during development are
separate and cannot be assumed available in validation environments.

The later same-engine timing differential found native CGL interval 1 unreliable
on the tested macOS host. The media adapter now uses interval 0 with a bounded
CVDisplayLink gate; its callback only marks readiness/coalesces UI wakes. The
clock preserves phase through adjacent frames and stops after two empty refreshes
or a paused final frame. It restores the previous native interval at teardown.
The exact differential and current resource failure are in
[experiments/native-swap.md](experiments/native-swap.md). This corrects observed
warm frame throughput; A/V sync and the CPU ceiling remain unqualified.

The target dimension guard is currently 4096 pixels on each axis. RGBA8 costs
`width * height * 4` bytes per target: two maximum-size targets are 128 MiB,
with at most one newly allocated replacement coexisting until AfterRendering,
so resize can reach 192 MiB of application-owned media targets. The current
1384×778 pair is 8,614,016 bytes (~8.22 MiB). Engine, UI, driver and compositor
allocations are additional/possibly overlapping unified memory. An aggregate
admission limit must explicitly account for this resize overlap; it must not
silently reduce requested video quality or delete a still-borrowed target.
No 4K or maximum-target performance support is inferred from the smaller test.
