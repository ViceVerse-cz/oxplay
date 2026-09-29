// SPDX-License-Identifier: GPL-3.0-or-later
//! Real compiled shared components, synthetic acknowledgements and injected
//! Slint keys. No renderer, network, media engine or OS accessibility claim.
// Release deliberately omits Slint element introspection metadata.
#![cfg(debug_assertions)]
use i_slint_backend_testing::{ElementHandle, init_no_event_loop, mock_elapsed_time};
use slint::{
    ComponentHandle, Model, SharedString,
    platform::{Key, WindowEvent},
};
use std::{cell::Cell, rc::Rc, time::Duration};

slint::include_modules!();

fn settle() {
    for _ in 0..3 {
        mock_elapsed_time(Duration::from_millis(1));
        slint::platform::update_timers_and_animations();
    }
}
fn app() -> App {
    init_no_event_loop();
    let app = App::new().unwrap();
    app.window().set_size(slint::LogicalSize::new(1000., 800.));
    app.show().unwrap();
    settle();
    app
}
fn element(app: &App, label: &str) -> ElementHandle {
    // Text descendants may inherit the same label; target the actual control.
    let mut matches = ElementHandle::find_by_accessible_label(app, label)
        .filter(|element| element.accessible_enabled().is_some());
    let found = matches.next().unwrap_or_else(|| panic!("Missing {label}"));
    assert!(matches.next().is_none(), "Ambiguous {label}");
    found
}
fn key(app: &App, key: Key) {
    let text: SharedString = key.into();
    app.window()
        .dispatch_event_with_result(WindowEvent::KeyPressed { text: text.clone() })
        .unwrap();
    app.window()
        .dispatch_event_with_result(WindowEvent::KeyReleased { text })
        .unwrap();
}
fn down(app: &App, label: &str) {
    element(app, label).invoke_accessible_expand_action();
    settle();
    key(app, Key::DownArrow);
    key(app, Key::Escape);
    settle();
}
fn value(app: &App, label: &str, expected: &str) {
    settle();
    assert_eq!(
        element(app, label).accessible_value().as_deref(),
        Some(expected),
        "{label}"
    );
}

#[test]
fn search_and_channel_filters_keep_the_acknowledged_choice_after_rejection_and_navigation() {
    let app = app();
    app.set_page(0);
    app.set_home_active(false);
    app.set_guest_scope(0);
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    app.on_search_kind_changed(move |index| output.set(Some(index)));
    value(&app, "Search result type", "All results");
    down(&app, "Search result type");
    assert_eq!(attempted.get(), Some(1));
    assert_eq!(app.get_search_kind(), 0);
    value(&app, "Search result type", "All results");
    // Delayed admission/navigation update after the widget assigned its fields.
    app.set_search_kind(2);
    value(&app, "Search result type", "Channels");
    app.set_search_kind(0);
    value(&app, "Search result type", "All results");

    app.set_guest_scope(1);
    let output = attempted.clone();
    app.on_channel_tab_changed(move |index| output.set(Some(index)));
    value(&app, "Channel content type", "Videos");
    down(&app, "Channel content type");
    assert_eq!(attempted.get(), Some(1));
    assert_eq!(app.get_channel_tab(), 0);
    value(&app, "Channel content type", "Videos");
    app.set_channel_tab(3);
    value(&app, "Channel content type", "Playlists");
    app.set_channel_tab(0);
    value(&app, "Channel content type", "Videos");
}

#[test]
fn caption_choice_changes_only_after_acknowledgement_and_off_restores_the_label() {
    let app = app();
    let ui = app.global::<CaptionsUi>();
    ui.set_tracks(
        Rc::new(slint::VecModel::from(vec![
            "Off".into(),
            "TEST FIXTURE English".into(),
            "TEST FIXTURE Czech".into(),
        ]))
        .into(),
    );
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    ui.on_select(move |index| output.set(Some(index)));
    app.invoke_show_captions();
    value(&app, "Subtitle or caption track", "Off");
    down(&app, "Subtitle or caption track");
    assert_eq!(attempted.get(), Some(1));
    assert_eq!(ui.get_selected(), 0);
    value(&app, "Subtitle or caption track", "Off");
    ui.set_selected(1);
    value(&app, "Subtitle or caption track", "TEST FIXTURE English");
    element(&app, "Turn captions off / cancel").invoke_accessible_default_action();
    assert_eq!(attempted.get(), Some(0));
    value(&app, "Subtitle or caption track", "TEST FIXTURE English");
    ui.set_selected(0);
    value(&app, "Subtitle or caption track", "Off");
    let replacement = Rc::new(slint::VecModel::from(vec![
        "Off".into(),
        "TEST FIXTURE replacement".into(),
    ]));
    ui.set_tracks(replacement.clone().into());
    ui.set_selected(1);
    value(
        &app,
        "Subtitle or caption track",
        "TEST FIXTURE replacement",
    );
    // Incremental metadata can rename a row without changing the numeric index.
    replacement.set_row_data(1, "TEST FIXTURE updated label".into());
    value(
        &app,
        "Subtitle or caption track",
        "TEST FIXTURE updated label",
    );
}

#[test]
fn appearance_rejects_unadmitted_changes_and_tracks_later_rollback() {
    let app = app();
    app.set_page(3);
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    app.on_theme_changed(move |index| output.set(Some(index)));
    value(&app, "Appearance", "Follow system");
    down(&app, "Appearance");
    assert_eq!(attempted.get(), Some(1));
    assert_eq!(app.get_theme(), 0);
    value(&app, "Appearance", "Follow system");
    let weak = app.as_weak();
    app.on_theme_changed(move |index| weak.upgrade().unwrap().set_theme(index));
    down(&app, "Appearance");
    assert_eq!(app.get_theme(), 1);
    value(&app, "Appearance", "Light");
    // Asynchronous persistence failure and reset remain authoritative after edits.
    app.set_theme(0);
    value(&app, "Appearance", "Follow system");
    app.set_theme(2);
    value(&app, "Appearance", "Dark");
}

#[test]
fn quality_does_not_change_the_acknowledged_stream_on_callback_rejection() {
    let app = app();
    app.set_page(2);
    app.set_remote_video(true);
    app.set_loaded(true);
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    app.on_quality(move |index| output.set(Some(index)));
    settle();
    element(&app, "Playback settings").invoke_accessible_default_action();
    value(&app, "Maximum video quality", "Up to 1080p");
    down(&app, "Maximum video quality");
    assert_eq!(attempted.get(), Some(1));
    assert_eq!(app.get_quality_index(), 0);
    value(&app, "Maximum video quality", "Up to 1080p");
    app.set_quality_index(1);
    value(&app, "Maximum video quality", "Up to 720p");
    app.set_quality_index(0);
    value(&app, "Maximum video quality", "Up to 1080p");
}

#[test]
fn local_save_destination_is_a_dialog_draft_and_resets_for_a_new_dialog() {
    let app = app();
    let ui = app.global::<SaveUi>();
    ui.set_collections(
        Rc::new(slint::VecModel::from(vec![
            "TEST FIXTURE first".into(),
            "TEST FIXTURE second".into(),
        ]))
        .into(),
    );
    ui.on_begin(|| true);
    let saved = Rc::new(Cell::new(None));
    let output = saved.clone();
    app.on_save_to_playlist(move |index| output.set(Some(index)));
    assert!(app.invoke_show_local_save());
    value(&app, "Local playlist to save into", "TEST FIXTURE first");
    down(&app, "Local playlist to save into");
    value(&app, "Local playlist to save into", "TEST FIXTURE second");
    assert_eq!(saved.get(), None, "Selection must not submit a save");
    element(&app, "Save").invoke_accessible_default_action();
    assert_eq!(saved.get(), Some(1));
    // Closing and reopening a pending save keeps the frozen destination.
    ui.set_busy(true);
    app.invoke_close_local_save();
    assert!(app.invoke_show_local_save());
    value(&app, "Local playlist to save into", "TEST FIXTURE second");
    assert_eq!(
        element(&app, "Local playlist to save into").accessible_enabled(),
        Some(false)
    );
    app.invoke_close_local_save();
    ui.set_busy(false);
    assert!(app.invoke_show_local_save());
    value(&app, "Local playlist to save into", "TEST FIXTURE first");
}
