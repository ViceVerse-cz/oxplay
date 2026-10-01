// SPDX-License-Identifier: GPL-3.0-or-later
//! A compact mode of the existing window, never another presenter or player.
//!
//! Pinned Slint's Winit adapter owns `always-on-top`; the shared Window binds
//! that property to picture-in-picture (normal mode is not topmost). Winit
//! 0.30.13 offers no window-level getter, and its level request is an OS hint.
//! Native Wayland does not implement that request and is rejected explicitly.
use crate::App;
use slint::winit_030::winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    monitor::MonitorHandle,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Window, WindowButtons, WindowId},
};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::cell::RefCell;

const NORMAL_MINIMUM: LogicalSize<f64> = LogicalSize::new(760.0, 600.0);
const COMPACT_MINIMUM: LogicalSize<f64> = LogicalSize::new(360.0, 240.0);
const COMPACT_SIZE: LogicalSize<f64> = LogicalSize::new(480.0, 270.0);

#[derive(Clone)]
struct Saved {
    window: WindowId,
    monitor: Option<MonitorHandle>,
    size: LogicalSize<f64>,
    position: PhysicalPosition<i32>,
    maximized: bool,
    buttons: WindowButtons,
    decorated: bool,
    decoration: LogicalSize<f64>,
}

#[derive(Clone)]
struct CompactGeometry {
    monitor: Option<MonitorHandle>,
    size: LogicalSize<f64>,
    position: PhysicalPosition<i32>,
}

#[derive(Default)]
pub struct Controller {
    saved: RefCell<Option<Saved>>,
    compact: RefCell<Option<CompactGeometry>>,
}

fn available(window: &Window) -> Result<(), &'static str> {
    match window
        .window_handle()
        .map_err(|_| "The window is not ready for picture-in-picture.")?
        .as_raw()
    {
        RawWindowHandle::AppKit(_)
        | RawWindowHandle::Win32(_)
        | RawWindowHandle::Xlib(_)
        | RawWindowHandle::Xcb(_) => Ok(()),
        RawWindowHandle::Wayland(_) => Err(
            "Picture-in-picture needs an always-on-top window, which this native Wayland backend does not support.",
        ),
        _ => Err("Picture-in-picture is unavailable on this window backend."),
    }
}

/// Checks native API capability, not compositor or platform qualification.
/// Call from an active render/window callback after the native window exists.
/// Slint `show()` may return before Winit has created that window.
pub fn availability(window: &slint::Window) -> Result<(), &'static str> {
    window
        .with_winit_window(available)
        .unwrap_or(Err("The window is not ready for picture-in-picture."))
}

impl Controller {
    pub fn active(&self) -> bool {
        self.saved.borrow().is_some()
    }

    /// Caller retires hidden-row focus and closes popups before entering.
    /// The normal shared UI owns the matching native minimum-size bindings.
    pub fn enter(&self, app: &App) -> Result<(), &'static str> {
        if self.active() {
            return Ok(());
        }
        if app.window().is_fullscreen() {
            return Err("Leave fullscreen before opening picture-in-picture.");
        }
        let compact = self.compact.borrow().clone();
        let (saved, compact_size, compact_position) = app
            .window()
            .with_winit_window(|window| {
                available(window)?;
                if window.fullscreen().is_some() {
                    return Err("Wait for fullscreen to close before opening picture-in-picture.");
                }
                let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                if !valid_size(size) {
                    return Err("The window size is not ready for picture-in-picture.");
                }
                let saved = Saved {
                    window: window.id(),
                    monitor: window.current_monitor(),
                    size,
                    position: window
                        .outer_position()
                        .map_err(|_| "The window position is unavailable.")?,
                    maximized: window.is_maximized(),
                    buttons: window.enabled_buttons(),
                    decorated: window.is_decorated(),
                    decoration: LogicalSize::new(
                        f64::from(
                            window
                                .outer_size()
                                .width
                                .saturating_sub(window.inner_size().width),
                        ) / window.scale_factor(),
                        f64::from(
                            window
                                .outer_size()
                                .height
                                .saturating_sub(window.inner_size().height),
                        ) / window.scale_factor(),
                    ),
                };
                // Remember the user's compact placement between toggles, but
                // resolve monitor identity against today's geometry. A removed
                // display must not strand the borderless drag/return controls.
                let monitor = restore_monitor(
                    compact
                        .as_ref()
                        .and_then(|compact| compact.monitor.as_ref()),
                    window.available_monitors(),
                    || {
                        window
                            .current_monitor()
                            .or_else(|| window.primary_monitor())
                    },
                );
                let geometry = compact
                    .as_ref()
                    .map(|compact| (compact.size, compact.position))
                    .unwrap_or((COMPACT_SIZE, saved.position));
                let geometry = monitor
                    .map(|monitor| {
                        fit_window(
                            geometry.0,
                            geometry.1,
                            monitor.position(),
                            monitor.size(),
                            monitor.scale_factor(),
                            (0, 0),
                            COMPACT_MINIMUM,
                        )
                    })
                    .unwrap_or((geometry.0, saved.position));
                Ok((saved, geometry.0, geometry.1))
            })
            .ok_or("The window is not ready for picture-in-picture.")??;

        // No RefCell guard spans a Slint callback/property update. Retain the
        // first snapshot throughout minimize/restore and repeated entry requests.
        *self.saved.borrow_mut() = Some(saved.clone());
        // Slint reapplies native decorations from Window.no-frame whenever
        // window properties update; use the shared binding, not only Winit.
        app.set_window_borderless(true);
        app.set_picture_in_picture(true);
        let weak = app.as_weak();
        app.on_pip_drag(move || {
            let Some(app) = weak.upgrade() else { return };
            if app.get_picture_in_picture() {
                // Invoked synchronously after the video-background drag threshold.
                // macOS can consume release: no pressed-state latch is kept.
                let result = app
                    .window()
                    .with_winit_window(|window| window.drag_window());
                if !matches!(result, Some(Ok(()))) {
                    app.set_status("The window manager could not move picture in picture.".into());
                }
            }
        });
        app.window().with_winit_window(|window| {
            window.set_maximized(false);
            window.set_decorations(false);
            // Supported by macOS/Windows. X11 ignores button hints; the host
            // must also reject observed native fullscreen while PiP is active.
            window.set_enabled_buttons(saved.buttons & !WindowButtons::MAXIMIZE);
            window.set_min_inner_size(Some(COMPACT_MINIMUM));
        });
        app.window().set_size(slint::LogicalSize::new(
            compact_size.width as f32,
            compact_size.height as f32,
        ));
        app.window()
            .with_winit_window(|window| window.set_outer_position(compact_position));
        Ok(())
    }

    /// Restores the existing window. Native close/Escape can call this instead
    /// of hiding it; ordinary shutdown may drop this value with no native work.
    /// Unavailable windows retain the snapshot, allowing an explicit retry.
    /// A native fullscreen transition also retains it: the host requests
    /// fullscreen=false, then retries after an observed window event confirms
    /// fullscreen has ended. There is no polling timer or implicit retry here.
    pub fn exit(&self, app: &App) -> Result<(), &'static str> {
        let Some(saved) = self.saved.borrow().as_ref().cloned() else {
            return Ok(());
        };
        let (size, position, compact) = app
            .window()
            .with_winit_window(|window| {
                if window.id() != saved.window {
                    return Err("The original picture-in-picture window is no longer available.");
                }
                if window.fullscreen().is_some() || app.window().is_fullscreen() {
                    return Err("Leave fullscreen before restoring the main window.");
                }
                // PiP may have been dragged to another display. Restore the
                // original host there while it exists, using a fresh handle's
                // current geometry/scale rather than the captured monitor data.
                let monitor =
                    restore_monitor(saved.monitor.as_ref(), window.available_monitors(), || {
                        window
                            .current_monitor()
                            .or_else(|| window.primary_monitor())
                    });
                let geometry = monitor
                    .map(|monitor| {
                        fit_restore(
                            saved.size,
                            saved.position,
                            monitor.position(),
                            monitor.size().width,
                            monitor.size().height,
                            monitor.scale_factor(),
                            restore_decoration(saved.decoration, monitor.scale_factor()),
                        )
                    })
                    .unwrap_or((saved.size, saved.position));
                // A minimized/native-maximized transient is not the compact
                // rectangle the user chose. Keep the last useful one instead.
                let compact = if window.is_minimized() == Some(true) || window.is_maximized() {
                    None
                } else {
                    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
                    valid_size(size)
                        .then(|| window.outer_position().ok())
                        .flatten()
                        .map(|position| CompactGeometry {
                            monitor: window.current_monitor(),
                            size,
                            position,
                        })
                };
                Ok((geometry.0, geometry.1, compact))
            })
            .ok_or("The window is not ready to restore from picture-in-picture.")??;

        self.saved.borrow_mut().take();
        if let Some(compact) = compact {
            *self.compact.borrow_mut() = Some(compact);
        }
        app.set_picture_in_picture(false);
        app.set_window_borderless(!saved.decorated);
        app.window().with_winit_window(|window| {
            window.set_decorations(saved.decorated);
            window.set_enabled_buttons(saved.buttons);
            window.set_min_inner_size(Some(NORMAL_MINIMUM));
        });
        // Winit rebuilds the macOS decoration mask without FullSizeContentView.
        // Reapply the integrated titlebar before interpreting the saved client
        // size; doing this on the later resize event changes that geometry.
        // The video-only change may have reconciled while still borderless.
        // Later Slint decoration updates see the requested state already set,
        // so Winit's early return preserves this restored native style bit.
        app.invoke_window_appearance_changed();
        app.window().set_size(slint::LogicalSize::new(
            size.width as f32,
            size.height as f32,
        ));
        app.window().with_winit_window(|window| {
            window.set_outer_position(position);
            // Maximization is restored last; no replay, reload or fullscreen.
            window.set_maximized(saved.maximized);
        });
        Ok(())
    }
}

/// Prefer the original display, not the display to which compact PiP moved.
/// A disconnected/unknown original display falls back to a current live one.
/// Kept independent of native handles so disconnect/reorder semantics are tested
/// without constructing a second native window or involving the media player.
fn restore_monitor<M: PartialEq>(
    original: Option<&M>,
    available: impl IntoIterator<Item = M>,
    fallback: impl FnOnce() -> Option<M>,
) -> Option<M> {
    original
        .and_then(|original| available.into_iter().find(|monitor| monitor == original))
        .or_else(fallback)
}

// Borderless outer-minus-inner is zero, so restore from the captured frame
// extents rather than measuring the compact window. Account for a DPI change.
fn restore_decoration(decoration: LogicalSize<f64>, scale: f64) -> (u32, u32) {
    (
        (decoration.width * scale).round() as u32,
        (decoration.height * scale).round() as u32,
    )
}

fn valid_size(size: LogicalSize<f64>) -> bool {
    size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0
}

/// Keep the logical client size across DPI changes. Clamp the physical outer
/// rectangle to the current monitor, accounting for native window decorations.
/// Winit exposes monitor bounds, not a portable work-area API. On displays
/// smaller than the normal UI minimum, the minimum wins and the origin stays
/// inside the monitor; this does not promise every control fits that display.
fn fit_restore(
    saved: LogicalSize<f64>,
    position: PhysicalPosition<i32>,
    origin: PhysicalPosition<i32>,
    width: u32,
    height: u32,
    scale: f64,
    decoration: (u32, u32),
) -> (LogicalSize<f64>, PhysicalPosition<i32>) {
    fit_window(
        saved,
        position,
        origin,
        PhysicalSize::new(width, height),
        scale,
        decoration,
        NORMAL_MINIMUM,
    )
}

fn fit_window(
    saved: LogicalSize<f64>,
    position: PhysicalPosition<i32>,
    origin: PhysicalPosition<i32>,
    bounds: PhysicalSize<u32>,
    scale: f64,
    decoration: (u32, u32),
    minimum: LogicalSize<f64>,
) -> (LogicalSize<f64>, PhysicalPosition<i32>) {
    let PhysicalSize { width, height } = bounds;
    if !scale.is_finite() || scale <= 0.0 || width == 0 || height == 0 {
        return (saved, position);
    }
    let max_width = ((f64::from(width) - f64::from(decoration.0)) / scale).max(minimum.width);
    let max_height = ((f64::from(height) - f64::from(decoration.1)) / scale).max(minimum.height);
    let size = LogicalSize::new(
        saved.width.clamp(minimum.width, max_width),
        saved.height.clamp(minimum.height, max_height),
    );
    let x_min = f64::from(origin.x);
    let y_min = f64::from(origin.y);
    let x_max =
        (f64::from(origin.x) + f64::from(width) - size.width * scale - f64::from(decoration.0))
            .max(x_min);
    let y_max =
        (f64::from(origin.y) + f64::from(height) - size.height * scale - f64::from(decoration.1))
            .max(y_min);
    (
        size,
        PhysicalPosition::new(
            f64::from(position.x).clamp(x_min, x_max).round() as i32,
            f64::from(position.y).clamp(y_min, y_max).round() as i32,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_pip_to_another_display_does_not_move_the_restored_main_window() {
        // Display order can change; identity rather than index selects original.
        assert_eq!(
            restore_monitor(Some(&7), [9, 7], || panic!("original is connected")),
            Some(7)
        );
        assert_eq!(restore_monitor(Some(&7), [9], || Some(9)), Some(9));
        assert_eq!(restore_monitor(None, [9], || Some(9)), Some(9));
        assert_eq!(restore_monitor(Some(&7), [], || None), None);

        let saved = LogicalSize::new(1000.0, 700.0);
        let position = PhysicalPosition::new(-2000, 120);
        // Restoring on the original left-hand monitor preserves the real main
        // window position, even if the compact window moved to the right one.
        let original = fit_restore(
            saved,
            position,
            PhysicalPosition::new(-2560, 0),
            2560,
            1440,
            1.0,
            (0, 28),
        );
        assert_eq!(original, (saved, position));
        let disconnected = fit_restore(
            saved,
            position,
            PhysicalPosition::new(0, 0),
            1920,
            1080,
            1.0,
            (0, 28),
        );
        assert_eq!(disconnected, (saved, PhysicalPosition::new(0, 120)));
    }

    #[test]
    fn borderless_restore_uses_original_frame_extents_at_the_new_scale() {
        let frame = LogicalSize::new(8.0, 28.0);
        assert_eq!(restore_decoration(frame, 2.0), (16, 56));
        assert_eq!(restore_decoration(frame, 1.0), (8, 28));
        let (size, position) = fit_restore(
            LogicalSize::new(1200.0, 800.0),
            PhysicalPosition::new(5000, 5000),
            PhysicalPosition::new(0, 0),
            1920,
            1080,
            2.0,
            restore_decoration(frame, 2.0),
        );
        assert_eq!(size, LogicalSize::new(952.0, 600.0));
        assert_eq!(position, PhysicalPosition::new(0, 0));
        assert_eq!(restore_decoration(LogicalSize::new(0.0, 0.0), 2.0), (0, 0));
    }

    #[test]
    fn restore_preserves_logical_size_across_dpi_and_negative_desktop_origin() {
        let (size, position) = fit_restore(
            LogicalSize::new(900.0, 650.0),
            PhysicalPosition::new(-1800, 70),
            PhysicalPosition::new(-2560, 0),
            2560,
            1600,
            2.0,
            (0, 56),
        );
        assert_eq!(size, LogicalSize::new(900.0, 650.0));
        assert_eq!(position, PhysicalPosition::new(-1800, 70));
    }

    #[test]
    fn restore_clamps_missing_monitor_position_and_accounts_for_decorations() {
        let (size, position) = fit_restore(
            LogicalSize::new(1800.0, 1000.0),
            PhysicalPosition::new(4000, -2000),
            PhysicalPosition::new(0, 0),
            1920,
            1080,
            1.0,
            (16, 40),
        );
        assert_eq!(size, LogicalSize::new(1800.0, 1000.0));
        assert_eq!(position, PhysicalPosition::new(104, 0));
    }

    #[test]
    fn restore_never_shrinks_below_shared_ui_minimum() {
        let (size, position) = fit_restore(
            LogicalSize::new(1320.0, 860.0),
            PhysicalPosition::new(-900, -900),
            PhysicalPosition::new(0, 0),
            640,
            480,
            1.0,
            (0, 24),
        );
        assert_eq!(size, NORMAL_MINIMUM);
        assert_eq!(position, PhysicalPosition::new(0, 0));
        assert!(!valid_size(LogicalSize::new(f64::NAN, 1.0)));
        assert!(!valid_size(LogicalSize::new(1.0, 0.0)));
    }
}
