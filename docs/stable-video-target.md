# Stable admitted video target experiment

Status: implemented behind explicit `SEREIN_STABLE_VIDEO_TARGET=1`, default off. Focused ownership tests and the finite native checks below have passed; broader native and resource qualification remain open. The ABBA below did not establish a repeatable performance improvement. Latest documented functional checkpoint 83a07b4; Slint pinned cf3b07d4917e6759a63b0c03913a2594ec653414, FemtoVG 0.27.0, installed libmpv 0.41/API 2.5. The default retains alternating-target behavior until functional and resource validation. `RenderStats.stable_video_target` records selection; `stable_target_draws`, `private_target_draws` and `target_publications` are bounded saturating counters of successful render outcomes.

## Narrow change

Keep both existing persistent targets and PublicationPair. Stop alternating their identities only during already-admitted, same-load, same-size playback. This is NOT a single-target startup/transition design.

The presenter chooses displayed as the mpv render destination only if all are true:

- allow_publication is true and current_load_frame_ready() is true;
- accepted load ID is nonzero and equals published_load;
- displayed exists and its exact physical size matches the request;
- the same window/context still owns the presenter.

Otherwise choose next exactly as today. Private startup, unready/failed/audio-only loads, any load identity change, forbidden publication and resizing must never modify displayed. Render into next; only a successful, admitted publication swaps next/displayed and updates published_load. Private work returns None without swapping or creating a borrowed Image. A stable displayed render does not swap; return the identical borrowed image value (host still unconditionally installs every Some).

Paused redraws are allowed on displayed only under the same predicate: this redraws the same admitted native load. Paused replacement first renders privately into next. Stop makes identity zero and account crossing additionally awaits native VO destruction; this optimization must not weaken that barrier. Existing frame-ready predicate remains ordinary admission, not a universal EOF/privacy proof.

## Actual source contracts and ordering

Installed mpv/render.h:61–75 requires every OpenGL render call to use the same current context as creation and forbids callback-thread rendering. render_gl.h:37–59 describes use of that caller-controlled thread/context and state restoration. These requirements are already enforced by the rendering notifier and GlState guard.

Pinned mpv video/out/opengl/libmpv_gl.c wrap_fbo() wraps the caller FBO per render call. Its init explicitly treats libmpv as an external swapchain controlled by the caller, installs an empty swapchain callback set and disables mpv changing swap interval. vo_libmpv.c mpv_render_context_render() invokes the renderer on the calling thread. The final FBO is not asynchronously overwritten by the decoder callback. GPU execution remains asynchronous; API return is not a GPU completion fence.

Pinned Slint internal/renderers/femtovg/lib.rs:246–260 flushes queued background commands before BeforeRendering. It then traverses/draws items, flushes/submits all scene commands at321–322, drains texture cache and calls AfterRendering at333, before presenting. All these commands use the original context. Consequently the order is previous Slint texture sample -> current mpv FBO write -> current Slint texture sample. Ordinary framebuffer writes/texture sampling in this single GL command stream use GL's implicit command/hazard ordering; no CPU readback or glFinish is needed. This is not a cross-context or external-surface synchronization claim.

The current GlState reset unbinds Slint texture units before mpv renders, so merely retaining the image wrapper must not create a framebuffer feedback loop. Preserve that guard. Do not add shared contexts, deferred custom GPU queues, sampling of the render target inside the same mpv draw, or cross-window images under this proof. Such changes require a new synchronization design.

Slint core/graphics/image.rs:1389–1396 explicitly requires a valid texture created by the notifier's owning context and forbids sharing across windows. It does not require immutable pixel contents. The stock opengl_texture example alternates targets, so the stable-target behavior is a separately qualified application experiment rather than something already demonstrated by that example.

## Cache mechanism and simpler alternative

Slint core/graphics/image.rs:820 compares borrowed images by texture ID, dimensions and origin. BorrowedOpenGLTextureBuilder::build at1409 only constructs the ImageInner value; retaining a prebuilt Image per target saves no demonstrated expensive allocation and does NOT prevent invalidation when alternating A/B IDs.

ImageCacheKey::new at468 deliberately returns None for borrowed textures. FemtoVG itemrenderer.rs:1255–1310 nevertheless has a per-image-item graphics cache. Its tracked source property changes on every A/B swap, so it recreates Texture::new_from_image rather than obtaining a global cached texture. images.rs:165–189 imports the native texture into a new FemtoVG image wrapper. FemtoVG lib.rs:666–681 states these are bookkeeping allocations and do not own native GPU storage; gl_texture.rs:16–24 sets owned=false, and delete at304–307 only frees GL textures when owned=true.

Keeping the displayed ID stable avoids invalidating that per-item cache solely because of each new frame. The cached wrapper samples the original live GL texture on each draw. This targets CPU bookkeeping/property invalidation, not a full-frame GPU allocation or copy proven to occur today. Current historical profiles do not show import bookkeeping as a dominant cost; the ABBA below did not establish a repeatable CPU benefit.

Do not add a cache-rendering-hint to the video Image or any ancestor containing it: such a layer could freeze its pixels when the source value stays equal. The reviewed shared UI has one video Image, no colorize, and no cache hint on that Image or a video-containing ancestor. Colorizing creates an intermediate cached image and would also need explicit invalidation if enabled later. Preserve this restriction in code/docs and a structural regression check. Retaining separate Image values for A/B is simpler but would not remove the source changes or per-item import path; do not adopt it as an equivalent optimization.

## Lifetime and resource bounds

The caller-owned GL textures still outlive every borrowed Slint Image/native wrapper. On resize, create/replace only next; render and publish it, retain replaced storage through AfterRendering, then delete retired targets. Do not resize/delete displayed before Slint replaces its property and finishes sampling. On teardown clear the UI image, stop the display clock, destroy media render context with its GL context current, and delete targets in the existing order. Native wrapper deletion never transfers ownership or deletes the application texture.

Keep target bytes/allocation counters and current dimension/aggregate limits unchanged. Counters distinguish stable displayed renders, private next renders and new publications; target reuse is not reported as zero-copy. The spare target is still allocated/reused through the same existing path, preserving the pair and resize behavior.

## Qualification before enabling or benchmarking

1. Pure ownership tests: stable same-load writes displayed without swapping; private render, load change, allow_publication=false and resize always choose next; borrowed old target stays untouched; retirement follows successful replacement plus AfterRendering.
2. Native existing handoff sequence: video -> paused audio-only + resize (blank) -> paused video -> terminal stop (blank/clock stopped) -> paused video. Repeat quality, fresh-position refresh and subtitle reattachment regressions, resize/fullscreen/minimize/restore and hidden controls.
3. Verify moving output, subtitle updates and controls with all current optional UI cache flags. No video-containing cached layer or colorize allowed.
4. Same-binary off/on ABBA with matched clip, geometry, visible controls, decoder and display conditions. Record UI draws, actual media renders, dropped frames, process-tree CPU/RAM and target bytes separately. If useful, collect a separate short profile to verify native-wrapper import/cache invalidation disappears; profiler runs are not resource qualification.

Primary source locations examined locally: .upstream/slint at the SHA above; /opt/homebrew/include/mpv/render{,_gl}.h; cached exact0.41 /tmp/serein-vo-libmpv.c and /tmp/serein-libmpv-gl.c; Cargo registry femtovg-0.27.0/src/lib.rs and renderer/opengl/gl_texture.rs. No vendor modifications.

## Finite native evidence at 83a07b4

Source `83a07b4b493a7d411fc21fdbc55daf529596be7e` is associated with release
SHA256 `c2ff7d1c0f93145713e2de039e9aa6b574051afcba5d07ed331650c025568398`
through 115 captured source-input hashes. The central checkpoint reported 243 Rust
tests passing, three ignored, 63 Python tests, strict workspace Clippy and the
locked release build. These do not qualify other operating systems or native
dependency reproducibility. The [evidence index](evidence/2026-09-29-motion-library-summary.json)
retains source captures, unmodified sanitized logs, metadata and artifact hashes.

Two local lifecycle runs used the same generated 1080p60 clip, isolated profiles,
and bounded display-awake fixture. They took exactly two diagnostic snapshots,
at 1,400 and 2,400 ms, before pause/seek/resize operations. The interior video ROI
was sampled on a 128×72 grid. Changed grid pixels were 833/9,216 with default
alternation and 852/9,216 with stable targets. Both checks passed and exited 0
(default 20.995 seconds; stable 20.498 seconds). This demonstrates changed composed
video pixels across these two samples, including when the borrowed image identity
stays stable. It does not establish continuous compositor presentation, scanout
pacing, perceptual A/V synchronization, subtitle animation, or compatibility with
every optional UI cache combination. Snapshot readback is diagnostic-only and
these runs are excluded from performance measurements.

The lifecycle includes paused seek, resize, related-list scrolling, fullscreen
restoration, text-field shortcut exclusion, hidden controls and minimize/restore.
Both retained one file load, zero catalog changes and the one explicit fixture
reset. The stable run recorded 691 stable-target draws, one private-target draw
and four publications, against 683 publications in the default run. Both allocated
eight targets during intentional geometry changes. Final VO drops were 74 and 66;
those transition-heavy values are retained, not treated as steady-playback results.
The earlier same-source stable video/audio-only/stop handoff checks are recorded
in the [corrected-clock evidence](clock-staging.md).

A separate guest scoped-transport refresh run with stable targets exited 0 after
70.512 seconds. It deferred refresh while paused, resumed from a fresh native
position of 51.483 seconds, and completed two file loads with zero catalog changes
or resets. Fresh load-correlated audio probes reported decoder/output rates of
48,000 Hz and advancing audio PTS (79.1925295 to 84.1934780). The cached
`audio_sample_rate=0` remains unknown; it is not a finding of absent audio. The
four released range sources completed 8+1+2+17 reads with zero errors. Active
VideoToolbox H.264, Opus/AVFoundation and 1080p60 were reported. The presenter
recorded 3,509 stable draws, six private draws, two publications, two allocations
and 7,936,128 target bytes; final VO/decoder drops were 1/0. This is functional
source-refresh evidence, not a CPU/RAM, energy, power-management, real-account or
perceptual A/V qualification. The default remains off; the ABBA below failed to establish a repeatable resource benefit.


## Same-binary ABBA at 83a07b4

The [raw samples, provenance and audited summary](evidence/2026-09-29-stable-target-abba-summary.json)
use the same `c2ff7d1c…` release and the **83a07b4 motion-stable source capture**;
they do not use the older 9c70e8e stable-target functional capture. All 115 source
input hashes and the measurement script hash were independently checked against
83a07b4. Four 85-second local 1080p60 H.264/AAC runs used 1320×860 logical pixels,
dark theme and 30 labeled related fixtures with empty thumbnails. Clock staging
and all optional UI caches were off. No smoke readback, screenshot, profiler or
GPU query was enabled. Each sampled 60 one-second intervals after ten seconds of
warmup. VideoToolbox and AVFoundation were active in the recorded snapshots.

| Run | Stable target | CPU mean / p95 (% one logical core) | RSS mean / peak (MiB) | Warm UI draws / media notifications | Warm VO / decoder drops |
| --- | --- | --- | --- | --- | --- |
| A1 | Off | 54.385 / 63.613 | 189.306 / 190.141 | 3,952 / 3,600 | 0 / 0 |
| B1 | On | 44.431 / 62.328 | 189.365 / 190.094 | 3,945 / 3,600 | 0 / 0 |
| B2 | On | 54.146 / 61.999 | 189.311 / 190.078 | 3,937 / 3,600 | 0 / 0 |
| A2 | Off | 53.724 / 62.001 | 189.352 / 190.234 | 3,958 / 3,600 | 0 / 0 |

Warm deltas are between the application's ten- and seventy-second checkpoints;
media notifications are counted separately from actual render calls. Startup VO
drops were 3/3/2/3 and did not increase through those checkpoints or final exit.
Every process exited 0 without forced termination or an occlusion event. Each
run retained zero catalog changes, one fixture initialization reset and two target
allocations totaling 7,936,128 bytes. Final actual video renders were
5,076/5,076/5,078/5,076. Stable mode reduced borrowed-image publications from
5,075 in each A run to one in each B run, with 5,074/5,076 stable-target draws.
This verifies that the intended mechanism was exercised; it did not materially
reduce the whole-window draw counts in this sequence.

The favorable B1 CPU result did not repeat in B2. All four means exceed the 25%
one-logical-core target, and three exceed the 50% optimized release CPU ceiling; the
experiment remains default off. Sampled RSS is below the playback memory target,
which alone cannot pass the combined optimized gate. Separate host-other CPU
means were 60.776/84.895/57.777/60.339%; B1's higher host load is a confound, not
proof of why its application CPU was lower. No causal speedup is assigned to one
favorable run.

The sampler temporally includes new VideoToolbox services without proving their
exclusive ownership, may miss short-lived descendants, and aggregates RSS that
can double-count shared pages. Unified GPU memory overlaps RSS and is not added;
render-target bytes above are separate accounting. The bounded `caffeinate -d -u`
fixture kept the display awake for these runs and proves neither application
power management nor energy efficiency. These results do not qualify continuous
compositor scanout, perceptual A/V sync, real-account playback or another platform.


### Stable-target subtitle and clear-data follow-up

The same 83a07b4/C2 release completed the native guest subtitle/refresh/clear
lifecycle in 85.198 seconds with stable targets enabled and clock staging off.
Captions were selected, disabled, reselected and reattached after the second file
load. Clearing began at stage 67; by stage 80 the barrier had completed, playback
was idle, the displayed image/title/channel and UI clocks were neutral, and a
search submitted while clearing had not started another load. The final counter
remained two file loads, zero catalog changes and one explicit clear reset.
There were 525 stable-target draws, five private draws and two publications.
These are functional transition checks, not performance samples.

After exit, a read-only inspection of the isolated profile found schema version 3,
zero rows in local playlists/items/subscriptions/history, all five optional
privacy flags off, volume 100, system theme and 30-day retention. No VTT remained;
the only regular files were the local SQLite database and caption registry lock.
This establishes logical application cleanup, not forensic erasure or an account
sign-out test. [Sanitized log, metadata and storage inspection](evidence/2026-09-29-motion-stable-clear-summary.json)
retain source/hash provenance without the private profile path. No real account
or credential was used.
