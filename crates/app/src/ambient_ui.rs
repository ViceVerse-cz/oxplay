// SPDX-License-Identifier: GPL-3.0-or-later
//! Ambient mode: a soft glow behind the watch-page video whose colours follow
//! the picture. The presenter supplies a 16×9 colour summary at most 4 Hz; this
//! module smooths it and renders a tiny 76×48 RGBA image that Slint scales up
//! with bilinear filtering behind the video host. Colour updates piggyback on
//! frames that already publish a new video image, so the glow adds no redraws
//! of its own while playing and freezes (keeping its colours) while paused.
use crate::{App, UiState};
use oxplay_media::{AMBIENT_CELLS, AMBIENT_COLUMNS, AMBIENT_ROWS, AmbientSample, GlPresenter};
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

/// Texels per sample cell in the glow image.
const SCALE: usize = 4;
/// Glow texels beyond each video edge. app.slint positions the image with the
/// same ratios: MARGIN / VIDEO_W of the width and MARGIN / VIDEO_H of the height.
pub const MARGIN: usize = 6;
pub const VIDEO_W: usize = AMBIENT_COLUMNS * SCALE; // 64
pub const VIDEO_H: usize = AMBIENT_ROWS * SCALE; // 36
pub const GLOW_W: usize = VIDEO_W + 2 * MARGIN; // 76
pub const GLOW_H: usize = VIDEO_H + 2 * MARGIN; // 48
/// Peak glow opacity next to the video edge.
const MAX_ALPHA: f32 = 0.55;
/// Gaussian spread, in glow texels (one sample cell is SCALE texels).
const SIGMA: f32 = 4.5;
/// Mild saturation lift so the glow reads as colour rather than grey.
const SATURATION: f32 = 1.3;
/// Exponential approach toward the newest sample (≈95% after 0.9 s).
const TIME_CONSTANT: f32 = 0.3;
/// Largest step applied at once, so resuming after a pause still eases.
const MAX_STEP: Duration = Duration::from_millis(100);
/// Image regeneration cap (~24 Hz) even for high-frame-rate video.
const MIN_IMAGE_INTERVAL: Duration = Duration::from_millis(40);
/// Matches `animate opacity` on the glow in app.slint: a replaced video's
/// colours fade out completely before the new video's colours fade in.
const FADE: Duration = Duration::from_millis(600);
/// Below this per-channel difference the transition is complete.
const SETTLED: f32 = 0.5 / 255.;

pub type Grid = [[f32; 3]; AMBIENT_CELLS];

/// Normalise a GPU summary: 0–1 channels with a mild saturation lift.
pub fn grid_from_sample(pixels: &[[u8; 4]; AMBIENT_CELLS]) -> Grid {
    let mut grid = [[0.; 3]; AMBIENT_CELLS];
    for (cell, pixel) in grid.iter_mut().zip(pixels) {
        let rgb = [pixel[0], pixel[1], pixel[2]].map(|c| f32::from(c) / 255.);
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        *cell = rgb.map(|c| (luma + (c - luma) * SATURATION).clamp(0., 1.));
    }
    grid
}

/// Frame-rate independent exponential smoothing of the colour grid.
#[derive(Clone)]
pub struct Smoother {
    current: Grid,
    target: Grid,
    has_colours: bool,
    settled: bool,
}
impl Default for Smoother {
    fn default() -> Self {
        Self {
            current: [[0.; 3]; AMBIENT_CELLS],
            target: [[0.; 3]; AMBIENT_CELLS],
            has_colours: false,
            settled: true,
        }
    }
}
impl Smoother {
    pub fn has_colours(&self) -> bool {
        self.has_colours
    }
    pub fn current(&self) -> &Grid {
        &self.current
    }
    pub fn settled(&self) -> bool {
        self.settled
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Jump straight to `grid` (first colours of a video, shown while invisible).
    pub fn snap(&mut self, grid: Grid) {
        self.current = grid;
        self.target = grid;
        self.has_colours = true;
        self.settled = true;
    }
    pub fn retarget(&mut self, grid: Grid) {
        if !self.has_colours {
            self.snap(grid);
        } else if grid != self.target {
            self.target = grid;
            self.settled = false;
        }
    }
    /// Advance by `elapsed`; returns whether the colours changed.
    pub fn step(&mut self, elapsed: Duration) -> bool {
        if self.settled {
            return false;
        }
        let dt = elapsed.min(MAX_STEP).as_secs_f32();
        let k = 1. - (-dt / TIME_CONSTANT).exp();
        let mut remaining = 0f32;
        for (current, target) in self.current.iter_mut().zip(&self.target) {
            for (c, t) in current.iter_mut().zip(target) {
                *c += (*t - *c) * k;
                remaining = remaining.max((t - *c).abs());
            }
        }
        if remaining < SETTLED {
            self.current = self.target;
            self.settled = true;
        }
        k > 0.
    }
}

/// Precomputed separable Gaussian weights and edge falloff. Rendering a grid
/// costs ~45k multiply-adds and allocates one 14.6 KB pixel buffer.
pub struct GlowRenderer {
    wx: Vec<[f32; AMBIENT_COLUMNS]>,
    wy: Vec<[f32; AMBIENT_ROWS]>,
    alpha: Vec<u8>,
}
fn weights<const N: usize>(texels: usize) -> Vec<[f32; N]> {
    (0..texels)
        .map(|x| {
            let p = x as f32 + 0.5;
            let mut w = [0.; N];
            for (i, weight) in w.iter_mut().enumerate() {
                let centre = MARGIN as f32 + (i as f32 + 0.5) * SCALE as f32;
                *weight = (-0.5 * ((p - centre) / SIGMA).powi(2)).exp();
            }
            // Normalised kernel regression: outside the video the nearest
            // edge cells dominate, extending edge colours outward.
            let sum: f32 = w.iter().sum();
            w.map(|weight| weight / sum)
        })
        .collect()
}
/// Opacity at a texel centre: full over the video, easing to exactly zero at
/// the outermost texels so the scaled image has no visible border.
pub fn falloff(x: usize, y: usize) -> f32 {
    let distance = |p: usize, len: usize| {
        let p = p as f32 + 0.5;
        (MARGIN as f32 - p).max(p - (MARGIN + len) as f32).max(0.)
    };
    let (dx, dy) = (distance(x, VIDEO_W), distance(y, VIDEO_H));
    let t = ((dx * dx + dy * dy).sqrt() / (MARGIN as f32 - 0.5)).min(1.);
    MAX_ALPHA * (1. - t) * (1. - t)
}
impl Default for GlowRenderer {
    fn default() -> Self {
        let alpha = (0..GLOW_H)
            .flat_map(|y| (0..GLOW_W).map(move |x| (falloff(x, y) * 255.).round() as u8))
            .collect();
        Self {
            wx: weights(GLOW_W),
            wy: weights(GLOW_H),
            alpha,
        }
    }
}
fn accumulate(sum: &mut [f32; 3], value: &[f32; 3], weight: f32) {
    for (s, v) in sum.iter_mut().zip(value) {
        *s += v * weight;
    }
}
impl GlowRenderer {
    pub fn render(&self, grid: &Grid) -> SharedPixelBuffer<Rgba8Pixel> {
        // Horizontal pass: sample rows -> glow columns.
        let mut rows = vec![[0f32; 3]; AMBIENT_ROWS * GLOW_W];
        let (lines, _) = rows.as_chunks_mut::<GLOW_W>();
        for (out, cells) in lines.iter_mut().zip(grid.as_chunks::<AMBIENT_COLUMNS>().0) {
            for (value, w) in out.iter_mut().zip(&self.wx) {
                for (cell, weight) in cells.iter().zip(w) {
                    accumulate(value, cell, *weight);
                }
            }
        }
        // Vertical pass into non-premultiplied RGBA8 with the baked falloff.
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(GLOW_W as u32, GLOW_H as u32);
        let pixels = buffer.make_mut_slice();
        let (lines, _) = pixels.as_chunks_mut::<GLOW_W>();
        for ((line, w), alpha) in lines
            .iter_mut()
            .zip(&self.wy)
            .zip(self.alpha.as_chunks::<GLOW_W>().0)
        {
            for (x, (pixel, a)) in line.iter_mut().zip(alpha).enumerate() {
                let mut rgb = [0f32; 3];
                for (row, weight) in w.iter().enumerate() {
                    accumulate(&mut rgb, &rows[row * GLOW_W + x], *weight);
                }
                let [r, g, b] = rgb.map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
                *pixel = Rgba8Pixel::new(r, g, b, *a);
            }
        }
        buffer
    }
}

#[derive(Default)]
struct Inner {
    renderer: Option<GlowRenderer>,
    smoother: Smoother,
    load: u64,
    /// Last value written to `ambient-ready`; never read back from Slint
    /// inside BeforeRendering (that would schedule a redundant draw).
    shown: bool,
    hidden_at: Option<Instant>,
    /// First colours of a new video, waiting for the old glow to fade out.
    held: Option<Grid>,
    last_step: Option<Instant>,
}
#[derive(Default)]
pub struct State {
    inner: RefCell<Inner>,
    reveal: Timer,
}

impl Inner {
    fn publish(&mut self, app: &App) {
        let renderer = self.renderer.get_or_insert_with(GlowRenderer::default);
        app.set_ambient_image(slint::Image::from_rgba8(
            renderer.render(self.smoother.current()),
        ));
    }
    /// Show held colours once any fade-out has finished. Returns the time
    /// still to wait, if any.
    fn reveal(&mut self, app: &App, now: Instant) -> Option<Duration> {
        let grid = self.held?;
        let wait = self
            .hidden_at
            .map_or(Duration::ZERO, |at| FADE.saturating_sub(now - at));
        if !wait.is_zero() {
            return Some(wait);
        }
        self.held = None;
        self.hidden_at = None;
        self.smoother.snap(grid);
        self.publish(app);
        self.last_step = Some(now);
        if !self.shown {
            self.shown = true;
            app.set_ambient_ready(true);
        }
        None
    }
}

/// Whether the presenter should sample: the glow is on screen and the window
/// is not occluded. Reads only Slint state that Rust never writes.
pub fn sampling_wanted(app: &App, state: &UiState) -> bool {
    !state.hidden.get() && app.get_ambient_active()
}

/// Call in BeforeRendering right after `GlPresenter::render`. `published`
/// is true when that call returned a new video image for this frame.
pub fn after_render(app: &App, state: &Rc<UiState>, presenter: &mut GlPresenter, published: bool) {
    let now = Instant::now();
    let mut inner = state.ambient_ui.inner.borrow_mut();
    let load = presenter.observed_load_request();
    if load != inner.load {
        // Replaced, stopped or first video: fade the old colours out; the new
        // video's first sample waits for that fade before appearing.
        inner.load = load;
        inner.smoother.clear();
        inner.held = None;
        inner.last_step = None;
        state.ambient_ui.reveal.stop();
        if inner.shown {
            inner.shown = false;
            inner.hidden_at = Some(now);
            app.set_ambient_ready(false);
        }
    }
    if let Some(sample) = presenter.take_ambient_sample() {
        accept(&mut inner, sample, load);
    }
    if inner.held.is_some() {
        if let Some(wait) = inner.reveal(app, now) {
            schedule_reveal(app, state, wait);
        }
        return;
    }
    if !published || !inner.shown {
        return;
    }
    let elapsed = inner.last_step.map_or(Duration::ZERO, |at| now - at);
    if elapsed < MIN_IMAGE_INTERVAL || inner.smoother.settled() {
        return;
    }
    inner.last_step = Some(now);
    if inner.smoother.step(elapsed) {
        inner.publish(app);
    }
}

fn accept(inner: &mut Inner, sample: AmbientSample, load: u64) {
    if sample.load_request_id != load || load == 0 {
        return;
    }
    let grid = grid_from_sample(&sample.pixels);
    if inner.smoother.has_colours() {
        inner.smoother.retarget(grid);
    } else {
        inner.held = Some(grid);
    }
}

/// One-shot: a paused first frame renders no later frame to reveal on.
fn schedule_reveal(app: &App, state: &Rc<UiState>, wait: Duration) {
    if state.ambient_ui.reveal.running() {
        return;
    }
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .ambient_ui
        .reveal
        .start(TimerMode::SingleShot, wait, move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            let _ = state
                .ambient_ui
                .inner
                .borrow_mut()
                .reveal(&app, Instant::now());
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(rgb: [f32; 3]) -> Grid {
        [rgb; AMBIENT_CELLS]
    }

    #[test]
    fn summary_normalises_and_lifts_saturation_without_changing_greys() {
        let mut pixels = [[128, 128, 128, 255]; AMBIENT_CELLS];
        pixels[0] = [200, 100, 50, 255];
        pixels[1] = [255, 0, 0, 0];
        let grid = grid_from_sample(&pixels);
        let grey = 128. / 255.;
        assert!(grid[2].iter().all(|c| (c - grey).abs() < 1e-6));
        let [r, g, b] = grid[0];
        assert!(r > 200. / 255. && b < 50. / 255., "saturation lifted");
        assert!(r > g && g > b, "hue order preserved");
        // Alpha is ignored and channels stay in range.
        assert_eq!(grid[1][0], 1.);
        assert!(grid.iter().flatten().all(|c| (0. ..=1.).contains(c)));
    }

    #[test]
    fn smoothing_eases_monotonically_and_settles_exactly() {
        let mut s = Smoother::default();
        assert!(!s.has_colours());
        s.retarget(uniform([0.; 3]));
        assert!(s.has_colours() && s.settled(), "first colours snap");
        s.retarget(uniform([1.; 3]));
        let mut previous = 0.;
        let mut steps = 0;
        while !s.settled() {
            assert!(s.step(Duration::from_millis(40)));
            let value = s.current()[0][0];
            assert!(value > previous && value <= 1.);
            previous = value;
            steps += 1;
            assert!(steps < 100, "never settled");
        }
        assert_eq!(s.current()[0], [1.; 3]);
        // ~95% within 0.9 s at 25 updates/s, fully settled a little later.
        assert!((20..=60).contains(&steps), "{steps} steps");
        assert!(
            !s.step(Duration::from_millis(40)),
            "settled colours are stable"
        );
    }

    #[test]
    fn smoothing_is_frame_rate_independent_and_long_gaps_still_ease() {
        let mut coarse = Smoother::default();
        let mut fine = Smoother::default();
        for s in [&mut coarse, &mut fine] {
            s.snap(uniform([0.; 3]));
            s.retarget(uniform([1.; 3]));
        }
        for _ in 0..3 {
            coarse.step(Duration::from_millis(80));
        }
        for _ in 0..12 {
            fine.step(Duration::from_millis(20));
        }
        assert!((coarse.current()[0][0] - fine.current()[0][0]).abs() < 1e-4);
        // A resume after a long pause advances by at most MAX_STEP.
        let mut resumed = Smoother::default();
        resumed.snap(uniform([0.; 3]));
        resumed.retarget(uniform([1.; 3]));
        resumed.step(Duration::from_secs(30));
        assert!(resumed.current()[0][0] < 0.5);
        // Retargeting to the same colours does not restart a transition.
        resumed.clear();
        resumed.snap(uniform([0.5; 3]));
        resumed.retarget(uniform([0.5; 3]));
        assert!(resumed.settled());
    }

    #[test]
    fn glow_extends_edge_colours_and_fades_to_nothing_at_its_border() {
        // Left half red, right half blue.
        let mut grid = uniform([0., 0., 1.]);
        for row in 0..AMBIENT_ROWS {
            for column in 0..AMBIENT_COLUMNS / 2 {
                grid[row * AMBIENT_COLUMNS + column] = [1., 0., 0.];
            }
        }
        let image = GlowRenderer::default().render(&grid);
        assert_eq!(
            (image.width(), image.height()),
            (GLOW_W as u32, GLOW_H as u32)
        );
        let px = |x: usize, y: usize| image.as_slice()[y * GLOW_W + x];
        let mid = GLOW_H / 2;
        // Colours outside the video follow the nearest edge.
        let left = px(MARGIN / 2, mid);
        let right = px(GLOW_W - 1 - MARGIN / 2, mid);
        assert!(left.r > 240 && left.b < 15, "{left:?}");
        assert!(right.b > 240 && right.r < 15, "{right:?}");
        // Opacity peaks at the video edge and reaches zero on every border.
        assert_eq!(px(MARGIN, mid).a, (MAX_ALPHA * 255.).round() as u8);
        assert!(px(MARGIN - 1, mid).a > px(MARGIN - 3, mid).a);
        for x in 0..GLOW_W {
            assert_eq!(px(x, 0).a, 0);
            assert_eq!(px(x, GLOW_H - 1).a, 0);
        }
        for y in 0..GLOW_H {
            assert_eq!(px(0, y).a, 0);
            assert_eq!(px(GLOW_W - 1, y).a, 0);
        }
        // Corners fall off radially, so they are dimmer than the edge centre.
        assert!(px(2, 2).a < px(2, mid).a);
        // Symmetric geometry.
        assert_eq!(falloff(1, mid), falloff(GLOW_W - 2, mid));
        assert_eq!(falloff(GLOW_W / 2, 1), falloff(GLOW_W / 2, GLOW_H - 2));
    }

    /// Opt-in release microbenchmark of the per-update CPU work:
    /// `cargo test --release -p oxplay ambient_ui::tests::update_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "timing microbenchmark; run explicitly in release"]
    fn update_cost() {
        let renderer = GlowRenderer::default();
        let mut pixels = [[0u8; 4]; AMBIENT_CELLS];
        for (i, pixel) in pixels.iter_mut().enumerate() {
            *pixel = [(i * 7) as u8, (i * 13) as u8, (i * 29) as u8, 255];
        }
        let mut smoother = Smoother::default();
        smoother.snap(grid_from_sample(&[[0; 4]; AMBIENT_CELLS]));
        let mut medians = Vec::new();
        for _ in 0..21 {
            let start = Instant::now();
            for _ in 0..200 {
                smoother.retarget(grid_from_sample(&pixels));
                smoother.step(Duration::from_millis(40));
                let image = slint::Image::from_rgba8(renderer.render(smoother.current()));
                std::hint::black_box(image);
                smoother.snap(grid_from_sample(&[[0; 4]; AMBIENT_CELLS]));
            }
            medians.push(start.elapsed().as_secs_f64() * 1e6 / 200.);
        }
        medians.sort_by(f64::total_cmp);
        eprintln!(
            "ambient update (summary+step+render+Image): median {:.2} µs",
            medians[10]
        );
    }

    #[test]
    fn slint_glow_geometry_matches_the_rendered_image() {
        let ui = include_str!("../ui/app.slint");
        for ratio in [
            format!("root.video-width * {MARGIN} / {VIDEO_W}"),
            format!("root.video-height * {MARGIN} / {VIDEO_H}"),
            format!("root.video-width * {GLOW_W} / {VIDEO_W}"),
            format!("root.video-height * {GLOW_H} / {VIDEO_H}"),
        ] {
            assert!(ui.contains(&ratio), "app.slint glow geometry lacks {ratio}");
        }
        assert!(
            ui.contains(&format!("duration: {}ms", FADE.as_millis())),
            "fade duration drifted"
        );
    }
}
