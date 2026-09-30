// SPDX-License-Identifier: GPL-3.0-or-later
//! Restricted local-only macOS presenter diagnostic. All controls remain Slint.
#[cfg(target_os = "macos")]
#[path = "native_child_popups.rs"]
mod popups;

use crate::{App, UiState};
use slint::{
    ComponentHandle,
    winit_030::{WinitWindowAccessor, winit},
};
#[cfg(target_os = "macos")]
use std::cell::RefCell;
use std::{cell::Cell, rc::Rc};

pub struct State {
    pub enabled: bool,
    awaiting_layout: Cell<bool>,
    #[cfg(target_os = "macos")]
    presenter: RefCell<Option<oxplay_media::NativeChildPresenter>>,
}
impl State {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            awaiting_layout: Cell::new(true),
            #[cfg(target_os = "macos")]
            presenter: RefCell::new(None),
        }
    }
}

pub fn validate_environment() -> Result<(), &'static str> {
    if [
        "OXPLAY_STABLE_VIDEO_TARGET",
        "OXPLAY_VIDEO_LEAD_MS",
        "OXPLAY_VIDEO_PREPARE_MS",
        "OXPLAY_DIAGNOSTIC_NULL_AUDIO",
        "OXPLAY_DIAGNOSTIC_AUTOSYNC",
        "OXPLAY_MEDIA_TIMING",
        "OXPLAY_GPU_TIMING",
    ]
    .into_iter()
    .any(|name| std::env::var_os(name).is_some())
    {
        return Err(
            "Unset other media rendering/timing experiment variables before the native-child diagnostic",
        );
    }
    Ok(())
}

pub fn setup(app: &App, state: &UiState) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let view = app
            .window()
            .with_winit_window(|window| match window.window_handle().ok()?.as_raw() {
                RawWindowHandle::AppKit(handle) => Some(handle.ns_view),
                _ => None,
            })
            .flatten()
            .ok_or("Native child requires this window's AppKit view")?;
        if state.native_child.presenter.borrow().is_some() {
            return Err("Native child is already initialized".into());
        }
        // SAFETY: called by this window's rendering setup on the AppKit thread.
        // The adapter keeps Winit's content view in place and restores CGL state.
        let presenter = unsafe { oxplay_media::NativeChildPresenter::new(&state.player, view) }
            .map_err(|error| error.to_string())?;
        let info = presenter.graphics_info().to_owned();
        match presenter.window_number() {
            Ok(number) => eprintln!("native child owning window number: {number}"),
            Err(_) => eprintln!("native child owning window number: unavailable"),
        }
        *state.native_child.presenter.borrow_mut() = Some(presenter);
        state.native_child.awaiting_layout.set(true);
        Ok(info)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, state);
        Err("Native child is an unqualified macOS-only diagnostic".into())
    }
}

/// No UI setters/getters while borrowing the presenter. This also works before
/// Slint has applied the input event or evaluated changed geometry bindings.
pub fn hide(state: &UiState) {
    if !state.native_child.enabled {
        return;
    }
    state.native_child.awaiting_layout.set(true);
    #[cfg(target_os = "macos")]
    if let Some(presenter) = state.native_child.presenter.borrow_mut().as_mut()
        && let Err(error) = presenter.hide()
    {
        eprintln!("native child hide failed: {error}");
    }
}

#[cfg(target_os = "macos")]
fn fail(app: &App, state: &UiState, error: String) {
    hide(state);
    let _ = state.player.stop();
    app.set_loaded(false);
    app.set_status(error.into());
}

/// Only the real Slint render/layout boundary releases the input/geometry fence.
pub fn before_render(app: &App, state: &UiState) {
    if !state.native_child.enabled {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        use oxplay_media::{NativeChildGeometry, NativeRect};
        // These getters can evaluate layout/change callbacks. Finish them all
        // before acquiring the RefCell: a callback may synchronously call hide.
        let video = NativeRect {
            x: app.get_video_window_x() as f64,
            y: app.get_video_window_y() as f64,
            width: app.get_video_width() as f64,
            height: app.get_video_height() as f64,
        };
        let clip = NativeRect {
            x: app.get_native_clip_x() as f64,
            y: app.get_native_clip_y() as f64,
            width: app.get_native_clip_width() as f64,
            height: app.get_native_clip_height() as f64,
        };
        let intersects = video.width > 0.
            && video.height > 0.
            && clip.width > 0.
            && clip.height > 0.
            && video.x < clip.x + clip.width
            && clip.x < video.x + video.width
            && video.y < clip.y + clip.height
            && clip.y < video.y + video.height;
        // Finish lazy UI evaluation before inspecting Slint's popup stack.
        let visible =
            intersects && app.get_native_child_visible() && app.get_loaded() && !state.hidden.get();
        let popup_clear = popups::is_clear(app.window());
        let geometry = NativeChildGeometry {
            video,
            clip,
            visible: visible && popup_clear,
        };
        let result = {
            let mut presenter = state.native_child.presenter.borrow_mut();
            presenter
                .as_mut()
                .map(|presenter| presenter.set_geometry(geometry))
        };
        if let Some(Err(error)) = result {
            fail(app, state, error.to_string());
            return;
        }
        // A media wake alone must not reveal after a popup closes: first
        // observe a popup-free layout boundary, then freshly render the child.
        state.native_child.awaiting_layout.set(!popup_clear);
        render(app, state);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Called by media notifications, independently of Slint window redraws. No
/// per-frame UI property/model assignment or borrowed texture is published.
pub fn render(app: &App, state: &UiState) {
    if !state.native_child.enabled {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let visible = app.get_native_child_visible() && app.get_loaded() && !state.hidden.get();
        // No Slint property dependency is installed by this direct RefCell
        // read. The adapter drops its borrow before we touch the presenter.
        let popup_clear = popups::is_clear(app.window());
        if !popup_clear {
            state.native_child.awaiting_layout.set(true);
        }
        let allowed = visible && popup_clear && !state.native_child.awaiting_layout.get();
        let result = {
            let mut presenter = state.native_child.presenter.borrow_mut();
            presenter
                .as_mut()
                .map(|presenter| presenter.render(allowed))
        };
        if let Some(Err(error)) = result {
            fail(app, state, error.to_string());
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

pub fn teardown(state: &UiState) {
    if !state.native_child.enabled {
        return;
    }
    hide(state);
    #[cfg(target_os = "macos")]
    let presenter = state.native_child.presenter.borrow_mut().take();
    #[cfg(target_os = "macos")]
    if let Some(presenter) = presenter {
        eprintln!("native child stats: {:?}", presenter.stats());
        drop(presenter);
    }
}

#[cfg(target_os = "macos")]
pub fn statistics(state: &UiState) -> Option<oxplay_media::NativeChildStats> {
    state
        .native_child
        .presenter
        .borrow()
        .as_ref()
        .map(|presenter| presenter.stats())
}

fn request_layout(app: &App) {
    // The pinned macOS backend's deferred Slint request path previously stalled
    // with a pending draw. This is one explicit input/overlay recovery request;
    // media frame notifications never call it.
    app.window()
        .with_winit_window(|window| window.request_redraw());
}

pub fn window_event(app: &App, state: &UiState, event: &winit::event::WindowEvent) {
    use winit::event::{ElementState, WindowEvent};
    if !state.native_child.enabled {
        return;
    }
    if matches!(
        event,
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } | WindowEvent::MouseWheel { .. }
            | WindowEvent::Resized(_)
            | WindowEvent::Moved(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Focused(_)
            | WindowEvent::Occluded(_)
    ) || matches!(event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed)
    {
        hide(state);
        // One redraw per explicit input/geometry event, not per media frame.
        request_layout(app);
    }
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    if !state.native_child.enabled {
        return;
    }
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_native_child_hide(move || {
        let Some(state) = state_weak.upgrade() else {
            return;
        };
        hide(&state);
        if let Some(app) = weak.upgrade() {
            request_layout(&app);
        }
    });
    // Installed after ordinary bindings. UI disabling alone is not admission
    // control: accessibility/programmatic callbacks must also stay offline.
    macro_rules! block {
        ($callback:ident, ($($argument:ident),*)) => {{
            let weak = app.as_weak();
            app.$callback(move |$($argument),*| {
                $(let _ = $argument;)*
                if let Some(app) = weak.upgrade() {
                    app.set_status("Native-child diagnostic: only the explicitly selected local file is available; online and account actions are disabled".into());
                }
            });
        }};
    }
    block!(on_search, (query));
    block!(on_select_video, (index));
    block!(on_guest_back, ());
    block!(on_more, ());
    block!(on_guest_previous, ());
    block!(on_search_kind_changed, (index));
    block!(on_channel_tab_changed, (index));
    block!(on_follow_guest_channel, ());
    block!(on_account_import, ());
    block!(on_account_import_path, (path, remember));
    block!(on_account_reconnect, ());
    block!(on_account_disconnect, ());
    block!(on_account_tab, (tab));
    block!(on_account_open, (index));
    block!(on_account_action, (index));
    block!(on_account_play, (index));
    block!(on_account_next, ());
    block!(on_account_reconcile, ());
    block!(on_account_rating, (like));
    block!(on_open_youtube, ());
    block!(on_open_export_guide, ());
    app.set_status(
        "DIAGNOSTIC — local native child video; layering, timing and performance are unqualified"
            .into(),
    );
}
