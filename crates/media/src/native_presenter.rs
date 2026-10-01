// SPDX-License-Identifier: GPL-3.0-or-later
//! Native media output into the same GPU textures the Slint compositor samples.
//! The custom libmpv backend retains its demux, audio clock and subtitles.
use crate::presenter::{FrameTarget, PublicationPair, RenderStats};
use crate::wgpu_ambient::WgpuAmbientSampler;
use crate::{MediaError, Player, Result, checked, ffi};
use std::{
    ffi::c_void,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

#[cfg(target_os = "macos")]
#[path = "native_metal.rs"]
mod platform;

#[cfg(target_os = "linux")]
pub(crate) async fn linux_gpu_configuration(
    settings: slint::wgpu_30::WGPUSettings,
) -> Result<slint::wgpu_30::WGPUConfiguration> {
    platform::configuration(settings).await
}
#[cfg(target_os = "linux")]
#[path = "native_vulkan.rs"]
mod platform;
#[cfg(windows)]
#[path = "native_d3d.rs"]
mod platform;

struct Target {
    native: platform::Target,
    image: slint::Image,
    width: u32,
    height: u32,
}
impl Target {
    fn new(
        output: &mut platform::Output,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let native = output.new_target(width, height)?;
        let texture = &native.texture;
        // WGPU must know the allocation is initialized before native code
        // writes it, or its lazy initialization would erase the first frame.
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Initialize native video allocation"),
        });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear new native video allocation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        queue.submit([encoder.finish()]);
        let image = slint::Image::try_from(texture.clone())
            .map_err(|_| MediaError("Cannot publish a native GPU video texture".into()))?;
        Ok(Self {
            native,
            image,
            width,
            height,
        })
    }
    fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
}

/// UI-thread-only owner; native callbacks only enqueue coalesced UI work.
pub struct NativePresenter {
    player: Player,
    device: wgpu::Device,
    queue: wgpu::Queue,
    context: *mut ffi::RenderContext,
    output: platform::Output,
    targets: PublicationPair<Target>,
    retired: Vec<Target>,
    pending_frame: Option<ffi::FrameInfo>,
    deadline_timer: slint::Timer,
    scheduled_deadline: Option<i64>,
    timing_lead_ms: u32,
    prepare_ms: u32,
    observed_load: u64,
    published_load: u64,
    stats: RenderStats,
    measure: bool,
    graphics_info: String,
    ambient: WgpuAmbientSampler,
    ambient_enabled: bool,
}

impl NativePresenter {
    pub fn new(
        player: &Player,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        display: Option<u32>,
    ) -> Result<Self> {
        if player.inner.renderer_attached.get() {
            return Err(MediaError("A media presenter is already attached".into()));
        }
        let measure = std::env::var_os("OXPLAY_MEDIA_TIMING").is_some_and(|v| v == "1");
        let wake = player.inner.wake.clone();
        let ambient_wake = wake.clone();
        let ambient = WgpuAmbientSampler::new(
            device,
            queue,
            Arc::new(move || {
                // A paused first frame has no subsequent decode notification.
                // This finite completion redraw publishes only its tiny summary.
                ambient_wake.due.store(true, Ordering::Release);
                ambient_wake.notify();
            }),
            measure,
        )?;
        let mut output = platform::Output::new(device, queue)?;
        let mut context = std::ptr::null_mut();
        // SAFETY: output verifies the native handles belong to these WGPU
        // objects. Both objects and callback userdata outlive the render context.
        unsafe {
            output.create_context(player.inner.raw, &wake, &mut context)?;
            ffi::mpv_render_context_set_update_callback(
                context,
                Some(crate::render_wake),
                Arc::as_ptr(&wake).cast_mut().cast(),
            );
        }
        #[cfg(target_os = "macos")]
        {
            let clock = match crate::macos::PresentationClock::new_native(wake, display) {
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
        #[cfg(not(target_os = "macos"))]
        let _ = display;
        player.inner.renderer_attached.set(true);
        let info = device.adapter_info();
        Ok(Self {
            player: player.clone(),
            device: device.clone(),
            queue: queue.clone(),
            context,
            output,
            targets: PublicationPair::default(),
            retired: Vec::with_capacity(2),
            pending_frame: None,
            deadline_timer: slint::Timer::default(),
            scheduled_deadline: None,
            timing_lead_ms: player.inner.snapshot.borrow().timing_lead_ms,
            prepare_ms: player.inner.snapshot.borrow().prepare_ms,
            observed_load: 0,
            published_load: 0,
            stats: RenderStats {
                stable_video_target: true,
                ..Default::default()
            },
            measure,
            graphics_info: format!(
                "Native {:?} video + UI | {} | {}",
                info.backend, info.name, info.driver
            ),
            ambient,
            ambient_enabled: false,
        })
    }
    pub fn graphics_info(&self) -> &str {
        &self.graphics_info
    }
    pub fn stats(&self) -> RenderStats {
        RenderStats {
            ambient: self.ambient.stats(),
            ..self.stats
        }
    }
    pub fn observed_load_request(&self) -> u64 {
        self.observed_load
    }
    pub fn set_ambient_sampling(&mut self, enabled: bool) {
        self.ambient_enabled = enabled;
        self.ambient.set_enabled(enabled);
    }
    pub fn take_ambient_sample(&mut self) -> Option<crate::AmbientSample> {
        self.ambient.take()
    }
    pub fn set_display(&self, display: Option<u32>) -> Result<()> {
        #[cfg(target_os = "macos")]
        if let Some(display) = display
            && let Some(clock) = self.player.inner.presentation_clock.borrow().as_ref()
        {
            clock.set_native_display(display)?;
        }
        #[cfg(not(target_os = "macos"))]
        let _ = display;
        Ok(())
    }

    /// Render only at the owning Slint BeforeRendering boundary. No CPU video
    /// pixels are read, copied or uploaded by this adapter.
    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        allow_publication: bool,
    ) -> Result<Option<slint::Image>> {
        self.stats.ui_draws += 1;
        let load = self.player.inner.active_load_request.get();
        let changed_load = load != self.observed_load;
        if changed_load {
            self.observed_load = load;
            self.published_load = 0;
            self.pending_frame = None;
            self.deadline_timer.stop();
            self.scheduled_deadline = None;
        }
        if let Err(error) = self.ambient.collect(load) {
            eprintln!("ambient sampling disabled: {error}");
        }
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let limit = self.device.limits().max_texture_dimension_2d.min(4096);
        if width > limit || height > limit {
            return Err(MediaError(
                "Native video target exceeds the validated pixel limit".into(),
            ));
        }
        let ready = allow_publication && self.player.current_load_frame_ready();
        let first = ready && self.published_load != load;
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
        if !update && !due && !resized && !first {
            self.sample_displayed(load, ready);
            return Ok(None);
        }
        if update {
            self.stats.updates += 1;
            let flags = unsafe { ffi::mpv_render_context_update(self.context) };
            self.pending_frame = if flags & 1 != 0 {
                let mut info = ffi::FrameInfo::default();
                unsafe {
                    checked(
                        ffi::mpv_render_context_get_info(
                            self.context,
                            ffi::RenderParam::new(11, &mut info),
                        ),
                        "Read native frame deadline",
                    )?;
                }
                (info.flags & 1 != 0).then_some(info)
            } else {
                None
            };
        }
        #[cfg(target_os = "macos")]
        let display_clock = self.player.inner.wake.clock.enabled.load(Ordering::Acquire);
        #[cfg(not(target_os = "macos"))]
        let display_clock = false;
        if let Some(info) = self.pending_frame {
            let now = unsafe { ffi::mpv_get_time_ns(self.player.inner.raw) };
            if info.flags & (2 | 4) == 0 && info.target_time != 0 {
                let delay = crate::presenter::deadline_delay(
                    info.target_time,
                    now,
                    self.timing_lead_ms,
                    if display_clock { 0 } else { self.prepare_ms },
                )?;
                if !display_clock
                    && self.timing_lead_ms > 0
                    && let Some(delay) = delay
                {
                    if self.scheduled_deadline != Some(info.target_time)
                        || !self.deadline_timer.running()
                    {
                        let wake = self.player.inner.wake.clone();
                        self.deadline_timer
                            .start(slint::TimerMode::SingleShot, delay, move || {
                                wake.due.store(true, Ordering::Release);
                                wake.notify();
                            });
                        self.scheduled_deadline = Some(info.target_time);
                        self.stats.scheduled_wakes += 1;
                    }
                    return Ok(None);
                }
            }
            if self.measure && info.target_time != 0 {
                let offset = info.target_time.saturating_sub(now) / 1000;
                self.stats.target_time_samples += 1;
                if offset > 0 {
                    self.stats.early_us += offset as u64;
                    self.stats.max_early_us = self.stats.max_early_us.max(offset as u64);
                } else {
                    self.stats.late_us += offset.unsigned_abs();
                    self.stats.max_late_us = self.stats.max_late_us.max(offset.unsigned_abs());
                }
            }
        }
        self.deadline_timer.stop();
        self.scheduled_deadline = None;
        if self.pending_frame.is_none() && !first && (!resized || self.targets.displayed.is_none())
        {
            // A readback completion can reveal a paused glow without asking
            // mpv to render the same frame again.
            self.sample_displayed(load, ready);
            return Ok(None);
        }
        if self.output.busy(self.context)? {
            // Native completion invokes the same bounded wake when a slot
            // becomes free. Leave the mpv frame unconsumed until that wake.
            self.player.inner.wake.due.store(true, Ordering::Release);
            return Ok(None);
        }
        if self
            .targets
            .next
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let target = Target::new(&mut self.output, &self.device, &self.queue, width, height)?;
            if let Some(old) = self.targets.next.replace(target) {
                self.retired.push(old);
            }
            self.stats.target_allocations += 1;
        }
        let mode = FrameTarget::choose(true, ready, load, self.published_load, !resized);
        let target = self
            .targets
            .render_target(mode)
            .expect("native target allocated");
        let start = self.measure.then(Instant::now);
        // The platform adapter establishes native barriers/fences, while the
        // WGPU encoder records matching tracked texture usage for the UI read.
        self.output
            .render(self.context, &target.native, width, height)?;
        if let Some(start) = start {
            let elapsed = start.elapsed().as_micros() as u64;
            self.stats.timing_samples += 1;
            self.stats.mpv_render_us += elapsed;
            self.stats.max_mpv_render_us = self.stats.max_mpv_render_us.max(elapsed);
        }
        if self.ambient_enabled && mode != FrameTarget::Private {
            let now = Instant::now();
            if self.ambient.due(now, load) {
                let snapshot = self.player.inner.snapshot.borrow();
                let rect =
                    crate::ambient::content_rect(width, height, snapshot.width, snapshot.height);
                drop(snapshot);
                if let Err(error) = self.ambient.sample(&target.native.texture, rect, load, now) {
                    eprintln!("ambient sampling disabled: {error}");
                }
            }
        }
        self.pending_frame = None;
        self.stats.video_draws += 1;
        match mode {
            FrameTarget::Private => self.stats.private_target_draws += 1,
            FrameTarget::PublishNext => self.stats.target_publications += 1,
            FrameTarget::ReuseDisplayed => self.stats.stable_target_draws += 1,
        }
        let image = self.targets.finish_frame(mode).map(|t| t.image.clone());
        if image.is_some() {
            self.published_load = load;
        }
        self.refresh_bytes();
        Ok(image)
    }
    fn refresh_bytes(&mut self) {
        self.stats.target_bytes = self
            .targets
            .displayed
            .iter()
            .chain(self.targets.next.iter())
            .chain(self.retired.iter())
            .map(Target::bytes)
            .sum();
    }
    fn sample_displayed(&mut self, load: u64, ready: bool) {
        if !self.ambient_enabled || !ready || self.published_load != load {
            return;
        }
        let now = Instant::now();
        if !self.ambient.due(now, load) {
            return;
        }
        let Some(target) = self.targets.displayed.as_ref() else {
            return;
        };
        let snapshot = self.player.inner.snapshot.borrow();
        let rect = crate::ambient::content_rect(
            target.width,
            target.height,
            snapshot.width,
            snapshot.height,
        );
        drop(snapshot);
        if let Err(error) = self.ambient.sample(&target.native.texture, rect, load, now) {
            eprintln!("ambient sampling disabled: {error}");
        }
    }
    pub fn after_render(&mut self) {
        // WGPU submitted commands own their texture references. The native
        // renderer retains its references through command-buffer completion.
        self.retired.clear();
        self.refresh_bytes();
    }
}
impl Drop for NativePresenter {
    fn drop(&mut self) {
        self.deadline_timer.stop();
        self.ambient.delete();
        #[cfg(target_os = "macos")]
        self.player.inner.presentation_clock.borrow_mut().take();
        unsafe {
            ffi::mpv_render_context_set_update_callback(self.context, None, std::ptr::null_mut());
            // Custom native backends drain retained submissions before their
            // completion callback's Wake allocation can be released.
            ffi::mpv_render_context_free(self.context);
        }
        self.player.inner.renderer_attached.set(false);
        self.player.inner.wake.frame.store(false, Ordering::Release);
        #[cfg(windows)]
        self.player.inner.display_request.update(false);
    }
}

unsafe extern "C" fn native_slot_wake(data: *mut c_void) {
    let wake = unsafe { &*data.cast::<crate::Wake>() };
    wake.due.store(true, Ordering::Release);
    wake.notify();
}
