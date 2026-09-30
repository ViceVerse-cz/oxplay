# Ambient mode

Ambient mode ("Filmové osvětlení" in Czech YouTube) draws a soft glow behind the
watch-page video whose colours follow the picture. It is on by default and can
be switched off in the player's **Playback settings → Ambient mode** row. The
preference lasts for the session only: persisting it would need a new
`local_preferences` column and SQLite schema migration, which is left for a
later change. Like YouTube, the glow appears only with the dark theme.

## Scope

The glow is shown on the regular and theatre watch page while a video is loaded
and presentable. It is hidden, and colour sampling stops, in fullscreen,
picture-in-picture, the corner mini-player, the light theme, loading/error
states, when the glow is scrolled off screen, and while the window is occluded.
The macOS native-child presenter experiment is unsupported and shows no glow;
any GL failure in the sampler disables ambient mode for that presenter without
affecting playback.

## Colour summary (media crate)

`GlPresenter` already renders every mpv frame into its own RGBA8 texture target.
When the host enables sampling (`set_ambient_sampling`), at most every 250 ms
and only for frames that are published (never private startup frames), it:

1. blits the displayed picture area of the target (excluding mpv letterbox
   bars, from the observed video dimensions) to a 128×72 texture with LINEAR
   filtering, then halves it three times (64×36, 32×18, 16×9) — each halving
   is an exact 2×2 box average, so every final cell averages 64 samples;
2. reads the final 16×9 RGBA8 result (576 bytes) into a pixel-pack buffer and
   inserts a fence;
3. on a later BeforeRendering, collects the bytes with a zero-timeout fence
   check, so the UI thread never waits for the GPU.

The first sample of each load (and after the glow is shown again) is read
synchronously instead, because a video paused on its first frame renders no
later frame to collect on. That is one bounded pipeline stall per load. All
work happens inside the presenter's existing saved/reset GL state; the pack
pixel-store values it changes are saved and restored around the read. No full
frame is copied to the CPU and no screenshot command is sent to mpv. Paused
video renders no frames, so sampling stops and the last colours remain.

GPU memory: four small targets (128×72 … 16×9, ~48 KiB) plus a 576-byte
buffer, allocated on first use and freed with the presenter. `RenderStats`
(printed as `render stats` at exit) reports issued/delivered/discarded samples,
busy fence checks and, with `SEREIN_MEDIA_TIMING=1`, CPU time spent.

A low-resolution `screenshot-raw` was rejected: it asks mpv to render or copy a
frame on its own thread, returns a full-size image through the client API and
would need CPU downscaling — far more work than four tiny blits.

## Glow rendering (app crate)

`ambient_ui.rs` converts each summary to 0–1 colours with a mild saturation
lift, then eases toward it with frame-rate independent exponential smoothing
(time constant 0.3 s, ≈95% after 0.9 s). The smoothed grid is rendered into a
76×48 RGBA image: a separable, normalised Gaussian extends each edge colour
outward, and a baked radial falloff (peak opacity 0.55) reaches exactly zero at
the image border. The central 64×36 texels sit behind the video host, so the
visible glow extends 6/64 of the video width (and 6/36 of its height) beyond
each edge — about 60–100 px in the default window, ~120 px in a large
theatre view. Slint scales the image with bilinear filtering; since the image is
already blurred, this stays soft.

The glow image is drawn directly above the canvas and below every other
element, so it never covers controls, text or popups. It reaches under the
header, tab strip and guide: while it shows, one canvas spans the whole window
and those chrome surfaces paint no background of their own, so the colour is
identical without the glow and a translucent window gets a single tint.

Colour updates are applied only in BeforeRendering calls that already publish a
new video frame, and at most ~24 times per second; there is no timer or
animation driving them, so the glow adds no redraws of its own while playing.
A replaced video fades the old glow out (600 ms Slint opacity animation) and
fades the new colours in once its first sample arrives, never blending the two
videos. These fades are the only animation-driven redraws. A quality change or
stream refresh is a new native load, so it also fades the glow out and in.

## Cost

Measured (release, reference M1 host, `ambient_ui::tests::update_cost`): one
glow update — summary conversion, smoothing step, 76×48 render and
`slint::Image` creation — has a median of 52 µs (21 batches of 200). At the
24 Hz cap that is about 1.25 ms per second (~0.13% of one core), and updates
stop once colours settle. It excludes FemtoVG's texture upload of the image.

Expected, not measured: per sample, ~20 GL calls issuing four blits that write
about 12,500 destination pixels and a 576-byte readback; the fence check is one
call per frame while a read is in flight. The bilinear-scaled 76×48 image adds
one textured quad and a 14.6 KB texture upload per glow update. No whole-app
CPU, GPU time, energy or frame-drop measurement was possible in this session:
the native presenter could not start without an awake display (`macOS create
display clock failed (-6661)`), so the feature is also visually unverified.
Ambient mode is on by default, so future playback resource samples include it;
compare against a run with it switched off before attributing changes.

## Validation

Unit tests cover letterbox source rectangles, sample rate limiting, colour
normalisation, smoothing convergence and frame-rate independence, glow
geometry/falloff and the Slint geometry constants. A compiled-UI test checks
the toggle and that sampling is requested only on the regular/theatre watch
page in the dark theme.
