// SPDX-License-Identifier: GPL-3.0-or-later
//! Restricted macOS diagnostic. Main-thread native video surface only; the host
//! retains its one shared Slint UI. Never instantiate beside GlPresenter.
use crate::{MediaError, Player, Result, checked, ffi, render_wake};
use glow::HasContext;
use std::{
    ffi::{c_char, c_void},
    ptr::NonNull,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct NativeRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl NativeRect {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite)
            && self.width > 0.0
            && self.height > 0.0
            && self.width <= 4096.0
            && self.height <= 4096.0
            && self.x.abs() <= 65536.0
            && self.y.abs() <= 65536.0
    }
    fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}
/// Top-left logical content coordinates, before backing-scale conversion.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct NativeChildGeometry {
    pub video: NativeRect,
    pub clip: NativeRect,
    pub visible: bool,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeChildStats {
    pub media_wakeups: u64,
    pub render_notifications: u64,
    pub display_clock_ticks: u64,
    pub render_calls: u64,
    pub updates: u64,
    pub renders: u64,
    pub skipped_frames: u64,
    /// Successful flush/reveal operations, not measured display completions.
    pub publications: u64,
    pub geometry_changes: u64,
    pub render_us: u64,
    pub max_render_us: u64,
    pub flush_us: u64,
    pub max_flush_us: u64,
    pub target_samples: u64,
    pub early_us: u64,
    pub late_us: u64,
    pub max_early_us: u64,
    pub max_late_us: u64,
    pub backing_width: u32,
    pub backing_height: u32,
    /// Two RGBA8 color buffers estimate only; excludes driver/compositor/mpv.
    pub estimated_color_bytes: u64,
    pub visible: bool,
}
unsafe extern "C" {
    fn oxplay_child_is_main() -> i32;
    fn oxplay_child_create(parent: *mut c_void) -> *mut c_void;
    fn oxplay_child_context(surface: *mut c_void) -> *mut c_void;
    fn oxplay_child_window_number(surface: *mut c_void, number: *mut i64) -> i32;
    fn oxplay_child_geometry(
        surface: *mut c_void,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        cx: f64,
        cy: f64,
        cw: f64,
        ch: f64,
        pw: *mut i32,
        ph: *mut i32,
    ) -> i32;
    fn oxplay_child_hidden(surface: *mut c_void, hidden: i32) -> i32;
    fn oxplay_child_flush(surface: *mut c_void) -> i32;
    fn oxplay_child_destroy(surface: *mut c_void);
    fn CGLGetCurrentContext() -> *mut c_void;
    fn CGLSetCurrentContext(context: *mut c_void) -> i32;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}
fn main_thread() -> Result<()> {
    if unsafe { oxplay_child_is_main() } == 1 {
        Ok(())
    } else {
        Err(MediaError(
            "Native video child requires the AppKit main thread".into(),
        ))
    }
}
struct CurrentContext(*mut c_void);
impl CurrentContext {
    fn capture() -> Self {
        Self(unsafe { CGLGetCurrentContext() })
    }
    fn set(context: NonNull<c_void>) -> Result<Self> {
        let restore = Self::capture();
        if unsafe { CGLSetCurrentContext(context.as_ptr()) } != 0 {
            return Err(MediaError(
                "Cannot make native video context current".into(),
            ));
        }
        Ok(restore)
    }
}
impl Drop for CurrentContext {
    fn drop(&mut self) {
        if unsafe { CGLSetCurrentContext(self.0) } != 0 {
            // Continuing with a foreign context would corrupt Slint resources.
            // Context restoration failure is not a recoverable renderer error.
            std::process::abort();
        }
    }
}
struct Surface(NonNull<c_void>);
impl Drop for Surface {
    fn drop(&mut self) {
        let _restore = CurrentContext::capture();
        unsafe {
            oxplay_child_destroy(self.0.as_ptr());
        }
    }
}
unsafe extern "C" fn load_gl(_: *mut c_void, name: *const c_char) -> *mut c_void {
    // Darwin dlfcn.h RTLD_DEFAULT = (void *) -2. OpenGL is linked explicitly.
    unsafe { dlsym((-2isize) as *mut c_void, name) }
}
fn native_ok(result: i32, message: &'static str) -> Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(MediaError(message.into()))
    }
}

/// !Send/!Sync through Player. The host must synchronously hide before opening
/// any overlapping shared-UI overlay and drop before the native window/player.
pub struct NativeChildPresenter {
    player: Player,
    surface: Surface,
    cgl: NonNull<c_void>,
    context: NonNull<ffi::RenderContext>,
    geometry: Option<NativeChildGeometry>,
    observed_load: u64,
    fresh_required: bool,
    suppressed: bool,
    stats: NativeChildStats,
    graphics_info: String,
}
impl NativeChildPresenter {
    /// # Safety
    /// `ns_view` must be the live owning Slint window's AppKit NSView on the
    /// AppKit main thread. Keep that window alive until this presenter is dropped.
    /// The host admits local diagnostic media only and implements popup/clip policy.
    pub unsafe fn new(player: &Player, ns_view: NonNull<c_void>) -> Result<Self> {
        main_thread()?;
        if player.inner.renderer_attached.get() {
            return Err(MediaError("A presenter is already attached".into()));
        }
        let _restore = CurrentContext::capture();
        let surface = Surface(
            NonNull::new(unsafe { oxplay_child_create(ns_view.as_ptr()) }).ok_or_else(|| {
                MediaError("Cannot create experimental native video surface".into())
            })?,
        );
        let cgl = NonNull::new(unsafe { oxplay_child_context(surface.0.as_ptr()) })
            .ok_or_else(|| MediaError("Native video surface has no CGL context".into()))?;
        let _current = CurrentContext::set(cgl)?;
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| {
                load_gl(std::ptr::null_mut(), name.as_ptr()).cast_const()
            })
        };
        let version = gl.version();
        if version.is_embedded || (version.major, version.minor) < (3, 3) {
            return Err(MediaError(
                "Native video requires desktop OpenGL 3.3 or later".into(),
            ));
        }
        let graphics_info = unsafe {
            format!(
                "{} | {} | {}",
                gl.get_parameter_string(glow::VENDOR),
                gl.get_parameter_string(glow::RENDERER),
                gl.get_parameter_string(glow::VERSION)
            )
        };
        let mut init = ffi::GlInit {
            get_proc_address: load_gl,
            context: std::ptr::null_mut(),
        };
        let mut params = [
            ffi::RenderParam {
                kind: 1,
                data: c"opengl".as_ptr().cast_mut().cast(),
            },
            ffi::RenderParam::new(2, &mut init),
            ffi::RenderParam::end(),
        ];
        let mut raw = std::ptr::null_mut();
        unsafe {
            checked(
                ffi::mpv_render_context_create(&mut raw, player.inner.raw, params.as_mut_ptr()),
                "Create native child media renderer",
            )?;
        }
        let context = NonNull::new(raw)
            .ok_or_else(|| MediaError("Empty native media render context".into()))?;
        let clock = match unsafe { crate::macos::PresentationClock::new(player.inner.wake.clone()) }
        {
            Ok(clock) => clock,
            Err(error) => {
                unsafe {
                    ffi::mpv_render_context_free(context.as_ptr());
                }
                return Err(error);
            }
        };
        unsafe {
            ffi::mpv_render_context_set_update_callback(
                context.as_ptr(),
                Some(render_wake),
                Arc::as_ptr(&player.inner.wake).cast_mut().cast(),
            );
        }
        *player.inner.presentation_clock.borrow_mut() = Some(clock);
        player.inner.renderer_attached.set(true);
        Ok(Self {
            player: player.clone(),
            surface,
            cgl,
            context,
            geometry: None,
            observed_load: 0,
            fresh_required: true,
            suppressed: true,
            stats: NativeChildStats::default(),
            graphics_info,
        })
    }
    pub fn stats(&self) -> NativeChildStats {
        let mut stats = self.stats;
        stats.media_wakeups = self.player.inner.wake.count.load(Ordering::Relaxed);
        stats.render_notifications = self.player.inner.wake.render_count.load(Ordering::Relaxed);
        stats.display_clock_ticks = self.player.inner.wake.clock.ticks.load(Ordering::Relaxed);
        stats
    }
    pub fn graphics_info(&self) -> &str {
        &self.graphics_info
    }
    /// Explicit external diagnostic capture target. This reads only the retained
    /// parent's NSWindow.windowNumber; it neither enumerates nor captures windows.
    pub fn window_number(&self) -> Result<u32> {
        main_thread()?;
        let _restore = CurrentContext::capture();
        let mut number = 0i64;
        native_ok(
            unsafe { oxplay_child_window_number(self.surface.0.as_ptr(), &mut number) },
            "Native child owning window number is unavailable",
        )?;
        u32::try_from(number)
            .ok()
            .filter(|number| *number != 0)
            .ok_or_else(|| {
                MediaError("Native child owning window number is outside capture range".into())
            })
    }
    /// Immediately removes native pixels without waiting for a media callback.
    pub fn hide(&mut self) -> Result<()> {
        self.suppressed = true;
        // Explicit host hiding also invalidates drawable/backing information.
        // Call on window move, scale/display change and restoration; the next
        // identical logical geometry still reassociates its native drawable.
        self.geometry = None;
        self.hide_surface()
    }
    fn hide_surface(&mut self) -> Result<()> {
        main_thread()?;
        let _restore = CurrentContext::capture();
        native_ok(
            unsafe { oxplay_child_hidden(self.surface.0.as_ptr(), 1) },
            "Cannot hide native video child",
        )?;
        self.stats.visible = false;
        self.fresh_required = true;
        Ok(())
    }
    /// Idempotent logical geometry update. A changed surface remains hidden
    /// until render has drawn/flushed fresh pixels for the admitted current load.
    pub fn set_geometry(&mut self, geometry: NativeChildGeometry) -> Result<()> {
        main_thread()?;
        if self.geometry == Some(geometry) {
            self.suppressed = !geometry.visible;
            return Ok(());
        }
        self.hide()?;
        self.geometry = None;
        if !geometry.visible {
            self.geometry = Some(geometry);
            return Ok(());
        }
        if !geometry.video.valid()
            || !geometry.clip.valid()
            || !geometry.video.intersects(geometry.clip)
        {
            self.geometry = None;
            return Err(MediaError(
                "Invalid or nonintersecting native video geometry".into(),
            ));
        }
        let _current = CurrentContext::set(self.cgl)?;
        let v = geometry.video;
        let c = geometry.clip;
        let (mut width, mut height) = (0, 0);
        native_ok(
            unsafe {
                oxplay_child_geometry(
                    self.surface.0.as_ptr(),
                    v.x,
                    v.y,
                    v.width,
                    v.height,
                    c.x,
                    c.y,
                    c.width,
                    c.height,
                    &mut width,
                    &mut height,
                )
            },
            "Cannot update native video geometry",
        )?;
        if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
            self.geometry = None;
            return Err(MediaError(
                "Native video backing exceeds validated 4096-pixel limit".into(),
            ));
        }
        if let Some(clock) = self.player.inner.presentation_clock.borrow().as_ref() {
            unsafe {
                clock.update_display()?;
            }
        }
        self.stats.backing_width = width as u32;
        self.stats.backing_height = height as u32;
        self.stats.estimated_color_bytes = width as u64 * height as u64 * 8;
        self.stats.geometry_changes += 1;
        self.geometry = Some(geometry);
        self.suppressed = false;
        Ok(())
    }
    /// Service from a queued UI event or BeforeRendering, never an mpv/CV
    /// callback. No Slint redraw, CPU readback, timer, or report_swap is issued.
    pub fn render(&mut self, allow_publication: bool) -> Result<bool> {
        main_thread()?;
        self.stats.render_calls += 1;
        let result = self.render_inner(allow_publication);
        if result.is_err() {
            let _ = self.hide();
        }
        result
    }
    fn render_inner(&mut self, allow_publication: bool) -> Result<bool> {
        let load = self.player.inner.active_load_request.get();
        let changed = load != self.observed_load;
        if changed {
            self.hide_surface()?;
            self.observed_load = load;
        }
        let admitted = allow_publication
            && self.player.current_load_frame_ready()
            && !self.suppressed
            && self.geometry.is_some_and(|g| g.visible);
        if !admitted && self.stats.visible {
            self.hide_surface()?;
        }
        let wake = &self.player.inner.wake;
        if wake.frame.load(Ordering::Acquire)
            && wake.clock.enabled.load(Ordering::Acquire)
            && !wake.clock.ready.load(Ordering::Acquire)
        {
            return Ok(self.stats.visible);
        }
        wake.clock.ready.store(false, Ordering::Release);
        let update = wake.frame.swap(false, Ordering::AcqRel) || changed;
        let due = wake.due.swap(false, Ordering::AcqRel);
        if !update && !due && !(admitted && self.fresh_required) {
            return Ok(self.stats.visible);
        }
        let _current = CurrentContext::set(self.cgl)?;
        let mut frame = false;
        if update {
            self.stats.updates += 1;
            frame = unsafe { ffi::mpv_render_context_update(self.context.as_ptr()) } & 1 != 0;
        }
        if !frame && !(admitted && self.fresh_required) {
            return Ok(self.stats.visible);
        }
        let mut info = ffi::FrameInfo::default();
        unsafe {
            checked(
                ffi::mpv_render_context_get_info(
                    self.context.as_ptr(),
                    ffi::RenderParam::new(11, &mut info),
                ),
                "Read native frame deadline",
            )?;
        }
        if info.target_time > 0 && info.flags & (2 | 4) == 0 {
            // mpv0.41 vo_libmpv.c assigns nanosecond frame->pts (render.h's
            // microsecond comment is stale). Same clock as mpv_get_time_ns.
            let delta = target_delta_us(info.target_time, unsafe {
                ffi::mpv_get_time_ns(self.player.inner.raw)
            })?;
            let us = delta.unsigned_abs();
            self.stats.target_samples += 1;
            if delta < 0 {
                self.stats.early_us += us;
                self.stats.max_early_us = self.stats.max_early_us.max(us);
            } else {
                self.stats.late_us += us;
                self.stats.max_late_us = self.stats.max_late_us.max(us);
            }
        }
        let mut skip = i32::from(!admitted);
        let mut block = 0i32;
        let mut flip = 1i32;
        let mut fbo = ffi::Fbo {
            fbo: 0,
            width: self.stats.backing_width as i32,
            height: self.stats.backing_height as i32,
            internal_format: 0,
        };
        let mut params = [
            ffi::RenderParam::new(3, &mut fbo),
            ffi::RenderParam::new(4, &mut flip),
            ffi::RenderParam::new(12, &mut block),
            ffi::RenderParam::new(13, &mut skip),
            ffi::RenderParam::end(),
        ];
        let mut skip_params = [
            ffi::RenderParam::new(12, &mut block),
            ffi::RenderParam::new(13, &mut skip),
            ffi::RenderParam::end(),
        ];
        let params = if admitted {
            params.as_mut_ptr()
        } else {
            skip_params.as_mut_ptr()
        };
        let start = Instant::now();
        unsafe {
            checked(
                ffi::mpv_render_context_render(self.context.as_ptr(), params),
                "Render native video frame",
            )?;
        }
        let elapsed = start.elapsed().as_micros() as u64;
        self.stats.render_us += elapsed;
        self.stats.max_render_us = self.stats.max_render_us.max(elapsed);
        if !admitted {
            self.stats.skipped_frames += 1;
            return Ok(false);
        }
        self.stats.renders += 1;
        let start = Instant::now();
        native_ok(
            unsafe { oxplay_child_flush(self.surface.0.as_ptr()) },
            "Flush native video surface",
        )?;
        let elapsed = start.elapsed().as_micros() as u64;
        self.stats.flush_us += elapsed;
        self.stats.max_flush_us = self.stats.max_flush_us.max(elapsed);
        if !self.stats.visible {
            native_ok(
                unsafe { oxplay_child_hidden(self.surface.0.as_ptr(), 0) },
                "Reveal native video surface",
            )?;
        }
        self.stats.publications += 1;
        self.stats.visible = true;
        self.fresh_required = false;
        Ok(true)
    }
}

fn target_delta_us(target: i64, now: i64) -> Result<i64> {
    let delta = now
        .checked_sub(target)
        .ok_or_else(|| MediaError("Invalid native media deadline clock".into()))?;
    // Source-reviewed mpv0.41 nanoseconds; refuse gross units/clock errors.
    // Max configured lead100ms plus250ms tolerance, same as existing adapter.
    if !(-350_000_000..=5_000_000_000).contains(&delta) {
        return Err(MediaError(
            "Native media deadline outside validated nanosecond clock range".into(),
        ));
    }
    Ok(delta / 1000)
}
impl Drop for NativeChildPresenter {
    fn drop(&mut self) {
        let _ = self.hide();
        let _current = CurrentContext::set(self.cgl).unwrap_or_else(|_| std::process::abort());
        self.player.inner.presentation_clock.borrow_mut().take();
        unsafe {
            ffi::mpv_render_context_set_update_callback(
                self.context.as_ptr(),
                None,
                std::ptr::null_mut(),
            );
            ffi::mpv_render_context_free(self.context.as_ptr());
        }
        self.player.inner.renderer_attached.set(false);
        self.player.inner.wake.frame.store(false, Ordering::Release);
        self.player.inner.wake.due.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_rejects_invalid_sizes_coordinates_and_disjoint_clips() {
        let rect = NativeRect {
            x: 20.,
            y: -10.,
            width: 640.,
            height: 360.,
        };
        assert!(rect.valid());
        assert!(rect.intersects(NativeRect {
            x: 0.,
            y: 0.,
            width: 800.,
            height: 600.
        }));
        for value in [f64::NAN, f64::INFINITY, -1., 0., 4097.] {
            assert!(
                !NativeRect {
                    width: value,
                    ..rect
                }
                .valid()
            );
        }
        assert!(!rect.intersects(NativeRect { x: 660., ..rect }));
        assert!(!NativeRect { x: 65537., ..rect }.valid());
    }
    #[test]
    fn deadline_diagnostic_rejects_overflow_units_and_implausible_future_times() {
        assert_eq!(
            target_delta_us(2_000_000_000, 1_990_000_000).unwrap(),
            -10_000
        );
        assert_eq!(
            target_delta_us(2_000_000_000, 2_003_000_000).unwrap(),
            3_000
        );
        assert!(target_delta_us(i64::MAX, i64::MIN).is_err());
        assert!(target_delta_us(2_000_000_000, 1_000_000_000).is_err());
        assert!(target_delta_us(2_000_000, 2_000_000_000_000).is_err());
    }
}
