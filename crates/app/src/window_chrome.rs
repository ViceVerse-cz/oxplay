// SPDX-License-Identifier: GPL-3.0-or-later
//! Native window appearance with operating-system-owned controls and frame.
//!
//! The existing close-request handler remains authoritative. There is one
//! window and one presenter; changing chrome or appearance never recreates them.
//! Normal windows use system decorations; PiP temporarily owns borderlessness.
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
/// creation and whenever they change, keeping the default native frame opaque.
/// Keep native decorations: AppKit owns traffic lights and rounded frame corners.
pub fn attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes
        .with_decorations(true)
        .with_transparent(true)
        .with_blur(false)
}

#[derive(Default)]
pub struct Controller {
    requested_blur: Cell<Option<(WindowId, bool)>>,
}

impl Controller {
    /// Called on native-window availability and relevant window/mode changes,
    /// never on a timer. Winit exposes a blur request, not a success getter.
    pub fn synchronize(&self, app: &App) {
        let Some((translucency, blur, description)) = app
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
                let appearance = !app.get_native_video_child();
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
    controller.synchronize(app);
    controller
}
