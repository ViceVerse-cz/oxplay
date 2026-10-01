// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite, explicitly invoked Metal surface regression; never part of playback.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("unsupported: this probe requires macOS Apple M1 Metal");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use slint::winit_030::winit;
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::window::{Window, WindowId};

    #[derive(Default)]
    struct Probe {
        window: Option<std::sync::Arc<Window>>,
        result: Option<Result<(), String>>,
        started: Option<std::time::Instant>,
    }
    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let window = event_loop.create_window(
                Window::default_attributes()
                    .with_title("Oxplay finite Metal surface probe")
                    .with_inner_size(winit::dpi::PhysicalSize::new(32, 32))
                    .with_visible(false),
            );
            match window {
                Ok(window) => {
                    window.request_redraw();
                    self.window = Some(std::sync::Arc::new(window));
                    self.started = Some(std::time::Instant::now());
                }
                Err(error) => {
                    self.result = Some(Err(error.to_string()));
                    event_loop.exit();
                }
            }
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            // The first redraw can precede AppKit's visibility update. Metal
            // correctly rejects that drawable as occluded; wait for visibility.
            if self.result.is_none() && matches!(event, WindowEvent::Occluded(false)) {
                self.result = Some(run_probe(self.window.as_ref().unwrap().clone()));
                self.window.take();
                event_loop.exit();
            }
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if self.result.is_none() {
                if self
                    .started
                    .is_some_and(|start| start.elapsed() >= std::time::Duration::from_millis(300))
                {
                    // An initially visible window need not emit an Occluded(false)
                    // transition. Give AppKit one event-loop turn before probing it.
                    self.result = Some(run_probe(self.window.as_ref().unwrap().clone()));
                    self.window.take();
                    event_loop.exit();
                } else if self
                    .started
                    .is_some_and(|start| start.elapsed() > std::time::Duration::from_secs(5))
                {
                    self.result = Some(Err(
                        "probe window did not become visible within 5 seconds".into()
                    ));
                    event_loop.exit();
                } else {
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                        std::time::Instant::now() + std::time::Duration::from_millis(100),
                    ));
                }
            }
        }
    }
    let mut probe = Probe::default();
    EventLoop::new()?.run_app(&mut probe)?;
    probe.result.ok_or("probe did not run")??;
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_probe(
    window: std::sync::Arc<slint::winit_030::winit::window::Window>,
) -> Result<(), String> {
    use slint::wgpu_30::wgpu;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wgpu::hal::Adapter as _;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        struct ThreadWake(std::thread::Thread);
        impl std::task::Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut cx = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::park();
        }
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::VALIDATION,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // A hosted hidden window is intentionally rejected as Occluded by Metal.
    // Use WGPU's supported standalone CAMetalLayer surface on this main thread;
    // the real layer owns hardware drawables without requiring screen exposure.
    let layer: objc2::rc::Retained<objc2::runtime::AnyObject> =
        unsafe { objc2::msg_send![objc2::class!(CAMetalLayer), new] };
    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
            objc2::rc::Retained::as_ptr(&layer).cast_mut().cast(),
        ))
    }
    .map_err(|e| e.to_string())?;
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .map_err(|e| e.to_string())?;
    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Metal || !info.name.starts_with("Apple M1") {
        return Err(format!("expected Apple M1 Metal, got {info:?}"));
    }
    // Exercise the actual compiled HAL adapter helper, not a detached policy constant.
    let ordered = unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }
        .ok_or("Metal HAL adapter unavailable")?
        .get_ordered_texture_usages();
    let original = wgpu::TextureUses::INCLUSIVE
        | wgpu::TextureUses::COLOR_TARGET
        | wgpu::TextureUses::DEPTH_STENCIL_WRITE;
    assert_eq!(ordered - wgpu::TextureUses::PRESENT, original);
    assert!(ordered.contains(wgpu::TextureUses::PRESENT));
    let skip = |old, new, mask: wgpu::TextureUses| old == new && mask.contains(old);
    for old in wgpu::TextureUses::all().iter() {
        for new in wgpu::TextureUses::all().iter() {
            assert_eq!(
                skip(old, new, ordered) != skip(old, new, original),
                old == wgpu::TextureUses::PRESENT && old == new,
            );
        }
    }
    let (device, queue) =
        block_on(adapter.request_device(&Default::default())).map_err(|e| e.to_string())?;
    let errors = Arc::new(AtomicUsize::new(0));
    let errors_for_callback = errors.clone();
    device.on_uncaptured_error(Arc::new(move |error| {
        errors_for_callback.fetch_add(1, Ordering::Relaxed);
        eprintln!("uncaptured GPU error: {error}");
    }));
    for size in [32, 48] {
        let _ =
            window.request_inner_size(slint::winit_030::winit::dpi::PhysicalSize::new(size, size));
        let config = surface
            .get_default_config(&adapter, size, size)
            .ok_or("no surface configuration")?;
        surface.configure(&device, &config);
        for (step, render) in [false, true, true, true].into_iter().enumerate() {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let frame = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                other => return Err(format!("surface acquisition failed: {other:?}")),
            };
            if render {
                let view = frame.texture.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: (step % 2) as f64,
                                    g: 0.,
                                    b: 0.,
                                    a: 1.,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        ..Default::default()
                    });
                }
                queue.submit([encoder.finish()]);
            }
            // First fresh drawable intentionally has no scene submission.
            // WGPU must initialize it before its real presentation.
            queue.present(frame);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(5)),
                })
                .map_err(|e| e.to_string())?;
            if let Some(error) = block_on(scope.pop()) {
                return Err(format!(
                    "validation error at size={size} step={step}: {error}"
                ));
            }
            eprintln!("present probe: size={size} render={render} completed");
        }
    }
    assert_eq!(errors.load(Ordering::Relaxed), 0);
    eprintln!(
        "present probe passed: {} {:?}; 8 presentations, 2 fresh no-scene cases, 0 GPU errors",
        info.name, info.backend
    );
    Ok(())
}
