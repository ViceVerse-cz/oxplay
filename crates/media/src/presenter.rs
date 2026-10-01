// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{MediaError, Player, Result, checked, ffi, render_wake};
use glow::HasContext;
use std::{
    ffi::{CStr, c_char, c_void},
    num::NonZeroU32,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderStats {
    pub updates: u64,
    pub video_draws: u64,
    pub ui_draws: u64,
    pub target_allocations: u64,
    pub target_bytes: u64,
    /// Explicit experiment; false unless OXPLAY_STABLE_VIDEO_TARGET=1.
    pub stable_video_target: bool,
    pub stable_target_draws: u64,
    pub private_target_draws: u64,
    /// Successful renders selected for publication from next; not displayed
    /// pixels, GPU completion, or proof that the compositor presented them.
    pub target_publications: u64,
    /// Enabled only with OXPLAY_MEDIA_TIMING=1. Totals are wall-clock µs.
    pub timing_samples: u64,
    pub gl_save_us: u64,
    pub mpv_update_us: u64,
    pub mpv_render_us: u64,
    pub gl_restore_us: u64,
    pub max_mpv_render_us: u64,
    pub target_time_samples: u64,
    pub early_us: u64,
    pub late_us: u64,
    pub max_early_us: u64,
    pub max_late_us: u64,
    pub scheduled_wakes: u64,
    /// Newest callback to BeforeRendering; lower bound when callbacks coalesce.
    pub callback_latency_samples: u64,
    pub callback_latency_us: u64,
    pub max_callback_latency_us: u64,
    pub gpu: crate::GpuTimingStats,
    /// Ambient-mode colour sampling; all zero unless the host enables it.
    pub ambient: crate::AmbientStats,
}
// The host may borrow displayed until the next published image replaces it.
// Private frames and private-target resize must only touch next. The opt-in
// stable path may update already admitted pixels in that same GL command stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameTarget {
    Private,
    PublishNext,
    ReuseDisplayed,
}
impl FrameTarget {
    pub(crate) fn choose(
        stable: bool,
        ready: bool,
        load: u64,
        published: u64,
        same_size: bool,
    ) -> Self {
        if !ready || load == 0 {
            Self::Private
        } else if stable && load == published && same_size {
            Self::ReuseDisplayed
        } else {
            Self::PublishNext
        }
    }
}
pub(crate) struct PublicationPair<T> {
    pub(crate) displayed: Option<T>,
    pub(crate) next: Option<T>,
}
impl<T> Default for PublicationPair<T> {
    fn default() -> Self {
        Self {
            displayed: None,
            next: None,
        }
    }
}
impl<T> PublicationPair<T> {
    pub(crate) fn render_target(&self, target: FrameTarget) -> Option<&T> {
        match target {
            FrameTarget::ReuseDisplayed => self.displayed.as_ref(),
            FrameTarget::Private | FrameTarget::PublishNext => self.next.as_ref(),
        }
    }
    pub(crate) fn finish_frame(&mut self, target: FrameTarget) -> Option<&T> {
        match target {
            FrameTarget::Private => None,
            FrameTarget::PublishNext => {
                std::mem::swap(&mut self.displayed, &mut self.next);
                self.displayed.as_ref()
            }
            FrameTarget::ReuseDisplayed => self.displayed.as_ref(),
        }
    }
}
pub(crate) struct Target {
    texture: glow::NativeTexture,
    pub(crate) fbo: glow::NativeFramebuffer,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
impl Target {
    pub(crate) unsafe fn create(gl: &glow::Context, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let texture = gl.create_texture().map_err(MediaError)?;
            let fbo = match gl.create_framebuffer() {
                Ok(fbo) => fbo,
                Err(e) => {
                    gl.delete_texture(texture);
                    return Err(MediaError(e));
                }
            };
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(texture);
                return Err(MediaError("Incomplete video framebuffer".into()));
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            Ok(Self {
                texture,
                fbo,
                width,
                height,
            })
        }
    }
    pub(crate) unsafe fn delete(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.texture);
        }
    }
    fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
}
/// Persistent, same-context double-buffered RGBA8 target. No CPU frame storage.
/// !Send/!Sync by construction; one presenter per engine and per Slint window.
pub struct GlPresenter {
    player: Player,
    gl: Rc<glow::Context>,
    context: *mut ffi::RenderContext,
    targets: PublicationPair<Target>,
    retired: Vec<Target>,
    stats: RenderStats,
    graphics_info: String,
    measure_timing: bool,
    timing_lead_ms: u32,
    prepare_ms: u32,
    pending_frame: Option<ffi::FrameInfo>,
    observed_load: u64,
    published_load: u64,
    deadline_timer: slint::Timer,
    scheduled_deadline: Option<i64>,
    gpu_timing: Option<crate::gpu_timing::GpuTiming>,
    ambient: crate::ambient::AmbientSampler,
    ambient_enabled: bool,
}
impl GlPresenter {
    /// # Safety
    /// Only call inside the owning Slint/FemtoVG 0.27 window's NativeOpenGL
    /// RenderingSetup, or BeforeRendering when recovering a failed setup and no
    /// presenter is attached. Creation saves/restores the borrowed GL state.
    /// No other custom GL renderers may share this context.
    /// The same desktop GL >=3.3 context must be current for every render,
    /// after_render and drop. Images must never cross window boundaries.
    pub unsafe fn new(
        player: &Player,
        get_proc_address: &dyn Fn(&CStr) -> *const c_void,
    ) -> Result<Self> {
        if player.inner.renderer_attached.get() {
            return Err(MediaError("A presenter is already attached".into()));
        }
        let gl =
            Rc::new(unsafe { glow::Context::from_loader_function_cstr(|s| get_proc_address(s)) });
        let graphics_info = unsafe {
            format!(
                "{} | {} | {}",
                gl.get_parameter_string(glow::VENDOR),
                gl.get_parameter_string(glow::RENDERER),
                gl.get_parameter_string(glow::VERSION)
            )
        };
        let version = gl.version();
        if version.is_embedded || (version.major, version.minor) < (3, 3) {
            return Err(MediaError(
                "Video adapter requires desktop OpenGL 3.3 or later; this context is unsupported"
                    .into(),
            ));
        }
        let gpu_mode = std::env::var_os("OXPLAY_GPU_TIMING");
        let gpu_timing = if gpu_mode
            .as_ref()
            .is_some_and(|v| v == "1" || v == "ui-elapsed")
        {
            let timer = unsafe {
                crate::gpu_timing::GpuTiming::new(
                    gl.clone(),
                    get_proc_address,
                    gpu_mode.is_some_and(|v| v == "ui-elapsed"),
                )
            };
            eprintln!(
                "GPU timeline diagnostic: {:?}; excludes pre-notifier clear and presentation",
                timer.stats()
            );
            Some(timer)
        } else {
            None
        };
        // libmpv resolves symbols synchronously during context creation; this
        // stack reference is not retained by libmpv (render_gl.h contract).
        struct Loader<'a>(&'a dyn Fn(&CStr) -> *const c_void);
        unsafe extern "C" fn load(data: *mut c_void, name: *const c_char) -> *mut c_void {
            let loader = unsafe { &*data.cast::<Loader<'_>>() };
            (loader.0)(unsafe { CStr::from_ptr(name) }).cast_mut()
        }
        let mut loader = Loader(get_proc_address);
        let mut init = ffi::GlInit {
            get_proc_address: load,
            context: std::ptr::from_mut(&mut loader).cast(),
        };
        let mut params = [
            ffi::RenderParam {
                kind: 1,
                data: c"opengl".as_ptr().cast_mut().cast(),
            },
            ffi::RenderParam::new(2, &mut init),
            ffi::RenderParam::end(),
        ];
        let mut context = std::ptr::null_mut();
        let _state = unsafe { GlState::save_and_reset(gl.clone()) };
        unsafe {
            checked(
                ffi::mpv_render_context_create(&mut context, player.inner.raw, params.as_mut_ptr()),
                "Create OpenGL media renderer",
            )?;
        }
        unsafe {
            ffi::mpv_render_context_set_update_callback(
                context,
                Some(render_wake),
                Arc::as_ptr(&player.inner.wake).cast_mut().cast(),
            );
        }
        #[cfg(target_os = "macos")]
        {
            let clock =
                match unsafe { crate::macos::PresentationClock::new(player.inner.wake.clone()) } {
                    Ok(clock) => clock,
                    Err(error) => {
                        unsafe {
                            ffi::mpv_render_context_set_update_callback(
                                context,
                                None,
                                std::ptr::null_mut(),
                            );
                            ffi::mpv_render_context_free(context);
                        }
                        return Err(error);
                    }
                };
            *player.inner.presentation_clock.borrow_mut() = Some(clock);
        }
        player.inner.renderer_attached.set(true);
        let measure_timing = std::env::var_os("OXPLAY_MEDIA_TIMING").is_some_and(|v| v == "1");
        Ok(Self {
            player: player.clone(),
            gl: gl.clone(),
            context,
            targets: PublicationPair::default(),
            retired: Vec::with_capacity(2),
            stats: RenderStats {
                stable_video_target: std::env::var_os("OXPLAY_STABLE_VIDEO_TARGET")
                    .is_some_and(|value| value == "1"),
                ..RenderStats::default()
            },
            graphics_info,
            measure_timing,
            timing_lead_ms: player.inner.snapshot.borrow().timing_lead_ms,
            prepare_ms: player.inner.snapshot.borrow().prepare_ms,
            pending_frame: None,
            observed_load: 0,
            published_load: 0,
            deadline_timer: slint::Timer::default(),
            scheduled_deadline: None,
            gpu_timing,
            ambient: crate::ambient::AmbientSampler::new(measure_timing),
            ambient_enabled: false,
        })
    }
    pub fn graphics_info(&self) -> &str {
        &self.graphics_info
    }
    pub fn stats(&self) -> RenderStats {
        let mut stats = self.stats;
        stats.gpu = self
            .gpu_timing
            .as_ref()
            .map(|g| g.stats())
            .unwrap_or_default();
        stats.ambient = self.ambient.stats();
        stats
    }
    /// Ambient-mode colour sampling. Off by default; the host enables it only
    /// while the glow is actually shown. Costs nothing while disabled.
    pub fn set_ambient_sampling(&mut self, enabled: bool) {
        if enabled && !self.ambient_enabled {
            self.ambient.rearm();
        }
        self.ambient_enabled = enabled;
    }
    /// Newest colour summary, at most one per sample (≤4 Hz while playing).
    pub fn take_ambient_sample(&mut self) -> Option<crate::AmbientSample> {
        self.ambient.take()
    }
    /// Native load identity observed by the latest render call; lets the host
    /// retire colours of a replaced video before any new sample exists.
    pub fn observed_load_request(&self) -> u64 {
        self.observed_load
    }
    /// # Safety
    /// Call only in BeforeRendering on the original window/context. Assign any
    /// returned image immediately and unconditionally to the window's video
    /// property. Set allow_publication=false if the host cannot do that. Sizes are
    /// physical pixels; zero-sized/hidden windows should not be drawn.
    pub unsafe fn render(
        &mut self,
        width: u32,
        height: u32,
        allow_publication: bool,
    ) -> Result<Option<slint::Image>> {
        self.stats.ui_draws += 1;
        if let Some(gpu) = &mut self.gpu_timing {
            unsafe {
                gpu.begin_frame();
            }
        }
        if self.ambient.has_pending() {
            // Non-blocking fence check; reads 576 bytes once the GPU is done.
            unsafe {
                if self.ambient_enabled {
                    self.ambient.collect(&self.gl);
                } else {
                    self.ambient.discard(&self.gl);
                }
            }
        }
        let result = unsafe { self.render_video(width, height, allow_publication) };
        if let Some(gpu) = &mut self.gpu_timing {
            // All early returns, errors, and UI-only frames use the same boundary.
            // No mpv GL work can occur between this and AfterRendering.
            unsafe { gpu.begin_ui() };
        }
        result
    }
    unsafe fn render_video(
        &mut self,
        width: u32,
        height: u32,
        allow_publication: bool,
    ) -> Result<Option<slint::Image>> {
        let load = self.player.inner.active_load_request.get();
        let changed_load = load != self.observed_load;
        if changed_load {
            self.observed_load = load;
            self.published_load = 0;
            self.pending_frame = None;
            self.deadline_timer.stop();
            self.scheduled_deadline = None;
        }
        let frame_ready = allow_publication && self.player.current_load_frame_ready();
        let first_publication = frame_ready && self.published_load != load;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        // Bound a pair to 128 MiB; no hidden quality reduction to meet budgets.
        if width > 4096 || height > 4096 {
            return Err(MediaError(
                "Video target exceeds validated 4096-pixel limit".into(),
            ));
        }
        let resized = self
            .targets
            .displayed
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height);
        #[cfg(target_os = "macos")]
        if self.player.inner.wake.clock.enabled.load(Ordering::Acquire) {
            if self.player.inner.wake.frame.load(Ordering::Acquire)
                && self.player.inner.wake.clock.waiting_for_tick()
            {
                return Ok(None);
            }
            self.player
                .inner
                .wake
                .clock
                .ready
                .store(false, Ordering::Release);
        }
        let update = self.player.inner.wake.frame.swap(false, Ordering::AcqRel) || changed_load;
        let due = self.player.inner.wake.due.swap(false, Ordering::AcqRel);
        if update && let Some(origin) = self.player.inner.wake.timing_origin {
            let callback = self
                .player
                .inner
                .wake
                .callback_time_ns
                .load(Ordering::Acquire);
            if callback != 0 {
                let now = (origin.elapsed().as_nanos() as u64).saturating_add(1);
                let latency = now.saturating_sub(callback) / 1000;
                self.stats.callback_latency_samples += 1;
                self.stats.callback_latency_us += latency;
                self.stats.max_callback_latency_us =
                    self.stats.max_callback_latency_us.max(latency);
            }
        }
        if !update && !due && !resized && !first_publication {
            return Ok(None);
        }
        #[cfg(target_os = "macos")]
        if resized && let Some(clock) = self.player.inner.presentation_clock.borrow().as_ref() {
            unsafe {
                clock.update_display()?;
            }
        }
        let stage = self.measure_timing.then(std::time::Instant::now);
        let state = unsafe { GlState::save_and_reset(self.gl.clone()) };
        if let Some(start) = stage {
            self.stats.gl_save_us += start.elapsed().as_micros() as u64;
        }
        let stage = self.measure_timing.then(std::time::Instant::now);
        if update {
            self.stats.updates += 1;
            let flags = unsafe { ffi::mpv_render_context_update(self.context) };
            if flags & 1 != 0 {
                let mut info = ffi::FrameInfo::default();
                unsafe {
                    checked(
                        ffi::mpv_render_context_get_info(
                            self.context,
                            ffi::RenderParam::new(11, &mut info),
                        ),
                        "Read frame deadline",
                    )?;
                }
                self.pending_frame = (info.flags & 1 != 0).then_some(info);
            } else {
                // mpv 0.41 update() tests ctx->next_frame, not the callback
                // reason: zero is authoritative cancellation/no pending frame.
                // A new frame arriving afterward raises a fresh atomic wake.
                self.pending_frame = None;
            }
        }
        if let Some(start) = stage {
            self.stats.mpv_update_us += start.elapsed().as_micros() as u64;
        }
        #[cfg(target_os = "macos")]
        let display_clock_controls_timing =
            self.player.inner.wake.clock.enabled.load(Ordering::Acquire);
        #[cfg(not(target_os = "macos"))]
        let display_clock_controls_timing = false;
        if let Some(info) = self.pending_frame {
            // mpv 0.41 vo_libmpv.c assigns frame->pts in nanoseconds; its
            // render.h microseconds comment is stale. API 2.5 is required.
            let now = unsafe { ffi::mpv_get_time_ns(self.player.inner.raw) };
            // The native display clock is the sole pacing mechanism on macOS;
            // validate the source-reviewed clock domain without a second timer.
            if display_clock_controls_timing && info.flags & (2 | 4) == 0 && info.target_time != 0 {
                let _ = deadline_delay(info.target_time, now, self.timing_lead_ms, 0)?;
            }
            // REDRAW/REPEAT frames and target=0 are immediate, e.g. paused seek.
            if self.timing_lead_ms > 0
                && !display_clock_controls_timing
                && info.flags & (2 | 4) == 0
                && info.target_time != 0
                && let Some(delay) =
                    deadline_delay(info.target_time, now, self.timing_lead_ms, self.prepare_ms)?
            {
                if self.scheduled_deadline != Some(info.target_time)
                    || !self.deadline_timer.running()
                {
                    let wake = self.player.inner.wake.clone();
                    self.deadline_timer
                        .start(slint::TimerMode::SingleShot, delay, move || {
                            // UI-thread one-shot wake only. GL remains confined
                            // to the next BeforeRendering callback.
                            wake.due.store(true, Ordering::Release);
                            wake.notify();
                        });
                    self.scheduled_deadline = Some(info.target_time);
                    self.stats.scheduled_wakes += 1;
                }
                // Keep the previous texture, even on resize, until due.
                return Ok(None);
            }
            if self.measure_timing && info.target_time != 0 {
                let offset_us = info.target_time.saturating_sub(now) / 1000;
                self.stats.target_time_samples += 1;
                if offset_us > 0 {
                    self.stats.early_us += offset_us as u64;
                    self.stats.max_early_us = self.stats.max_early_us.max(offset_us as u64);
                } else {
                    let late = offset_us.unsigned_abs();
                    self.stats.late_us += late;
                    self.stats.max_late_us = self.stats.max_late_us.max(late);
                }
            }
        }
        self.deadline_timer.stop();
        self.scheduled_deadline = None;
        if self.pending_frame.is_none()
            && !first_publication
            && (!resized || self.targets.displayed.is_none())
        {
            return Ok(None);
        }
        if self
            .targets
            .next
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let target = unsafe { Target::create(&self.gl, width, height)? };
            if let Some(old) = self.targets.next.replace(target) {
                self.retired.push(old);
            }
            self.stats.target_allocations += 1;
        }
        let frame_target = FrameTarget::choose(
            self.stats.stable_video_target,
            frame_ready,
            load,
            self.published_load,
            !resized,
        );
        // Same context command ordering is previous Slint sample -> this mpv
        // write -> next Slint sample. No private/startup/replacement work may
        // take this route. The video Image and its ancestors MUST NOT cache a
        // rendered layer or colorize the source; see docs/stable-video-target.md.
        let target = self
            .targets
            .render_target(frame_target)
            .expect("target allocated above");
        let source_fbo = target.fbo;
        let mut fbo = ffi::Fbo {
            fbo: target.fbo.0.get() as i32,
            width: width as i32,
            height: height as i32,
            internal_format: glow::RGBA8 as i32,
        };
        let mut flip = 0;
        let mut block = 0;
        let mut params = [
            ffi::RenderParam::new(3, &mut fbo),
            ffi::RenderParam::new(4, &mut flip),
            ffi::RenderParam::new(12, &mut block),
            ffi::RenderParam::end(),
        ];
        let stage = self.measure_timing.then(std::time::Instant::now);
        if let Some(gpu) = &mut self.gpu_timing {
            unsafe {
                gpu.begin_media();
            }
        }
        let rendered = unsafe { ffi::mpv_render_context_render(self.context, params.as_mut_ptr()) };
        if let Some(gpu) = &mut self.gpu_timing {
            unsafe {
                gpu.end_media();
            }
        }
        checked(rendered, "Render video")?;
        if let Some(start) = stage {
            let elapsed = start.elapsed().as_micros() as u64;
            self.stats.mpv_render_us += elapsed;
            self.stats.max_mpv_render_us = self.stats.max_mpv_render_us.max(elapsed);
            self.stats.timing_samples += 1;
        }
        // Private startup frames are never shown, so they never tint the glow.
        if self.ambient_enabled && frame_target != FrameTarget::Private {
            let now = std::time::Instant::now();
            if self.ambient.due(now, load) {
                let rect = {
                    let snapshot = self.player.inner.snapshot.borrow();
                    crate::ambient::content_rect(width, height, snapshot.width, snapshot.height)
                };
                // Still inside the saved/reset state. A failure only disables
                // ambient sampling; playback and publication are unaffected.
                if let Err(error) =
                    unsafe { self.ambient.sample(&self.gl, source_fbo, rect, load, now) }
                {
                    eprintln!("ambient sampling disabled: {error}");
                }
            }
        }
        let stage = self.measure_timing.then(std::time::Instant::now);
        drop(state);
        if let Some(start) = stage {
            self.stats.gl_restore_us += start.elapsed().as_micros() as u64;
        }
        self.stats.video_draws += 1;
        match frame_target {
            FrameTarget::Private => {
                self.stats.private_target_draws = self.stats.private_target_draws.saturating_add(1)
            }
            FrameTarget::PublishNext => {
                self.stats.target_publications = self.stats.target_publications.saturating_add(1)
            }
            FrameTarget::ReuseDisplayed => {
                self.stats.stable_target_draws = self.stats.stable_target_draws.saturating_add(1)
            }
        }
        self.pending_frame = None;
        self.stats.target_bytes = self
            .targets
            .displayed
            .iter()
            .chain(self.targets.next.iter())
            .chain(self.retired.iter())
            .map(Target::bytes)
            .sum();
        // Slint may still borrow displayed. Private startup work must remain in
        // next until an image is actually handed to the host for replacement.
        // Swapping here without publication would make the next render overwrite
        // a still-borrowed target, and resizing could retire that target early.
        let Some(target) = self.targets.finish_frame(frame_target) else {
            return Ok(None);
        };
        // mpv FBO output is top-left with FLIP_Y=0; Slint defaults to top-left.
        // Color/orientation still require fixture validation on each platform.
        let image = unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                target.texture.0,
                (width, height).into(),
            )
            .build()
        };
        // Consume startup frames privately: the VO must finish its first frame
        // before PLAYBACK_RESTART, including paused loads. Publication requires
        // the separately correlated native first-frame fence, never merely a
        // resize, cached FrameInfo, FILE_LOADED, or matching START_FILE ID.
        self.published_load = load;
        Ok(Some(image))
    }
    /// # Safety
    /// Call only in AfterRendering of the original window. Slint's draw commands
    /// and cache drain have completed; old borrowed images were replaced above.
    pub unsafe fn after_render(&mut self) {
        if let Some(gpu) = &mut self.gpu_timing {
            unsafe {
                gpu.end_frame();
            }
        }
        for target in self.retired.drain(..) {
            unsafe {
                target.delete(&self.gl);
            }
        }
        self.stats.target_bytes = self
            .targets
            .displayed
            .iter()
            .chain(self.targets.next.iter())
            .map(Target::bytes)
            .sum();
        // Do not report_swap: Slint's AfterRendering is *before* actual swap.
    }
}
impl Drop for GlPresenter {
    fn drop(&mut self) {
        // End an interrupted UI diagnostic before libmpv teardown can run its
        // own non-nestable elapsed queries. The owning GL context is current.
        self.gpu_timing.take();
        self.deadline_timer.stop();
        // Ambient objects are never left bound; the owning context is current.
        unsafe { self.ambient.delete(&self.gl) };
        #[cfg(target_os = "macos")]
        self.player.inner.presentation_clock.borrow_mut().take();
        self.player.inner.wake.due.store(false, Ordering::Release);
        // Host clears its Image property before dropping in RenderingTeardown.
        // GL deletion defers GPU storage reclamation for in-flight commands.
        unsafe {
            {
                let _state = GlState::save_and_reset(self.gl.clone());
                ffi::mpv_render_context_set_update_callback(
                    self.context,
                    None,
                    std::ptr::null_mut(),
                );
                ffi::mpv_render_context_free(self.context);
            }
            // Restore saved bindings before deleting targets: restoring a deleted
            // texture name afterward would violate the core-profile GL contract.
            for target in self
                .targets
                .displayed
                .take()
                .into_iter()
                .chain(self.targets.next.take())
                .chain(self.retired.drain(..))
            {
                target.delete(&self.gl);
            }
        }
        self.player.inner.renderer_attached.set(false);
        self.player.inner.wake.frame.store(false, Ordering::Release);
        // Without a presenter no video is visible; release at teardown.
        #[cfg(windows)]
        self.player.inner.display_request.update(false);
    }
}

// libmpv requires standard GL defaults on entry and restores defaults on exit,
// except viewport/scissor/blend/clear/dither. Slint requires its incoming state
// preserved. Snapshot both bindings and raster state; no pixel readback occurs.
struct GlState {
    gl: Rc<glow::Context>,
    integers: Vec<(u32, i32)>,
    enabled: Vec<(u32, bool)>,
    viewport: [i32; 4],
    scissor: [i32; 4],
    clear: [f32; 4],
    color_mask: [i32; 4],
    textures: Vec<[i32; 7]>,
}
const INTEGER_STATES: &[u32] = &[
    glow::CURRENT_PROGRAM,
    glow::VERTEX_ARRAY_BINDING,
    glow::ARRAY_BUFFER_BINDING,
    glow::DRAW_FRAMEBUFFER_BINDING,
    glow::READ_FRAMEBUFFER_BINDING,
    glow::RENDERBUFFER_BINDING,
    glow::ACTIVE_TEXTURE,
    glow::BLEND_SRC_RGB,
    glow::BLEND_DST_RGB,
    glow::BLEND_SRC_ALPHA,
    glow::BLEND_DST_ALPHA,
    glow::BLEND_EQUATION_RGB,
    glow::BLEND_EQUATION_ALPHA,
    glow::UNPACK_ALIGNMENT,
    glow::UNPACK_ROW_LENGTH,
    glow::UNPACK_SKIP_PIXELS,
    glow::UNPACK_SKIP_ROWS,
    glow::PIXEL_UNPACK_BUFFER_BINDING,
    glow::PIXEL_PACK_BUFFER_BINDING,
];
const ENABLES: &[u32] = &[
    glow::BLEND,
    glow::SCISSOR_TEST,
    glow::DEPTH_TEST,
    glow::STENCIL_TEST,
    glow::CULL_FACE,
    glow::DITHER,
    glow::FRAMEBUFFER_SRGB,
];
const TEXTURES: &[(u32, u32)] = &[
    (glow::TEXTURE_2D, glow::TEXTURE_BINDING_2D),
    (glow::TEXTURE_3D, glow::TEXTURE_BINDING_3D),
    (glow::TEXTURE_CUBE_MAP, glow::TEXTURE_BINDING_CUBE_MAP),
    (glow::TEXTURE_RECTANGLE, glow::TEXTURE_BINDING_RECTANGLE),
    (glow::TEXTURE_2D_ARRAY, glow::TEXTURE_BINDING_2D_ARRAY),
    (glow::TEXTURE_1D, glow::TEXTURE_BINDING_1D),
];
impl GlState {
    unsafe fn save_and_reset(gl: Rc<glow::Context>) -> Self {
        unsafe {
            let integers = INTEGER_STATES
                .iter()
                .map(|&key| (key, gl.get_parameter_i32(key)))
                .collect();
            let enabled = ENABLES
                .iter()
                .map(|&key| (key, gl.is_enabled(key)))
                .collect();
            let mut viewport = [0; 4];
            let mut scissor = [0; 4];
            let mut clear = [0.; 4];
            let mut color_mask = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor);
            gl.get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut clear);
            gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut color_mask);
            // Reviewed FemtoVG 0.27 render()/set_uniforms() uses only texture
            // units 0 and 1. Our adapter also uses only unit 0; mpv 0.41
            // ra_gl.c:disable_binding unbinds every unit it uses. Thus units
            // >=2 remain at GL defaults across notifier calls. Do not query all
            // MAX_COMBINED_TEXTURE_IMAGE_UNITS on every video frame (80 on M1).
            // Re-audit this invariant when either renderer revision changes.
            let mut textures = Vec::with_capacity(2);
            for unit in 0..2 {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                let mut bindings = [0; 7];
                for (i, &(target, binding)) in TEXTURES.iter().enumerate() {
                    bindings[i] = gl.get_parameter_i32(binding);
                    gl.bind_texture(target, None);
                }
                bindings[6] = gl.get_parameter_i32(glow::SAMPLER_BINDING);
                gl.bind_sampler(unit as u32, None);
                textures.push(bindings);
            }
            gl.active_texture(glow::TEXTURE0);
            gl.use_program(None);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            for &enable in ENABLES {
                gl.disable(enable);
            }
            gl.color_mask(true, true, true, true);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
            gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
            Self {
                gl,
                integers,
                enabled,
                viewport,
                scissor,
                clear,
                color_mask,
                textures,
            }
        }
    }
    fn get(&self, key: u32) -> i32 {
        self.integers
            .iter()
            .find(|(k, _)| *k == key)
            .expect("saved GL state")
            .1
    }
}
impl Drop for GlState {
    fn drop(&mut self) {
        unsafe {
            let gl = &self.gl;
            for (unit, bindings) in self.textures.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                for (i, &(target, _)) in TEXTURES.iter().enumerate() {
                    gl.bind_texture(
                        target,
                        NonZeroU32::new(bindings[i] as u32).map(glow::NativeTexture),
                    );
                }
                gl.bind_sampler(
                    unit as u32,
                    NonZeroU32::new(bindings[6] as u32).map(glow::NativeSampler),
                );
            }
            gl.active_texture(self.get(glow::ACTIVE_TEXTURE) as u32);
            gl.use_program(
                NonZeroU32::new(self.get(glow::CURRENT_PROGRAM) as u32).map(glow::NativeProgram),
            );
            gl.bind_vertex_array(
                NonZeroU32::new(self.get(glow::VERTEX_ARRAY_BINDING) as u32)
                    .map(glow::NativeVertexArray),
            );
            gl.bind_buffer(
                glow::ARRAY_BUFFER,
                NonZeroU32::new(self.get(glow::ARRAY_BUFFER_BINDING) as u32)
                    .map(glow::NativeBuffer),
            );
            gl.bind_buffer(
                glow::PIXEL_UNPACK_BUFFER,
                NonZeroU32::new(self.get(glow::PIXEL_UNPACK_BUFFER_BINDING) as u32)
                    .map(glow::NativeBuffer),
            );
            gl.bind_buffer(
                glow::PIXEL_PACK_BUFFER,
                NonZeroU32::new(self.get(glow::PIXEL_PACK_BUFFER_BINDING) as u32)
                    .map(glow::NativeBuffer),
            );
            gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                NonZeroU32::new(self.get(glow::DRAW_FRAMEBUFFER_BINDING) as u32)
                    .map(glow::NativeFramebuffer),
            );
            gl.bind_framebuffer(
                glow::READ_FRAMEBUFFER,
                NonZeroU32::new(self.get(glow::READ_FRAMEBUFFER_BINDING) as u32)
                    .map(glow::NativeFramebuffer),
            );
            gl.bind_renderbuffer(
                glow::RENDERBUFFER,
                NonZeroU32::new(self.get(glow::RENDERBUFFER_BINDING) as u32)
                    .map(glow::NativeRenderbuffer),
            );
            gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            gl.scissor(
                self.scissor[0],
                self.scissor[1],
                self.scissor[2],
                self.scissor[3],
            );
            gl.clear_color(self.clear[0], self.clear[1], self.clear[2], self.clear[3]);
            gl.color_mask(
                self.color_mask[0] != 0,
                self.color_mask[1] != 0,
                self.color_mask[2] != 0,
                self.color_mask[3] != 0,
            );
            gl.blend_func_separate(
                self.get(glow::BLEND_SRC_RGB) as u32,
                self.get(glow::BLEND_DST_RGB) as u32,
                self.get(glow::BLEND_SRC_ALPHA) as u32,
                self.get(glow::BLEND_DST_ALPHA) as u32,
            );
            gl.blend_equation_separate(
                self.get(glow::BLEND_EQUATION_RGB) as u32,
                self.get(glow::BLEND_EQUATION_ALPHA) as u32,
            );
            for key in [
                glow::UNPACK_ALIGNMENT,
                glow::UNPACK_ROW_LENGTH,
                glow::UNPACK_SKIP_PIXELS,
                glow::UNPACK_SKIP_ROWS,
            ] {
                gl.pixel_store_i32(key, self.get(key));
            }
            for &(enable, value) in &self.enabled {
                if value {
                    gl.enable(enable);
                } else {
                    gl.disable(enable);
                }
            }
        }
    }
}

/// Validate the source-reviewed nanosecond clock domain, then round *up* to
/// Slint's millisecond timer resolution. Never spin with repeated zero-ms timers.
pub(crate) fn deadline_delay(
    target_ns: i64,
    now_ns: i64,
    lead_ms: u32,
    prepare_ms: u32,
) -> Result<Option<std::time::Duration>> {
    let delta = target_ns
        .checked_sub(now_ns)
        .ok_or_else(|| MediaError("Invalid media deadline clock".into()))?;
    if delta.unsigned_abs() > 5_000_000_000 || delta > i64::from(lead_ms) * 1_000_000 + 250_000_000
    {
        return Err(MediaError(
            "Media deadline outside validated mpv 0.41 nanosecond clock range".into(),
        ));
    }
    // The target is presentation time. Leave a bounded diagnostic allowance
    // for GPU preparation/UI composition instead of starting work at target.
    let wait_ns = delta - i64::from(prepare_ms) * 1_000_000;
    if wait_ns <= 0 {
        return Ok(None);
    }
    let millis = (wait_ns as u64).div_ceil(1_000_000);
    Ok(Some(std::time::Duration::from_millis(millis)))
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    #[test]
    fn stable_target_is_only_reused_after_admission_and_never_for_private_work() {
        use std::cell::Cell;
        let displayed = Rc::new(Cell::new(1));
        let mut pair = PublicationPair {
            displayed: Some(displayed.clone()),
            next: Some(Rc::new(Cell::new(0))),
        };
        let stable = FrameTarget::choose(true, true, 7, 7, true);
        assert_eq!(stable, FrameTarget::ReuseDisplayed);
        pair.render_target(stable).unwrap().set(2);
        assert!(Rc::ptr_eq(pair.finish_frame(stable).unwrap(), &displayed));
        assert_eq!(displayed.get(), 2);
        for (ready, load, published, same_size) in [
            (false, 7, 7, true),  // publication forbidden / frame not ready
            (false, 8, 7, true),  // private replacement
            (false, 0, 7, true),  // terminal stop
            (true, 0, 0, true),   // zero identity is private even with bad readiness
            (false, 7, 7, false), // private resize
        ] {
            let plan = FrameTarget::choose(true, ready, load, published, same_size);
            assert_eq!(plan, FrameTarget::Private);
            pair.render_target(plan).unwrap().set(3);
            assert!(pair.finish_frame(plan).is_none());
            assert_eq!(displayed.get(), 2);
        }
        for (enabled, load, published, same_size) in [
            (false, 7, 7, true), // unchanged default alternation
            (true, 8, 7, true),  // first admitted frame of replacement
            (true, 7, 7, false), // admitted resize
            (true, 7, 0, true),  // no previous publication
        ] {
            assert_eq!(
                FrameTarget::choose(enabled, true, load, published, same_size),
                FrameTarget::PublishNext
            );
        }
        // Publish a resized replacement; an old borrowed image is unchanged.
        let retired = pair.next.replace(Rc::new(Cell::new(4))).unwrap();
        assert!(!Rc::ptr_eq(&retired, &displayed));
        let replacement = pair.finish_frame(FrameTarget::PublishNext).unwrap().clone();
        assert_eq!(displayed.get(), 2);
        assert_eq!(replacement.get(), 4);
        pair.render_target(FrameTarget::ReuseDisplayed)
            .unwrap()
            .set(5);
        assert_eq!(replacement.get(), 5);
        assert_eq!(displayed.get(), 2);
    }

    // A small brace-scope scan, not a Slint parser: comments and quoted strings
    // are skipped so sibling properties cannot masquerade as ancestor state.
    // It guards this workspace's single live video Image against introducing a
    // cached/colorized intermediate layer while its source identity is stable.
    fn video_has_no_cached_or_colorized_ancestor(source: &str) -> bool {
        let chars: Vec<_> = source.chars().collect();
        let mut scopes: Vec<(Option<usize>, String)> = vec![(None, String::new())];
        let mut current = 0;
        let mut at = 0;
        while at < chars.len() {
            match (chars[at], chars.get(at + 1)) {
                ('/', Some('/')) => {
                    at += 2;
                    while at < chars.len() && chars[at] != '\n' {
                        at += 1;
                    }
                }
                ('/', Some('*')) => {
                    at += 2;
                    while at + 1 < chars.len() && (chars[at], chars[at + 1]) != ('*', '/') {
                        at += 1;
                    }
                    at += 2;
                }
                ('"', _) => {
                    at += 1;
                    while at < chars.len() && chars[at] != '"' {
                        at += if chars[at] == '\\' { 2 } else { 1 };
                    }
                    at += 1;
                }
                ('{', _) => {
                    scopes.push((Some(current), String::new()));
                    current = scopes.len() - 1;
                    at += 1;
                }
                ('}', _) => {
                    let Some(parent) = scopes[current].0 else {
                        return false;
                    };
                    current = parent;
                    at += 1;
                }
                (character, _) => {
                    if !character.is_whitespace() {
                        scopes[current].1.push(character);
                    }
                    at += 1;
                }
            }
        }
        if current != 0 {
            return false;
        }
        let video_scopes: Vec<_> = scopes
            .iter()
            .enumerate()
            .filter(|(_, (_, body))| body.contains("source:root.video-texture;"))
            .map(|(index, _)| index)
            .collect();
        if video_scopes.len() != 1 {
            return false;
        }
        let mut scope = Some(video_scopes[0]);
        while let Some(index) = scope {
            let (parent, body) = &scopes[index];
            if body.contains("cache-rendering-hint:") || body.contains("colorize:") {
                return false;
            }
            scope = *parent;
        }
        true
    }

    #[test]
    fn live_video_remains_a_single_uncached_uncolorized_source() {
        let safe = r#"Window { Rectangle { cache-rendering-hint: true; } ScrollView { Image { source: root.video-texture; } } }"#;
        assert!(video_has_no_cached_or_colorized_ancestor(safe));
        assert!(!video_has_no_cached_or_colorized_ancestor(&safe.replace(
            "ScrollView {",
            "ScrollView { cache-rendering-hint: true;"
        )));
        assert!(!video_has_no_cached_or_colorized_ancestor(
            &safe.replace("source:", "colorize: red; source:")
        ));
        assert!(!video_has_no_cached_or_colorized_ancestor(&safe.replace(
            "Image { source: root.video-texture; }",
            "Image { source: root.video-texture; } Image { source: root.video-texture; }"
        )));
        assert!(video_has_no_cached_or_colorized_ancestor(include_str!(
            "../../app/ui/app.slint"
        )));
    }
    #[test]
    fn private_frames_and_resize_never_overwrite_or_retire_the_host_borrow() {
        use std::cell::Cell;
        let host_image = Rc::new(Cell::new(1u32));
        let mut targets = PublicationPair {
            displayed: Some(host_image.clone()),
            next: Some(Rc::new(Cell::new(0))),
        };
        for frame in 2..5 {
            targets.next.as_ref().unwrap().set(frame);
            assert!(targets.finish_frame(FrameTarget::Private).is_none());
            assert_eq!(host_image.get(), 1, "private work changed a borrowed image");
            assert!(Rc::ptr_eq(targets.displayed.as_ref().unwrap(), &host_image));
        }
        // A private resize replaces/retires only the unborrowed work target.
        let retired = targets.next.replace(Rc::new(Cell::new(5))).unwrap();
        assert!(!Rc::ptr_eq(&retired, &host_image));
        drop(retired);
        assert!(targets.finish_frame(FrameTarget::Private).is_none());
        assert_eq!(host_image.get(), 1);
        let replacement = targets
            .finish_frame(FrameTarget::PublishNext)
            .unwrap()
            .clone();
        assert_eq!(replacement.get(), 5);
        assert!(!Rc::ptr_eq(&replacement, &host_image));
        // The host replaces its image before the following private frame.
        drop(host_image);
        targets.next.as_ref().unwrap().set(6);
        assert!(targets.finish_frame(FrameTarget::Private).is_none());
        assert_eq!(replacement.get(), 5);
    }
    #[test]
    fn target_delay_preserves_time_and_rejects_mismatched_units() {
        let now = 100_000_000_000i64;
        assert_eq!(
            deadline_delay(now + 50_000_000, now, 50, 0).unwrap(),
            Some(std::time::Duration::from_millis(50))
        );
        assert_eq!(
            deadline_delay(now + 1, now, 50, 0).unwrap(),
            Some(std::time::Duration::from_millis(1))
        );
        assert_eq!(
            deadline_delay(now + 50_000_000, now, 50, 8).unwrap(),
            Some(std::time::Duration::from_millis(42))
        );
        assert_eq!(deadline_delay(now + 8_000_000, now, 50, 8).unwrap(), None);
        assert_eq!(deadline_delay(now, now, 50, 0).unwrap(), None);
        assert_eq!(deadline_delay(now - 1_000_000, now, 50, 0).unwrap(), None);
        assert!(deadline_delay((now + 50_000_000) / 1000, now, 50, 0).is_err());
        assert!(deadline_delay(now + 301_000_000, now, 50, 0).is_err());
        assert!(deadline_delay(i64::MAX, i64::MIN, 50, 0).is_err());
    }
}
