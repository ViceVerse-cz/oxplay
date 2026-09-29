// SPDX-License-Identifier: GPL-3.0-or-later
//! Native window appearance with operating-system-owned controls and frame.
//!
//! The existing close-request handler remains authoritative. There is one
//! window and one presenter; changing chrome or appearance never recreates them.
//! macOS places the shared header under an AppKit titlebar with real traffic
//! lights. Other backends retain their native frame. PiP owns borderlessness.
use crate::App;
use slint::{
    ComponentHandle,
    winit_030::{
        WinitWindowAccessor,
        winit::{
            raw_window_handle::{HasWindowHandle, RawWindowHandle},
            window::{WindowAttributes, WindowId},
        },
    },
};
use std::{cell::Cell, rc::Rc};

/// Must be installed before App::new. X11 cannot add an alpha visual later.
/// Pinned Slint's FemtoVG configuration prefers a transparency-capable config.
/// On macOS Slint overrides transparency from Window.background/no-frame at
/// creation and whenever they change. The shared canvas supplies a neutral tint
/// while the native compositor supplies requested blur where available.
/// Keep native decorations: AppKit owns traffic lights and rounded frame corners.
pub fn attributes(attributes: WindowAttributes, integrated: bool) -> WindowAttributes {
    let attributes = attributes
        .with_decorations(true)
        .with_transparent(true)
        .with_blur(false);
    #[cfg(target_os = "macos")]
    {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        // Apply after with_decorations: that method resets these attributes.
        attributes
            .with_titlebar_transparent(integrated)
            .with_title_hidden(integrated)
            .with_fullsize_content_view(integrated)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = integrated;
        attributes
    }
}

#[derive(Default)]
pub struct Controller {
    requested_blur: Cell<Option<(WindowId, bool)>>,
}

impl Controller {
    /// Called on native-window availability and relevant window/mode changes,
    /// never on a timer. Winit exposes a blur request, not a success getter.
    pub fn synchronize(&self, app: &App) {
        let Some((translucency, blur, description, integrated)) = app
            .window()
            .with_winit_window(|window| {
                let handle = window.window_handle().ok();
                let (translucency, blur, description) = match handle.as_ref().map(|h| h.as_raw()) {
                    Some(RawWindowHandle::AppKit(_)) => (
                        true,
                        true,
                        "Native macOS blur is experimental; system settings may affect its appearance. Turn off translucency for opaque surfaces.",
                    ),
                    Some(RawWindowHandle::Win32(_)) => (
                        true,
                        false,
                        "Translucency depends on your Windows graphics settings. Blur is unavailable on this system.",
                    ),
                    Some(RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) => (
                        true,
                        false,
                        "Your X11 desktop must support transparency. Blur is unavailable on this system.",
                    ),
                    Some(RawWindowHandle::Wayland(_)) => (
                        true,
                        false,
                        "Translucency depends on your Wayland desktop. Blur is unavailable on this system.",
                    ),
                    _ => (false, false, "Window translucency and blur are unavailable on this system."),
                };
                let appearance = !app.get_native_video_child();
                let normal = appearance
                    && !app.get_fullscreen_active()
                    && !app.get_picture_in_picture()
                    && window.is_decorated()
                    && window.fullscreen().is_none();
                #[cfg(target_os = "macos")]
                let integrated = normal && macos::integrate(window, app.get_native_header_height() as f64);
                #[cfg(not(target_os = "macos"))]
                let integrated = {
                    let _ = normal;
                    false
                };
                let requested = appearance
                    && blur
                    && app.get_window_translucent()
                    && app.get_window_blur_enabled()
                    && !app.get_fullscreen_active()
                    && !app.get_picture_in_picture();
                let blur_state = (window.id(), requested);
                if self.requested_blur.get() != Some(blur_state) {
                    window.set_blur(requested);
                    self.requested_blur.set(Some(blur_state));
                }
                (
                    appearance && translucency,
                    appearance && blur,
                    if appearance {
                        description
                    } else {
                        "Window translucency and blur are unavailable in the native-child diagnostic."
                    },
                    integrated,
                )
            })
        else {
            return;
        };
        if app.get_window_translucency_available() != translucency {
            app.set_window_translucency_available(translucency);
        }
        if app.get_window_blur_available() != blur {
            app.set_window_blur_available(blur);
        }
        if app.get_window_appearance_status() != description {
            app.set_window_appearance_status(description.into());
        }
        if app.get_native_header_integrated() != integrated {
            app.set_native_header_integrated(integrated);
        }
    }
}

pub fn bind(app: &App) -> Rc<Controller> {
    let controller = Rc::new(Controller::default());
    let weak = app.as_weak();
    let appearance = controller.clone();
    app.on_window_appearance_changed(move || {
        if let Some(app) = weak.upgrade() {
            appearance.synchronize(&app);
        }
    });
    let weak = app.as_weak();
    app.on_window_header_drag(move || {
        if let Some(app) = weak.upgrade()
            && app.get_native_header_integrated()
            && !app.get_picture_in_picture()
            && !app.get_fullscreen_active()
        {
            // Called by the shared header's otherwise empty hit region. Native
            // traffic lights and Slint controls receive their own input first.
            let _ = app
                .window()
                .with_winit_window(|window| window.drag_window());
        }
    });
    controller.synchronize(app);
    controller
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSView, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility,
    };
    use slint::winit_030::winit::{
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::Window,
    };

    pub(super) fn integrate(window: &Window, header_height: f64) -> bool {
        let Some(_main_thread) = MainThreadMarker::new() else {
            return false;
        };
        let Ok(handle) = window.window_handle() else {
            return false;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return false;
        };
        // SAFETY: the borrowed AppKit raw window handle contains Winit's live
        // NSView. This synchronous UI-thread call keeps the owning Window alive;
        // no native pointer or reference is retained beyond this scope.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let Some(native) = view.window() else {
            return false;
        };
        let mask = native.styleMask();
        if !mask.contains(NSWindowStyleMask::Titled) || mask.contains(NSWindowStyleMask::FullScreen)
        {
            return false;
        }
        // Winit 0.30.13 rebuilds its decoration mask on PiP restoration and
        // drops FullSizeContentView. Restore just this bit; preserve AppKit's
        // existing capabilities and all ownership of native system controls.
        if !mask.contains(NSWindowStyleMask::FullSizeContentView) {
            native.setStyleMask(mask | NSWindowStyleMask::FullSizeContentView);
        }
        if !native.titlebarAppearsTransparent() {
            native.setTitlebarAppearsTransparent(true);
        }
        if native.titleVisibility() != NSWindowTitleVisibility::Hidden {
            native.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        }
        center_traffic_lights(&native, view, header_height);
        true
    }

    fn center_traffic_lights(window: &NSWindow, content: &NSView, height: f64) {
        if !(32.0..=64.0).contains(&height) {
            return;
        }
        let buttons: Option<Vec<_>> = [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ]
        .into_iter()
        .map(|kind| window.standardWindowButton(kind))
        .collect();
        let Some(buttons) = buttons else { return };
        // SAFETY: AppKit access stays synchronous on the main thread while the
        // owning Winit window is alive. Native views are borrowed/retained only
        // for this call; buttons are never replaced, reparented or restyled.
        let Some(parent) = (unsafe { buttons[0].superview() }) else {
            return;
        };
        let Some(container) = (unsafe { parent.superview() }) else {
            return;
        };
        let Some(container_parent) = (unsafe { container.superview() }) else {
            return;
        };
        if std::ptr::eq(parent.as_ref(), content)
            || std::ptr::eq(container.as_ref(), content)
            || buttons.iter().any(|button| {
                unsafe { button.superview() }
                    .is_none_or(|view| !std::ptr::eq::<NSView>(view.as_ref(), parent.as_ref()))
            })
        {
            return;
        }
        let bounds = content.bounds();
        let mut top = bounds.origin;
        top.y = if content.isFlipped() {
            bounds.origin.y
        } else {
            bounds.origin.y + bounds.size.height
        };
        // Native titlebar containers normally cover only the system title row.
        // Extend their hit region to the shared header before moving buttons,
        // so their centers remain inside every native ancestor's bounds.
        let bars: [(&NSView, &NSView); 2] = [
            (container.as_ref(), container_parent.as_ref()),
            (parent.as_ref(), container.as_ref()),
        ];
        for (bar, host) in bars {
            let top = host.convertPoint_fromView(top, Some(content));
            let mut frame = bar.frame();
            frame.size.height = height;
            frame.origin.y = if host.isFlipped() {
                top.y
            } else {
                top.y - height
            };
            if bar.frame() != frame {
                bar.setFrame(frame);
            }
        }
        let mut target = top;
        target.y += if content.isFlipped() {
            height / 2.0
        } else {
            -height / 2.0
        };
        let target = parent.convertPoint_fromView(target, Some(content));
        for button in buttons {
            let frame = button.frame();
            let mut origin = frame.origin;
            origin.y = target.y - frame.size.height / 2.0;
            if (origin.y - frame.origin.y).abs() > 0.25 {
                button.setFrameOrigin(origin);
            }
        }
    }
}
