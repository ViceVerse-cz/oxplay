# macOS native video child: feasibility proposal

Status: **restricted local lifecycle and finite visual checks passed; production
and resource gates remain open, 2026-09-29**. The default borrowed-texture
presenter is unchanged.

Frozen `a45ff67692e0f25e22c188dc1583c3490d94a542` release
(SHA-256 `0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`)
passed two 14-stage lifecycle runs in 47.425 and 47.452 seconds. The second
retains 12 finite own-window captures. Separate ordinary-playback captures show
upright moving video in the native slot: 415 of 5,184 sampled video-interior
pixels changed above channel delta12, while 0 of 3,200 sidebar pixels changed.
This is an external diagnostic comparison, not application playback readback.
Both capture runs and all accompanying functional runs exited normally; owned
process-group absence was confirmed. No performance, cadence or perceptual
A/V qualification follows from these runs.

The [corrected evidence manifest](../evidence/2026-09-29-native-child-corrected-export.json)
retains full logs, summaries, source inventory, binary and original capture
hashes (manifest SHA-256
`5ac8763e8e3eaae12130ba32eb241c21f406e33ece54e0d74f0016c94738f8d6`).
Root captured 135 committed build inputs before and after the locked release
build; inventories were byte-identical. Later unbuilt changes are excluded.

The earlier d7c58acd run failed at23s because asynchronous user pause was undone
by fullscreen occlusion restoration. Its complete failed result remains in
[first-attempt evidence](../evidence/2026-09-29-native-child-first-export.json).
The corrected media-owned user intent/occlusion policy and its EOF cases are
recorded in [pause intent](../pause-intent.md).

## Authored implementation and its limits

`crates/media/src/native_child.rs` owns one render context on the AppKit main
thread and retains the existing `Player`. The same `renderer_attached` guard
rejects attaching this presenter alongside the borrowed-texture presenter.
`native_child.m` is surface glue only: a plain `NSView`, clipping parent, and an
explicit `NSOpenGLContext`. It contains no application controls, media commands,
or alternate application UI. It never replaces Winit's content view and returns
no hit-test target or first responder. Plain `NSView` avoids `NSOpenGLView`'s
implicit OpenGL update/reshape hooks; layer backing is established before
context attachment, matching the pinned Glutin surface ordering.

Every native mutation/destruction and render entry restores the previous CGL
context. A failed restoration terminates the diagnostic rather than permitting
subsequent Slint drawing with a foreign context. Geometry is validated before
resizing native buffers, then converted through AppKit backing coordinates.
The experimental limit is 4096 backing pixels per axis; exceeding it fails
without lowering quality. The reported two-color-buffer byte estimate excludes
mpv, driver and compositor allocations. Repeated identical geometry is cheap;
explicit hide invalidates the cached geometry for display/DPI reassociation.

Media callbacks retain the existing coalesced CVDisplayLink admission. Rendering
uses framebuffer zero, `FLIP_Y=1`, and nonblocking target-time mode; target-clock,
render and flush durations are recorded separately. This preserves an
experimental scheduling baseline, **not** a newly proven A/V timing contract.
No `report_swap`, application CPU frame readback, additional polling timer or
media-driven Slint redraw is introduced by the media adapter. Hidden/unready
frames are consumed with `SKIP_RENDERING` and no FBO parameter. A reveal requires
a fresh render/flush and the existing exact-load readiness gate. The terminal
VO-destruction privacy barrier remains necessary; ordinary first-frame readiness
is not universal proof for every EOF/no-frame input.

The host integration reserves shared Slint fullscreen controls outside the
native slot and must synchronously hide the child before overlapping popups,
information overlays, navigation or teardown. It is restricted to explicit
local-media diagnostics and does not admit remote/account playback. This
restriction is not a completed production overlay solution. Source-authored
geometry/deadline tests pass in the locked workspace suite. Native observations
now cover the finite lifecycle and captured visual states below. Other backing
scales/displays, full scroll clipping, all input/accessibility and overlay states,
pacing, perceptual A/V behavior and resource budgets remain unqualified. Slint snapshots alone
cannot establish the pixels of a separate native child.

## Decision and source constraints

A persistent Cocoa OpenGL child is a plausible **diagnostic candidate**, not a validated replacement. The application would retain its one compiled shared Slint UI and existing libmpv core. A macOS adapter would own only the video view, context, geometry, and presentation. No native buttons, menus, or account/browsing screens would be introduced.

The reviewed versions are Slint `cf3b07d4917e6759a63b0c03913a2594ec653414`, Winit 0.30.13 (`e9809ef54b18499bb4f2cac945719ecc2a61061b`), Glutin 0.32.3 (`20d1c103172aa4025f02cc94ca16a3169bea789c`), and installed mpv 0.41.0 headers. Registry source revisions came from their `.cargo_vcs_info.json` files.

* Winit exposes its `WinitView` through the AppKit raw handle, but its delegate's `view()` **unsafe-casts `NSWindow.contentView` to `WinitView`**. Replacing that content view with a wrapper to put video underneath the Slint view violates a concrete invariant. Raw-handle access does not authorize that reparenting scheme. [Pinned Winit source](https://github.com/rust-windowing/winit/blob/e9809ef54b18499bb4f2cac945719ecc2a61061b/src/platform_impl/macos/window_delegate.rs#L803-L806).
* Transparency alone does not resolve layering. Slint's CGL configuration requests transparency, and Glutin sets `kCGLCPSurfaceOpacity` to zero for a transparent configuration. FemtoVG nevertheless clears the framebuffer with the Slint window background; the current application also paints an opaque black video rectangle. A transparent underlay would require a valid native hierarchy and deliberate UI alpha composition, neither established here. [Slint context](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/backends/winit/renderer/femtovg/glcontext.rs), [Glutin context](https://github.com/rust-windowing/glutin/blob/20d1c103172aa4025f02cc94ca16a3169bea789c/glutin/src/api/cgl/context.rs), [FemtoVG clearing](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/internal/renderers/femtovg/lib.rs#L216-L250).
* A child **above** the unchanged Winit view avoids that content-view substitution, but can cover Slint pixels. Current fullscreen controls, technical information, and intersecting popups therefore prevent a drop-in replacement. The initial diagnostic could reserve nonoverlapping space using the same shared Slint components; production still requires a verified solution for every overlay and clipped/scrolling state. Hiding controls or replacing them with Cocoa controls is not a passing solution.
* The official mpv Cocoa example demonstrates `NSOpenGLView`, an independently created render context, default framebuffer 0, and `flushBuffer`. It does not demonstrate Slint layering, responsive scheduling, high-DPI correctness, or safe application teardown. Its native controls and direct main-thread drawing must not be copied as the application design. [Example at inspected revision](https://github.com/mpv-player/mpv-examples/blob/e0d1a84c99e8c469b58fde31a1541401acfb0eb2/libmpv/cocoa-rendergl/cocoa-rendergl.m).

## Proposed ownership and presentation contract

Keep the existing Winit content view and its first-responder/input responsibilities. Create and mutate the child view on the AppKit main thread. A validated hit-test policy must let existing Slint video clicks, scrolling, shortcuts, focus, and accessibility continue to work. Derive geometry from the actual visible player intersection, not just its unscrolled rectangle. Account for Winit's flipped coordinates, physical backing pixels, display-scale changes, fullscreen, occlusion, and expose events. Glutin/Winit explicitly establish layer backing before associating an OpenGL context; changing that association later is not a routine resize operation.

One serialized render owner holds a dedicated CGL context, current for every call involving its sole `mpv_render_context`. It renders directly to the child surface, without borrowing that surface into Slint. No render call occurs inside an mpv callback. Coalesced notifications schedule work; there is no permanent UI redraw timer. The first authored diagnostic uses the AppKit main thread for ownership and the existing asynchronous commands/property requests and nonblocking event drain. These APIs are explicitly safe on render API threads in installed `client.h`; initialization precedes the render context, and core destruction follows its destruction. No synchronous core command or cross-thread wait is added. A future worker owner cannot simply move Glutin's CGL methods to a thread: `make_current`, `update`, size queries and related methods can synchronously dispatch to main, which would deadlock if main simultaneously waited for that owner. Such a worker requires a separately validated asynchronous attachment/resize/shutdown protocol.

Preserve media timing: do not disable target-time waiting without an equivalent deadline scheduler, block the UI waiting for a frame, or equate submitting `flushBuffer` with measured display completion. If `report_swap` is used, report consistently according to the validated swap path. Track child renders/presents separately from Slint draws and UI assignments. [mpv threading, timing and swap contracts](https://github.com/mpv-player/mpv/blob/v0.41.0/libmpv/render.h), [OpenGL contract](https://github.com/mpv-player/mpv/blob/v0.41.0/libmpv/render_gl.h).

Candidate transfer path:

```text
compressed stream -> verified VideoToolbox decoder -> decoded buffers
 -> mpv CGL import/conversion [must trace and measure]
 -> child OpenGL framebuffer -> AppKit/WindowServer composition -> display
```

The design requires no application CPU readback or per-frame Slint image upload. That is not proof of every decoder/compositor transfer or end-to-end zero-copy. macOS direct hardware decoding through this mpv API requires a current CGL context. Verify the active decoder and actual transfer path again.

Keep the view/context persistent across control changes. Hide/clear account pixels synchronously at the presentation boundary on revocation and preserve exact-load/first-frame safeguards. Stop callbacks and drain ownership before freeing the render context; free it before destroying the mpv core. Only then release the CGL context and native view. Do not attach this candidate alongside a second render context on the same core.

## Restricted app admission and layout

`--native-video-child` is admitted only on macOS with an explicit `--local` file
and a **new absolute** `--data-root`; an existing directory or symlink is refused
before UI creation. It cannot combine URL/search, scoped transport, existing
smoke/soak modes, caches, staged progress, screenshots, helper overrides, or other
media rendering/timing environment experiments. Paused startup,
explicitly labeled related fixture rows, finite numeric `--diagnostics`, timed
exit, and window size/theme are permitted. A Slint texture screenshot cannot
capture the separately composed child, so `--snapshot` is rejected.

Admission is limited further to the previously measured synthetic MP4 with
SHA-256 `d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0`.
Before UI/engine creation the application opens a non-symlink regular file,
copies at most 128 MiB in 64-KiB chunks to a new 0600 file in the fresh private
profile, and hashes that same byte stream. Only an exact digest match publishes
the owned copy for playback. It never reopens the original path after hashing.
Normal shutdown/error cleanup unlinks the owned copy through the retained root
descriptor; a process crash may leave that private diagnostic copy. Arbitrary
subtitles are rejected in this first slice. Subtitle rendering remains an open
gate even if the caption popup lifecycle passes.

This is limited fixture admission, **not a comprehensive egress sandbox**.
The mpv 0.41 manual explicitly cautions that `access-references=no` may not be
respected by some FFmpeg demuxers; callback guards and arbitrary local filenames
alone cannot establish that references never reach the network. The diagnostic
does not broaden the default player's protocol policy or claim general
local-file network confinement.

The app adapter is `crates/app/src/native_child.rs`. Main creates either this
presenter or `GlPresenter`, then calls the same local-load function. Media wakes
service the native presenter directly instead of requesting a Slint redraw.
Geometry is synchronized at `BeforeRendering` after all potentially reentrant
Slint getters finish; no presenter borrow spans a UI setter. Teardown removes
the child before releasing the core.

The shared fullscreen layout reserves a 58-pixel bottom strip for controls and
an always-visible candidate warning. Technical information and app popups hide
the child; restoration requires fresh current-file rendering. Every explicit
popup-show path hides synchronously first. Pinned `lower_popups.rs` maintains
`PopupWindow.is-open` in the parent on every close path, so an aggregate tracks
the four app popups without inventing a `closed` callback. Watch ComboBoxes live
inside those tracked popups; other page controls cannot publish the child.

Input, scrolling, resize/move/backing-scale, activation and occlusion latch a
hidden state before layout recovery. Recovery requests one direct Winit redraw,
following the existing macOS workaround; media frames do not call that path.
Offscreen video is suppressed by actual rectangle intersection. Search, video
selection and account actions are disabled in UI and blocked again in Rust
callbacks; direct remote loading independently rejects this mode. No genuine
credentials or authenticated streams are admitted by this diagnostic.

CLI admission and completion tests pass in the locked workspace suite. The
Objective-C glue and compiled Slint UI build successfully in that suite; native
lifecycle/visibility observations are recorded below; complete input and resource qualification remain pending. Hiding video behind overlays is a
restricted diagnostic limitation, not a solved production overlay gate.

### Authored finite lifecycle diagnostic

`--native-video-child-smoke-test` additionally requires `--native-video-child`
and sets a minimum/default 46-second watchdog. Fourteen finite checkpoints
exercise the actual native presenter and shared Slint callbacks: initial
`videotoolbox` (not copy/unknown), 1920×1080 at 60fps ±0.1, AVFoundation output,
AAC at 48kHz, and publication observation; mute/pause followed by unmute with
unchanged volume; paused seek to 20s with a new native publication;
Info suppression; fresh paused reveal; resume and fullscreen with the reserved
control strip; pause and fullscreen exit; caption-popup suppression and Escape
close; Settings navigation and return; resize; minimize/restore; and stopped,
hidden terminal state. The local clip must last at least 60 seconds. Every
checkpoint requires exactly one native file load; hide/reveal checks compare
new publication counts rather than accepting old counts. Failure exits through
normal application teardown and remains an error even if later cleanup succeeds.
The lifecycle flag rejects `--paused` because it first verifies actual Playing;
ordinary restricted native-child mode can still start paused. The expected
audio/decoder values come from prior fixed-fixture evidence, including
`artifacts/raster-native-v2/local.log`, and were observed again in both corrected lifecycle runs. Engine output
metadata is not proof of audible output or perceptual synchronization.

The first finite native run failed stage23 as recorded above; later stages were
not reached in that attempt. Both corrected runs passed all14 checkpoints.
Stage23 observed Paused after fullscreen exit, and stage44 observed Idle with
zeroed clock, cleared decoder metadata, hidden child and inactive media clock.
The first corrected run reported 419 native publications, 12 geometry changes,
70 Slint draw callbacks, zero unrelated catalog notifications and one initial
fixture reset. These transition counters are not performance or display-cadence
results. A later generic shutdown stop does not invalidate the earlier confirmed
lifecycle stop.

Visual inspection of the external captures found upright video in normal,
fullscreen, resized and restored states, with shared Slint controls unobscured.
Info and caption-popup captures show a black video slot while the overlay is
visible; the Settings capture has no native video, and the stopped capture has
a black slot and zeroed clock. Fresh paused captures before/after Info
(window2/window4) and caption popup (window6/window8) are byte-identical.
These finite instants do not prove that every intermediate frame or every
possible overlay behaves correctly. Subtitle text itself, tooltip overlays,
other backing scales/displays, screen-reader/IME, hardware transfer accounting,
perceptual A/V, energy and resource budgets remain open. A successful
flush/reveal counter is not measured display completion.

Actual geometry matters: the default local run's normal window was clamped to
2640×1528 backing pixels at scale2, despite a larger requested size. Its video
was 664×373.5 logical pixels. The separate ordinary native captures were
2640×1592 including the title bar; use per-run geometry logs for comparisons,
rather than assuming requested window dimensions. The app still admits only
the exact synthetic fixture and labels the native child as unqualified.

## Go/no-go evidence

Proceed beyond a diagnostic only if all of these are demonstrated:

1. Video, every shared Slint control/overlay, keyboard/IME focus, scroll clipping, DPI changes, resize/fullscreen, pause/seek/subtitles, and teardown work in the same window without rebuilding the player.
2. A video-only frame updates the native surface without requesting a Slint window redraw; unrelated model notifications remain zero. UI interaction still redraws promptly.
3. Active hardware decoding, buffer transfers, frame pacing, A/V synchronization, account-pixel clearing, hidden-window behavior, and bounded surface lifetime are observed rather than inferred.
4. Matched whole-process CPU/RAM/GPU/energy measurements show repeatable improvement and meet the unchanged SPEC gates. Include WindowServer/compositor attribution limits and compare the same clip, size, controls, and focus state.

Failure of safe layering, input, ownership, or pacing is a no-go for this route. A successful isolated Cocoa video view alone is insufficient. Windows, X11, and native Wayland remain separately unvalidated; GStreamer remains a documented fallback candidate, not another default runtime.

## Remaining overlay and capture limits

Compact shared Actions include Slint tooltips. The current four explicit-popup
suppression flags do not cover tooltip visibility; their layering must be verified
or integrated before broader admission. Two ordinary watch captures cannot prove
fullscreen strip clipping, paused reveal pixels, popup/tooltip composition, hit
testing, backing-scale changes, cross-load stale-frame exclusion or A/V sync.
No screenshot loop is used for playback: the two external captures were finite
diagnostics excluded from performance measurement.

## Matched ABBA resource comparison on a45ff676

The same frozen release then ran default/native/native/default, each with ten
seconds of warm-up and sixty one-second resource samples. All four runs exited
normally with anchored application and wake-helper groups absent. The
[ABBA export](../evidence/2026-09-29-native-child-abba-export.json) retains all240
samples, full native/sampler logs, known diagnostic arguments with private paths
labeled, source/binary/native-library provenance and exact original/export
hashes. Its SHA-256 is
`29c136b025e0f8ba7feb9225b9823b9021bbf3bb97bec735c41fa9cb09176660`.
The [independent analysis](../evidence/2026-09-29-native-child-abba-analysis.json)
recomputed every mean/p95/peak, per-process sum, frame counter and log/source
hash association from the retained raw files.

All runs observed Apple M1 OpenGL4.1, direct `videotoolbox`, H2641920×1080 at60fps,
AAC48kHz/AVFoundation, speed1, volume100, unmuted, one file load, no cache pause
and no occlusion. Ten-,25- and70-second geometry checkpoints matched:
2640×1528 backing window at scale2, video logical `(260,76,664,373.5)` and clip
`(260,76,1032,640)`. The native backing was1328×747. Neither mode changed warm
geometry/event counters. Thirty labeled related fixture rows had **empty
thumbnails**, not thirty decoded raster images. The native warning also means
identical UI composition is not claimed. No screenshot or profiler ran within
these measurement intervals; prior visual observations are separate evidence.

Each cell below is mean / p95 / peak. CPU uses percent of one logical core.
RSS and OS physical footprint use MiB and are separate, non-additive ledgers.

| Run | App-attributed CPU | Aggregate RSS | OS physical footprint |
| --- | ---: | ---: | ---: |
| A1 default | 50.658 / 63.002 / 64.405 | 189.313 / 189.938 / 189.969 | 318.788 / 319.596 / 319.627 |
| B1 native | 46.437 / 54.210 / 56.001 | 189.863 / 190.766 / 190.813 | 312.799 / 313.674 / 316.627 |
| B2 native | 43.568 / 55.001 / 56.000 | 189.656 / 190.703 / 190.828 | 327.914 / 328.877 / 329.080 |
| A2 default | 48.784 / 61.393 / 61.932 | 189.265 / 189.938 / 189.984 | 318.448 / 319.377 / 319.455 |

The warm10→70-second VO-drop and decoder-drop deltas were zero in every run;
startup cumulative VO drops were2/3/3/2. Render-notification deltas were
3600/3600/3599/3600. Slint draw deltas were3838/240/239/3839, consistent with the
native path leaving the visible progress UI near4Hz. Unrelated catalog changes
remained zero with one initial fixture reset. These counters do not measure
actual display cadence or GPU rectangle coverage.

Native app-attributed mean CPU was lower in both repeats: the mode averages
were49.721% default and45.002% native, a4.718-percentage-point reduction (9.49%
relative) in this experiment. Both native means remained above the25% target;
this is not production or full-SPEC qualification. Two trials per mode do not
establish long-term repeatability. There was no consistent memory reduction;
native footprint varied substantially between its two trials.

Every sample contained one root process and one newly appearing, temporally
attributed VideoToolbox service, with stable sampled PIDs and rusage start
identities, no omitted entries and complete footprint observations. Two
preexisting VT services were excluded. Attribution is still not proof of
exclusive decoder ownership. Shared RSS pages may be double-counted, footprint
is not unique total system memory, and unified GPU allocation accounting remains
incomplete. Short-lived/reparented helpers and same-name PID reuse can escape
snapshot accounting; sequential ps/rusage reads are not atomic.

Shared host work matters. WindowServer appeared in all60 top-eight observations
per run, with mean CPU26.714/30.998/30.298/27.258%; coreaudiod was
6.505/7.265/7.135/6.477%. These are shared services outside the app total, not
costs exclusively attributable to this window. Total other-host CPU means were
50.380/51.249/60.192/49.195% (peaks135.086/78.307/164.555/73.670%). The app-side
reduction therefore does not demonstrate an equal system-wide or energy saving.
Top-eight omission never means a process did no work. No cargo/rustc entries
appeared in the recorded top-eight lists, which is narrower than proof of zero
foreign activity.

The next useful investigation is a separate, matched short sampling profile of
default and native root processes after warm-up, including thread-stack
attribution and the same geometry/decoder checks. Slint draws fell by roughly94%
while app CPU fell only9.49%, so another speculative redraw change would not
explain the remaining cost. Distinguish libmpv/driver submission, native flush,
audio/core threads and the4Hz shared UI before changing scheduling. The native
render/flush counters measure elapsed wall time, including waits; they cannot
be converted into CPU percentages. Independently measure compositor/system work
before claiming a power benefit. Such profiling is an attribution experiment,
not another resource qualification, and has not been run for this comparison.

## Readable Message popup: 16-stage frozen release validation

The `d78332c23602b1bfaec87688eccba657a208b9be` release, application SHA256
`e32cfa128282c781a80eeba06a8259207b07f54da946056b77185fa04efb88b9`,
passed the expanded16-stage local lifecycle in47.469seconds. The frozen build
record associates138 committed inputs, with matching before/after inventories;
the diagnostic rechecked binary and inventory identity before and after use.
All14 finite owning-window captures completed, and the app, capture tools and
bounded external display assertion groups were absent after anchored cleanup.
The owned copied fixture was removed normally. No provider/account content was
used.

The additional stages15/16 open the shared Message panel while paused, require
immediate native-child suppression, then dismiss it through Escape. Stage17
requires a fresh native publication. Capture5 shows the complete diagnostic
text wrapping inside the read-only Slint text control, with a visible Close
action and black native-video area. Capture6 shows the restored paused20-second
frame. Captures2/4/6 are byte-identical across Info and Message dismissal;
captures8/10 are identical across caption-popup dismissal. Inspected fullscreen,
resized and restored frames are upright with shared controls visible; Info,
Message, captions, Settings and stopped states do not display native video in
the inspected instants.

Initial observations were VideoToolbox H2641920×1080@60 and AVFoundation
AAC48kHz. Mute/unmute, paused seek, fullscreen/pause, minimize/restore and stop
checks passed with one load, zero unrelated catalog changes and one initial
fixture reset. Stage44 confirms the settled stop; the later generic shutdown
submits another stop, explaining its final `stop_pending=true`.

[The six-payload evidence manifest](../evidence/2026-09-29-ui-workflows-native-export.json)
(SHA256 `827a91422fe8dd0af9fc278ed588a8abdc6d9aecbf1209a072741b81b6534579`)
binds the exact log, source inventory, summary, capture analysis and documentary
harness, including hashes of all14 retained private PNGs. Ordinary captures are
2640×1592 including the title bar, fullscreen2880×1864, resized1920×1464;
ordinary logged backing video is1328×747. These screenshots do not qualify
continuous display cadence, perceptual A/V, long-message scrolling, text
selection, Close-button activation, screen readers/IME or resource budgets.
The native path remains an opt-in exact-local-fixture experiment.


### Pinned popup admission adapter

The restricted native-child experiment now has a compiled popup
admission adapter. It is not part of the captured release or qualified evidence
above. The source uses `WindowInner` through Slint's
`private_unstable_api::re_exports` at exact revision
`cf3b07d4917e6759a63b0c03913a2594ec653414`; this is an intentional unstable
integration that must be re-audited on every upstream update.

At that revision, `internal/compiler/passes/lower_tooltips.rs:58` rejects enclosing
references to a Tooltip's callbacks/properties, and line216 lowers tooltip show
and hide directly into popup operations. `internal/core/window.rs:670` exposes
the active popup stack internally. Winit does not override
`WindowAdapterInternal::create_child_window_adapter` (line249), whose default
returns no dedicated adapter; these popups use the same-window path at line2144.
Opening requests a redraw, and closing marks the popup region dirty and requests
another redraw at line2240.

The adapter conservatively suppresses native pixels while **any** Slint popup
is active, including built-in tooltips and widget dropdowns. It queries after
lazy geometry/visibility getters and again at native render admission. A failed
`try_borrow` also denies publication, and every borrow ends before native calls,
UI access or a presenter borrow. Reading this plain `RefCell` does not subscribe
a Slint property tracker or add a cache, model notification, timer or redraw
loop. Popup closure alone cannot reveal from a media wake: a popup-free
`BeforeRendering` must release the layout fence, after which the existing
fresh-render-before-reveal rule still applies. Existing explicit modal fences
remain in place.

Passing tests cover nested popup closure, contention failing closed and borrow
release; the locked development build and strict Clippy also pass. Actual delayed-hover tooltip visibility,
pointer routing, caption composition, native menus/dialogs outside this stack,
and fresh paused reveal require separate native validation. This change does
not broaden the exact local-fixture admission, enable account media, or adopt
the mpv timer patch.
