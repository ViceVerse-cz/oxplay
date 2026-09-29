// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared Slint chrome delegates only native window operations to Winit.
//!
//! The existing close-request handler remains authoritative. There is one
//! window and one presenter; changing chrome or appearance never recreates them.
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
/// Pinned Slint's FemtoVG configuration prefers a transparency-capable config;
/// an opaque shared UI background remains opaque until the user opts in.
pub fn attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes.with_transparent(true).with_blur(false)
}

#[derive(Default)]
pub struct Controller {
    requested_blur: Cell<Option<(WindowId, bool)>>,
}

impl Controller {
    /// Called on native-window availability and relevant window/mode changes,
    /// never on a timer. Winit exposes a blur request, not a success getter.
    pub fn synchronize(&self, app: &App) {
        let Some((translucency, blur, native_resize, maximized, description)) = app
            .window()
            .with_winit_window(|window| {
                let handle = window.window_handle().ok();
                let (translucency, blur, description) = match handle.as_ref().map(|h| h.as_raw()) {
                    Some(RawWindowHandle::AppKit(_)) => (
                        true,
                        true,
                        "Translucency is optional. Native macOS blur is experimental; system settings may affect its appearance.",
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
                let custom = app.get_custom_chrome() && !app.get_native_video_child();
                let requested = custom
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
                    custom && translucency,
                    custom && blur,
                    matches!(handle.as_ref().map(|h| h.as_raw()), Some(RawWindowHandle::AppKit(_))),
                    window.is_maximized(),
                    if custom {
                        description
                    } else {
                        "Custom window appearance is unavailable in the native-child diagnostic."
                    },
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
        if app.get_window_native_resize() != native_resize {
            app.set_window_native_resize(native_resize);
        }
        if app.get_window_maximized() != maximized {
            app.set_window_maximized(maximized);
        }
        if app.get_window_appearance_status() != description {
            app.set_window_appearance_status(description.into());
        }
    }
}

fn normal_chrome(app: &App) -> bool {
    app.get_custom_chrome()
        && !app.get_native_video_child()
        && !app.get_picture_in_picture()
        && !app.get_fullscreen_active()
}

pub fn bind(app: &App) -> Rc<Controller> {
    let controller = Rc::new(Controller::default());
    let weak = app.as_weak();
    app.on_window_minimize(move || {
        if let Some(app) = weak.upgrade().filter(normal_chrome) {
            app.window().set_minimized(true);
        }
    });
    let weak = app.as_weak();
    app.on_window_toggle_maximize(move || {
        if let Some(app) = weak.upgrade().filter(normal_chrome) {
            // Slint owns the maximized property; native-only changes would be
            // vulnerable to being overwritten by a later property update.
            app.window().set_maximized(!app.window().is_maximized());
        }
    });
    let weak = app.as_weak();
    app.on_window_close(move || {
        if let Some(app) = weak.upgrade()
            && app
                .window()
                .dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested)
                .is_err()
        {
            app.set_status("The window could not be closed. Try again.".into());
        }
    });
    let weak = app.as_weak();
    let appearance = controller.clone();
    app.on_window_appearance_changed(move || {
        if let Some(app) = weak.upgrade() {
            appearance.synchronize(&app);
        }
    });
    controller.synchronize(app);
    controller
}
