// SPDX-License-Identifier: GPL-3.0-or-later
//! Two explicit local-smoke snapshots, never the playback/performance path.
//! Slint may re-render for take_snapshot; this proves changing pixels in its
//! composed video image, not compositor pacing, continuous motion or A/V sync.
use crate::{App, UiState};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{cell::RefCell, rc::Rc, time::Duration};

const GRID_WIDTH: usize = 128;
const GRID_HEIGHT: usize = 72;
const MAX_SNAPSHOT_PIXELS: u64 = 8_388_608; // at most 32 MiB temporary RGBA

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}
struct Frame {
    region: Region,
    window: (u32, u32),
    load: u64,
    pixels: Vec<[u8; 3]>,
    hash: u64,
}
#[derive(Default)]
struct Outcome {
    first: Option<Frame>,
    failure: Option<&'static str>,
    passed: bool,
}
pub struct Smoke {
    outcome: Rc<RefCell<Outcome>>,
    _timers: Vec<Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let outcome = Rc::new(RefCell::new(Outcome::default()));
        let mut timers = Vec::new();
        for (index, milliseconds) in [(0, 1_400), (1, 2_400)] {
            let app = app.as_weak();
            let state = Rc::downgrade(state);
            let result = outcome.clone();
            let timer = Timer::default();
            timer.start(TimerMode::SingleShot, Duration::from_millis(milliseconds), move || {
                if result.borrow().failure.is_some() { return; }
                let (Some(app), Some(state)) = (app.upgrade(), state.upgrade()) else { return; };
                // Do not hold any diagnostic RefCell borrow while take_snapshot
                // can reenter the rendering notifier.
                let sampled = capture(&app, &state);
                match sampled {
                    Err(error) => {
                        result.borrow_mut().failure = Some(error);
                        eprintln!("motion smoke unavailable/failed: {error}");
                    }
                    Ok(frame) if index == 0 => {
                        eprintln!("motion smoke sample_ms={milliseconds} roi={:?} grid={}x{} hash={:016x}", frame.region, GRID_WIDTH, GRID_HEIGHT, frame.hash);
                        result.borrow_mut().first = Some(frame);
                    }
                    Ok(frame) => {
                        let first = result.borrow_mut().first.take();
                        let compared = first.as_ref().ok_or("motion smoke first snapshot is missing")
                            .and_then(|first| compare(first, &frame));
                        match compared {
                            Ok(changed) => {
                                eprintln!("motion smoke PASS sample_ms={milliseconds} roi={:?} grid={}x{} hash={:016x} changed_pixels={changed}/{}; finite snapshot readback, excluded from performance", frame.region, GRID_WIDTH, GRID_HEIGHT, frame.hash, frame.pixels.len());
                                result.borrow_mut().passed = true;
                            }
                            Err(error) => {
                                result.borrow_mut().failure = Some(error);
                                eprintln!("motion smoke unavailable/failed: {error}");
                            }
                        }
                        // Both small grids are released here after comparison.
                    }
                }
            });
            timers.push(timer);
        }
        Self {
            outcome,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        let outcome = self.outcome.borrow();
        if let Some(error) = outcome.failure {
            return Err(error);
        }
        outcome
            .passed
            .then_some(())
            .ok_or("motion smoke ended before both local snapshots completed")
    }
}

fn capture(app: &App, state: &UiState) -> Result<Frame, &'static str> {
    let media = state.player.snapshot();
    if !app.get_loaded()
        || app.get_remote_video()
        || state.hidden.get()
        || !state.presentation_ready.get()
        || !state.player.current_load_frame_ready()
        || media.paused
        || media.width <= 0
        || media.height <= 0
        || !matches!(media.state, serein_media::PlaybackState::Playing)
        || media.error.is_some()
        || app.get_watch_offset() != 0.
    {
        return Err(
            "motion smoke requires an awake, playing local moving-video fixture in the initial unscrolled watch page",
        );
    }
    let size = app.window().size();
    if size.width == 0
        || size.height == 0
        || u64::from(size.width) * u64::from(size.height) > MAX_SNAPSHOT_PIXELS
    {
        return Err("motion smoke window exceeds the bounded snapshot dimensions");
    }
    let scale = f64::from(app.window().scale_factor());
    let region = interior_region(
        [
            f64::from(app.get_video_window_x()),
            f64::from(app.get_video_window_y()),
            f64::from(app.get_video_width()),
            f64::from(app.get_video_height()),
        ],
        (
            u32::try_from(media.width).map_err(|_| "motion smoke video width is invalid")?,
            u32::try_from(media.height).map_err(|_| "motion smoke video height is invalid")?,
        ),
        scale,
        (size.width, size.height),
        app.get_show_info() && !app.get_fullscreen_active(),
    )?;
    // Pinned core/api.rs: take_snapshot returns window RGBA; FemtoVG lib.rs
    // uses the window adapter's physical size. This is intentional GPU readback
    // exactly twice in --smoke-test, not a software video fallback.
    let pixels = app
        .window()
        .take_snapshot()
        .map_err(|_| "motion smoke snapshot unsupported or failed; motion is unverified")?;
    if (pixels.width(), pixels.height()) != (size.width, size.height) {
        return Err(
            "motion smoke snapshot dimensions changed or do not match physical window pixels",
        );
    }
    let sampled = sample(pixels.as_bytes(), size.width, size.height, region)?;
    let hash = pixel_hash(&sampled);
    // The full snapshot is dropped now; only a 27 KiB RGB grid remains.
    Ok(Frame {
        region,
        window: (size.width, size.height),
        load: media.load_request_id,
        pixels: sampled,
        hash,
    })
}

fn interior_region(
    video: [f64; 4],
    native: (u32, u32),
    scale: f64,
    window: (u32, u32),
    info: bool,
) -> Result<Region, &'static str> {
    if !scale.is_finite()
        || scale <= 0.
        || video.iter().any(|v| !v.is_finite())
        || video[2] <= 0.
        || video[3] <= 0.
        || native.0 == 0
        || native.1 == 0
    {
        return Err("motion smoke has invalid video geometry");
    }
    let [x, y, width, height] = video.map(|value| value * scale);
    // Match Image image-fit: contain, then keep the central 60% horizontally
    // and central 50% vertically, excluding letterbox bars and caption area.
    let fit = (width / f64::from(native.0)).min(height / f64::from(native.1));
    let content_width = f64::from(native.0) * fit;
    let content_height = f64::from(native.1) * fit;
    let content_x = x + (width - content_width) / 2.;
    let content_y = y + (height - content_height) / 2.;
    let left = (content_x + content_width * 0.2).ceil();
    let right = (content_x + content_width * 0.8).floor();
    let top = (content_y + content_height * 0.25)
        .max(if info { y + 132. * scale } else { content_y })
        .ceil();
    let bottom = (content_y + content_height * 0.75).floor();
    if left < 0.
        || top < 0.
        || right > f64::from(window.0)
        || bottom > f64::from(window.1)
        || right - left < GRID_WIDTH as f64
        || bottom - top < GRID_HEIGHT as f64
    {
        return Err("motion smoke video interior is clipped or too small after excluding overlays");
    }
    Ok(Region {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

fn sample(
    bytes: &[u8],
    width: u32,
    height: u32,
    region: Region,
) -> Result<Vec<[u8; 3]>, &'static str> {
    if u64::from(width) * u64::from(height) * 4 != bytes.len() as u64
        || region.width < GRID_WIDTH as u32
        || region.height < GRID_HEIGHT as u32
        || region.x.checked_add(region.width).is_none_or(|x| x > width)
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|y| y > height)
    {
        return Err("motion smoke pixel buffer or region is invalid");
    }
    let mut result = Vec::with_capacity(GRID_WIDTH * GRID_HEIGHT);
    for row in 0..GRID_HEIGHT {
        let y = region.y as usize + row * region.height as usize / GRID_HEIGHT;
        for col in 0..GRID_WIDTH {
            let x = region.x as usize + col * region.width as usize / GRID_WIDTH;
            let at = (y * width as usize + x) * 4;
            result.push([bytes[at], bytes[at + 1], bytes[at + 2]]);
        }
    }
    Ok(result)
}
fn pixel_hash(pixels: &[[u8; 3]]) -> u64 {
    pixels
        .iter()
        .flatten()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}
fn compare(first: &Frame, second: &Frame) -> Result<usize, &'static str> {
    if first.region != second.region
        || first.window != second.window
        || first.load == 0
        || first.load != second.load
    {
        return Err("motion smoke video identity or geometry changed between snapshots");
    }
    if first.pixels.len() != GRID_WIDTH * GRID_HEIGHT || second.pixels.len() != first.pixels.len() {
        return Err("motion smoke sampled grid is incomplete");
    }
    let changed = first
        .pixels
        .iter()
        .zip(&second.pixels)
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    if changed < (GRID_WIDTH * GRID_HEIGHT).div_ceil(100) {
        return Err(
            "motion smoke found insufficient changed video pixels: use the moving local test fixture; static clips do not pass this diagnostic",
        );
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(pixels: Vec<[u8; 3]>) -> Frame {
        Frame {
            region: Region {
                x: 0,
                y: 0,
                width: 128,
                height: 72,
            },
            window: (128, 72),
            load: 7,
            hash: pixel_hash(&pixels),
            pixels,
        }
    }
    #[test]
    fn comparison_rejects_static_noise_identity_and_geometry_changes() {
        let first = frame(vec![[20; 3]; GRID_WIDTH * GRID_HEIGHT]);
        let mut second = frame(vec![[28; 3]; GRID_WIDTH * GRID_HEIGHT]);
        assert!(compare(&first, &first).is_err());
        assert!(compare(&first, &second).is_err());
        second.pixels[..100].fill([40; 3]);
        assert_eq!(compare(&first, &second), Ok(100));
        second.load = 8;
        assert!(compare(&first, &second).is_err());
        second.load = 7;
        second.region.x = 1;
        assert!(compare(&first, &second).is_err());
    }
    #[test]
    fn region_uses_physical_pixels_and_excludes_info_caption_and_letterbox() {
        let region = interior_region(
            [100., 76., 692., 389.25],
            (1920, 1080),
            2.,
            (2640, 1720),
            true,
        )
        .unwrap();
        assert!(region.y >= 2 * (76 + 132));
        assert!(region.y + region.height <= ((76. + 389.25 * 0.75) * 2.) as u32);
        assert!(region.x > 200);
        assert!(region.x + region.width < 1584);
        assert!(interior_region([0., 0., 100., 50.], (1920, 1080), 1., (100, 50), false).is_err());
        assert!(
            interior_region(
                [-500., 0., 692., 389.],
                (1920, 1080),
                1.,
                (1320, 860),
                false
            )
            .is_err()
        );
    }
    #[test]
    fn sample_bounds_and_grid_ignore_pixels_outside_the_region() {
        let region = Region {
            x: 10,
            y: 10,
            width: 128,
            height: 72,
        };
        let mut bytes = vec![0u8; 150 * 100 * 4];
        let first = sample(&bytes, 150, 100, region).unwrap();
        bytes[..150 * 4].fill(255); // unrelated top-row UI change
        assert_eq!(sample(&bytes, 150, 100, region).unwrap(), first);
        bytes[(10 * 150 + 10) * 4] = 255;
        let second = sample(&bytes, 150, 100, region).unwrap();
        assert_eq!(second.len(), GRID_WIDTH * GRID_HEIGHT);
        assert_ne!(pixel_hash(&first), pixel_hash(&second));
        assert!(sample(&bytes, 149, 100, region).is_err());
        assert!(
            sample(
                &bytes,
                150,
                100,
                Region {
                    x: u32::MAX,
                    ..region
                }
            )
            .is_err()
        );
    }
}
