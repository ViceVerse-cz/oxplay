// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite opt-in geometry observations; no polling, model writes or pixel reads.
use crate::{App, UiState};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};

pub fn record(seconds: u64, app: &App, state: &UiState) {
    let Some((width, height, scale)) = app.window().with_winit_window(|window| {
        let size = window.inner_size();
        (size.width, size.height, window.scale_factor())
    }) else {
        eprintln!("presentation geometry seconds={seconds}: unavailable");
        return;
    };
    // Generated getters are read outside any presenter borrow. They may lazily
    // evaluate Slint bindings, just as the normal geometry handoff does.
    eprintln!(
        "presentation geometry seconds={seconds}: window_px={width}x{height} scale={scale} video_logical=({},{},{},{}) clip_logical=({},{},{},{}) page={} fullscreen={} hidden={} native_child={} geometry_events={}",
        app.get_video_window_x(),
        app.get_video_window_y(),
        app.get_video_width(),
        app.get_video_height(),
        app.get_native_clip_x(),
        app.get_native_clip_y(),
        app.get_native_clip_width(),
        app.get_native_clip_height(),
        app.get_page(),
        app.get_fullscreen_active(),
        state.hidden.get(),
        app.get_native_video_child(),
        state
            .geometry_events
            .as_ref()
            .map_or(0, std::cell::Cell::get),
    );
    #[cfg(target_os = "macos")]
    if let Some(stats) = crate::native_child::statistics(state) {
        eprintln!(
            "native geometry seconds={seconds}: backing_px={}x{} geometry_changes={} visible={} renders={} publications={}",
            stats.backing_width,
            stats.backing_height,
            stats.geometry_changes,
            stats.visible,
            stats.renders,
            stats.publications,
        );
    }
}
