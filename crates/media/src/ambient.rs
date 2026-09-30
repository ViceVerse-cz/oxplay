// SPDX-License-Identifier: GPL-3.0-or-later
//! Ambient-mode colour summary: a 16×9 RGBA8 average of the frame mpv just
//! rendered into the presenter target. The frame is reduced on the GPU with a
//! chain of LINEAR framebuffer blits (the first one sub-samples, each later one
//! is an exact 2×2 box average) and only the final 576 bytes are read back.
//! Steady-state reads go through a pixel-pack buffer guarded by a fence, so the
//! UI thread never waits for the GPU; they are collected on a later
//! BeforeRendering. No full frame is ever copied to the CPU.
use crate::{MediaError, Result, presenter::Target};
use glow::HasContext;
use std::{
    num::NonZeroU32,
    time::{Duration, Instant},
};

pub const AMBIENT_COLUMNS: usize = 16;
pub const AMBIENT_ROWS: usize = 9;
pub const AMBIENT_CELLS: usize = AMBIENT_COLUMNS * AMBIENT_ROWS;
const BYTES: usize = AMBIENT_CELLS * 4;
/// Reduction chain. The first level is small enough to be cheap for any
/// admitted target, large enough that each final cell averages 64 samples.
const LEVELS: [(u32, u32); 4] = [(128, 72), (64, 36), (32, 18), (16, 9)];
/// At most four samples per second while enabled and frames are rendered.
pub(crate) const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

/// Colour summary of one rendered frame. `pixels` are row-major and use the
/// same top-left orientation as the published video image (sRGB-encoded RGBA8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmbientSample {
    pub load_request_id: u64,
    pub pixels: [[u8; 4]; AMBIENT_CELLS],
}

/// Diagnostic counters; included in [`crate::RenderStats`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AmbientStats {
    /// GPU reductions issued (synchronous first-of-load reads included).
    pub issued: u64,
    /// Samples handed to the host.
    pub delivered: u64,
    /// Fence checks that found the GPU still busy (the read was retried later).
    pub busy_checks: u64,
    /// Readbacks dropped because sampling was disabled or the load changed.
    pub discarded: u64,
    /// Wall-clock µs spent issuing/collecting, only with OXPLAY_MEDIA_TIMING=1.
    pub cpu_us: u64,
    pub failed: bool,
}

/// Source rectangle of the displayed picture inside an mpv target (mpv's
/// default keepaspect letterboxing). Unknown video dimensions use the target.
pub(crate) fn content_rect(
    target_width: u32,
    target_height: u32,
    video_width: i64,
    video_height: i64,
) -> [i32; 4] {
    let (tw, th) = (i64::from(target_width), i64::from(target_height));
    if video_width <= 0 || video_height <= 0 || tw == 0 || th == 0 {
        return [0, 0, tw as i32, th as i32];
    }
    // Compare aspect ratios with integer cross-multiplication.
    let (w, h) = if video_width * th > tw * video_height {
        (tw, (tw * video_height / video_width).max(1))
    } else {
        ((th * video_width / video_height).max(1), th)
    };
    let (x, y) = ((tw - w) / 2, (th - h) / 2);
    [x as i32, y as i32, (x + w) as i32, (y + h) as i32]
}

/// A new load samples immediately; otherwise wait for the interval and for
/// any in-flight readback to be collected (never queue more than one).
pub(crate) fn sample_due(
    last: Option<(Instant, u64)>,
    now: Instant,
    load: u64,
    pending: bool,
) -> bool {
    match last {
        None => true,
        Some((_, last_load)) if last_load != load => true,
        Some((at, _)) => !pending && now.saturating_duration_since(at) >= SAMPLE_INTERVAL,
    }
}

struct Pending {
    fence: glow::NativeFence,
    load: u64,
}

pub(crate) struct AmbientSampler {
    levels: Vec<Target>,
    pbo: Option<glow::NativeBuffer>,
    pending: Option<Pending>,
    last: Option<(Instant, u64)>,
    ready: Option<AmbientSample>,
    stats: AmbientStats,
    measure: bool,
}

impl AmbientSampler {
    pub(crate) fn new(measure: bool) -> Self {
        Self {
            levels: Vec::new(),
            pbo: None,
            pending: None,
            last: None,
            ready: None,
            stats: AmbientStats::default(),
            measure,
        }
    }
    pub(crate) fn stats(&self) -> AmbientStats {
        self.stats
    }
    pub(crate) fn take(&mut self) -> Option<AmbientSample> {
        self.ready.take()
    }
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn due(&self, now: Instant, load: u64) -> bool {
        !self.stats.failed && sample_due(self.last, now, load, self.pending.is_some())
    }
    /// Forget timing so the next enabled frame samples at once (e.g. after the
    /// glow was hidden and shown again).
    pub(crate) fn rearm(&mut self) {
        self.last = None;
    }

    unsafe fn allocate(&mut self, gl: &glow::Context) -> Result<()> {
        if !self.levels.is_empty() {
            return Ok(());
        }
        unsafe {
            for (width, height) in LEVELS {
                match Target::create(gl, width, height) {
                    Ok(level) => self.levels.push(level),
                    Err(error) => {
                        self.delete(gl);
                        return Err(error);
                    }
                }
            }
            let pbo = gl.create_buffer().map_err(MediaError)?;
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pbo));
            gl.buffer_data_size(glow::PIXEL_PACK_BUFFER, BYTES as i32, glow::STREAM_READ);
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            self.pbo = Some(pbo);
        }
        Ok(())
    }

    /// # Safety
    /// Call inside the presenter's saved/reset GL state (scissor disabled,
    /// framebuffer and pack-buffer bindings restored afterwards), with the
    /// owning context current, after mpv rendered into `source`.
    pub(crate) unsafe fn sample(
        &mut self,
        gl: &glow::Context,
        source: glow::NativeFramebuffer,
        rect: [i32; 4],
        load: u64,
        now: Instant,
    ) -> Result<()> {
        let started = self.measure.then(Instant::now);
        // A newer load supersedes any in-flight read of the previous one.
        let first_of_load = self.last.is_none_or(|(_, last)| last != load);
        if first_of_load && let Some(old) = self.pending.take() {
            unsafe { gl.delete_sync(old.fence) };
            self.stats.discarded += 1;
        }
        let result = unsafe { self.sample_inner(gl, source, rect, load, first_of_load) };
        self.last = Some((now, load));
        if result.is_err() {
            self.stats.failed = true;
            unsafe { self.delete(gl) };
        }
        if let Some(start) = started {
            self.stats.cpu_us += start.elapsed().as_micros() as u64;
        }
        result
    }

    unsafe fn sample_inner(
        &mut self,
        gl: &glow::Context,
        source: glow::NativeFramebuffer,
        rect: [i32; 4],
        load: u64,
        synchronous: bool,
    ) -> Result<()> {
        unsafe {
            self.allocate(gl)?;
            // mpv does not restore scissor state; blits honour the scissor
            // test. The presenter's GlState restores both afterwards.
            gl.disable(glow::SCISSOR_TEST);
            gl.color_mask(true, true, true, true);
            let mut read = source;
            let mut src = rect;
            for level in &self.levels {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(level.fbo));
                let (w, h) = (level.width as i32, level.height as i32);
                gl.blit_framebuffer(
                    src[0],
                    src[1],
                    src[2],
                    src[3],
                    0,
                    0,
                    w,
                    h,
                    glow::COLOR_BUFFER_BIT,
                    glow::LINEAR,
                );
                read = level.fbo;
                src = [0, 0, w, h];
            }
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
            let pack = PackState::save_and_reset(gl);
            let (w, h) = (AMBIENT_COLUMNS as i32, AMBIENT_ROWS as i32);
            if synchronous {
                // One stall per load: the first colours are needed even if the
                // video is paused on its first frame and no later frame renders.
                let mut bytes = [0u8; BYTES];
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
                gl.read_pixels(
                    0,
                    0,
                    w,
                    h,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut bytes)),
                );
                pack.restore(gl);
                self.deliver(load, &bytes);
            } else {
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, self.pbo);
                gl.read_pixels(
                    0,
                    0,
                    w,
                    h,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::BufferOffset(0),
                );
                pack.restore(gl);
                let fence = gl
                    .fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)
                    .map_err(MediaError)?;
                self.pending = Some(Pending { fence, load });
            }
            self.stats.issued += 1;
        }
        Ok(())
    }

    fn deliver(&mut self, load: u64, bytes: &[u8; BYTES]) {
        let mut pixels = [[0u8; 4]; AMBIENT_CELLS];
        let (chunks, _) = bytes.as_chunks::<4>();
        pixels.copy_from_slice(chunks);
        self.ready = Some(AmbientSample {
            load_request_id: load,
            pixels,
        });
        self.stats.delivered += 1;
    }

    /// Non-blocking: reads the pack buffer only once its fence has signalled.
    /// Preserves the caller's pack-buffer binding and pack pixel-store state.
    ///
    /// # Safety
    /// Owning context current, inside BeforeRendering.
    pub(crate) unsafe fn collect(&mut self, gl: &glow::Context) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let started = self.measure.then(Instant::now);
        unsafe {
            match gl.client_wait_sync(pending.fence, 0, 0) {
                glow::ALREADY_SIGNALED | glow::CONDITION_SATISFIED => {
                    let pending = self.pending.take().expect("pending readback");
                    gl.delete_sync(pending.fence);
                    let previous = gl.get_parameter_i32(glow::PIXEL_PACK_BUFFER_BINDING);
                    let mut bytes = [0u8; BYTES];
                    gl.bind_buffer(glow::PIXEL_PACK_BUFFER, self.pbo);
                    gl.get_buffer_sub_data(glow::PIXEL_PACK_BUFFER, 0, &mut bytes);
                    gl.bind_buffer(
                        glow::PIXEL_PACK_BUFFER,
                        NonZeroU32::new(previous as u32).map(glow::NativeBuffer),
                    );
                    self.deliver(pending.load, &bytes);
                }
                glow::TIMEOUT_EXPIRED => self.stats.busy_checks += 1,
                _ => {
                    // WAIT_FAILED: drop this read; the next sample starts over.
                    let pending = self.pending.take().expect("pending readback");
                    gl.delete_sync(pending.fence);
                    self.stats.discarded += 1;
                }
            }
        }
        if let Some(start) = started {
            self.stats.cpu_us += start.elapsed().as_micros() as u64;
        }
    }

    /// Drop an in-flight read and any undelivered sample (sampling disabled).
    ///
    /// # Safety
    /// Owning context current.
    pub(crate) unsafe fn discard(&mut self, gl: &glow::Context) {
        if let Some(pending) = self.pending.take() {
            unsafe { gl.delete_sync(pending.fence) };
            self.stats.discarded += 1;
        }
        self.ready = None;
    }

    /// # Safety
    /// Owning context current; none of these objects may still be bound.
    pub(crate) unsafe fn delete(&mut self, gl: &glow::Context) {
        unsafe {
            self.discard(gl);
            for level in self.levels.drain(..) {
                level.delete(gl);
            }
            if let Some(pbo) = self.pbo.take() {
                gl.delete_buffer(pbo);
            }
        }
    }
}

/// The presenter's per-frame GL snapshot does not include pack pixel-store
/// state; save and reset it only around the (at most 4 Hz) readback.
struct PackState([i32; 4]);
const PACK_KEYS: [u32; 4] = [
    glow::PACK_ALIGNMENT,
    glow::PACK_ROW_LENGTH,
    glow::PACK_SKIP_PIXELS,
    glow::PACK_SKIP_ROWS,
];
impl PackState {
    unsafe fn save_and_reset(gl: &glow::Context) -> Self {
        let mut saved = [0; 4];
        for (value, key) in saved.iter_mut().zip(PACK_KEYS) {
            *value = unsafe { gl.get_parameter_i32(key) };
        }
        unsafe {
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
            gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::PACK_SKIP_PIXELS, 0);
            gl.pixel_store_i32(glow::PACK_SKIP_ROWS, 0);
        }
        Self(saved)
    }
    unsafe fn restore(self, gl: &glow::Context) {
        for (value, key) in self.0.into_iter().zip(PACK_KEYS) {
            unsafe { gl.pixel_store_i32(key, value) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_rect_matches_mpv_letterboxing() {
        // Same aspect: whole target.
        assert_eq!(content_rect(1600, 900, 1920, 1080), [0, 0, 1600, 900]);
        // 4:3 in 16:9 is pillarboxed horizontally.
        assert_eq!(content_rect(1600, 900, 640, 480), [200, 0, 1400, 900]);
        // 2.39:1 in 16:9 is letterboxed vertically.
        let [x0, y0, x1, y1] = content_rect(1920, 1080, 1920, 804);
        assert_eq!((x0, x1), (0, 1920));
        assert_eq!(y1 - y0, 804);
        assert_eq!(y0, (1080 - 804) / 2);
        // Theatre host wider than 16:9 pillarboxes 16:9 content.
        assert_eq!(content_rect(1264, 633, 1920, 1080), [69, 0, 1194, 633]);
        // Unknown dimensions fall back to the full target.
        assert_eq!(content_rect(800, 450, 0, 0), [0, 0, 800, 450]);
        // Extreme aspect never produces an empty source rectangle.
        let [x0, _, x1, _] = content_rect(10, 1000, 1, 100_000);
        assert!(x1 > x0);
    }

    #[test]
    fn sampling_is_rate_limited_but_immediate_for_a_new_load() {
        let t0 = Instant::now();
        assert!(sample_due(None, t0, 1, false));
        let last = Some((t0, 1));
        assert!(!sample_due(last, t0 + Duration::from_millis(100), 1, false));
        assert!(sample_due(last, t0 + SAMPLE_INTERVAL, 1, false));
        // An uncollected read blocks further reads of the same load.
        assert!(!sample_due(last, t0 + Duration::from_secs(5), 1, true));
        // A replacement load samples at once, superseding the pending read.
        assert!(sample_due(last, t0 + Duration::from_millis(1), 2, true));
    }
}
