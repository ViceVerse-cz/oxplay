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
fn settings_quality_pointer_selection_stays_open_until_acknowledgement() {
    let app = app();
    app.set_page(2);
    app.set_remote_video(true);
    app.set_loaded(true);
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    app.on_quality(move |index| output.set(Some(index)));
    settle();
    element(&app, "Playback settings").invoke_accessible_default_action();
    settle();
    element(&app, "Maximum video quality: Up to 1080p")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    element(&app, "Maximum quality Up to 720p")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(attempted.get(), Some(1));
    assert_eq!(app.get_quality_index(), 0);
    assert_eq!(
        element(&app, "Maximum quality Up to 1080p").accessible_item_selected(),
        Some(true)
    );
    assert_eq!(
        element(&app, "Maximum quality Up to 720p").accessible_item_selected(),
        Some(false)
    );
    app.set_busy(true);
    settle();
    assert_eq!(
        element(&app, "Maximum quality Up to 480p").accessible_enabled(),
        Some(false)
    );
    app.set_quality_index(1);
    app.set_busy(false);
    settle();
    assert_eq!(
        element(&app, "Maximum quality Up to 720p").accessible_item_selected(),
        Some(true)
    );
    app.set_quality_index(0);
    settle();
    assert_eq!(
        element(&app, "Maximum quality Up to 1080p").accessible_item_selected(),
        Some(true)
    );
    key(&app, Key::Escape);
    settle();
    // Escape returns from the inline submenu; it does not dismiss its parent.
    element(&app, "Maximum video quality: Up to 1080p");
    key(&app, Key::Escape);
    settle();
    assert!(
        ElementHandle::find_by_accessible_label(&app, "Close playback settings")
            .next()
            .is_none()
    );
    element(&app, "Playback settings").invoke_accessible_default_action();
    settle();
    element(&app, "Maximum video quality: Up to 1080p");
}

#[test]
fn settings_speed_pointer_selection_keeps_authoritative_state_and_keyboard_back() {
    let app = app();
    app.set_page(2);
    app.set_remote_video(true);
    app.set_loaded(true);
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    app.on_speed(move |speed| output.set(Some(speed)));
    settle();
    element(&app, "Playback settings").invoke_accessible_default_action();
    settle();
    element(&app, "Playback speed: Normal")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    element(&app, "Playback speed 1.5×")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(attempted.get(), Some(1.5));
    assert_eq!(app.get_speed_index(), 1);
    assert_eq!(
        element(&app, "Playback speed Normal").accessible_item_selected(),
        Some(true)
    );
    app.set_speed_busy(true);
    settle();
    assert_eq!(
        element(&app, "Playback speed 2×").accessible_enabled(),
        Some(false)
    );
    app.set_speed_index(2);
    app.set_speed_busy(false);
    settle();
    assert_eq!(
        element(&app, "Playback speed 1.5×").accessible_item_selected(),
        Some(true)
    );
    key(&app, Key::Escape);
    settle();
    element(&app, "Playback speed: 1.5×");
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

#[test]
fn navigation_keeps_the_same_watch_data_and_places_the_mini_player_above_browsing() {
    let app = app();
    app.set_page(2);
    app.set_loaded(true);
    app.set_remote_video(true);
    app.set_video_title("TEST FIXTURE retained title".into());
    app.set_video_channel("TEST FIXTURE retained creator".into());
    app.global::<CommentsUi>()
        .set_description("TEST FIXTURE retained description".into());
    let related = Rc::new(slint::VecModel::from(vec![VideoRow {
        id: "fixtureWatch".into(),
        title: "TEST FIXTURE retained related video".into(),
        ..VideoRow::default()
    }]));
    app.set_watch_videos(related.clone().into());
    settle();
    assert!(!app.get_mini_player_active());
    let large_width = app.get_video_width();
    for page in [0, 1, 3, 4] {
        app.set_page(page);
        app.set_videos(
            Rc::new(slint::VecModel::from(vec![VideoRow {
                id: "fixtureBrowse".into(),
                title: "TEST FIXTURE different browsing page".into(),
                ..VideoRow::default()
            }]))
            .into(),
        );
        settle();
        assert!(app.get_mini_player_active());
        assert!(app.get_video_visible());
        assert!(app.get_progress_visible());
        assert!((app.get_video_width() - 380.).abs() < 1.);
        assert!((app.get_video_height() - 213.75).abs() < 1.);
        assert!((app.get_video_window_x() + app.get_video_width() - 980.).abs() < 1.);
        assert!((app.get_video_window_y() + app.get_video_height() - 780.).abs() < 1.);
        assert_eq!(app.get_watch_offset(), 0.);
        assert_eq!(app.get_video_channel(), "TEST FIXTURE retained creator");
        assert_eq!(
            app.get_watch_videos().row_data(0).unwrap().id,
            "fixtureWatch"
        );
    }
    app.set_page(2);
    settle();
    assert!(!app.get_mini_player_active());
    assert_eq!(app.get_video_width(), large_width);
    assert_eq!(app.get_video_title(), "TEST FIXTURE retained title");
    assert_eq!(
        app.global::<CommentsUi>().get_description(),
        "TEST FIXTURE retained description"
    );
    assert_eq!(related.row_count(), 1);
}

#[test]
fn video_background_and_transport_click_each_activate_pause_once_in_watch_and_mini() {
    let app = app();
    app.set_page(2);
    app.set_loaded(true);
    app.set_paused(false);
    let pauses = Rc::new(Cell::new(0));
    let output = pauses.clone();
    app.on_toggle_pause(move || output.set(output.get() + 1));
    for page in [2, 0, 3] {
        app.set_page(page);
        settle();
        ElementHandle::find_by_element_id(&app, "App::video-touch")
            .next()
            .unwrap()
            .mock_single_click(slint::platform::PointerEventButton::Left);
        settle();
        let after_background = pauses.get();
        assert!(after_background > 0);
        element(&app, "Pause").mock_single_click(slint::platform::PointerEventButton::Left);
        settle();
        assert_eq!(pauses.get(), after_background + 1);
    }
    assert_eq!(pauses.get(), 6);
}

#[test]
fn pip_video_drag_threshold_does_not_pause_or_capture_transport_buttons() {
    let app = app();
    app.set_page(2);
    app.set_loaded(true);
    app.set_paused(false);
    app.set_picture_in_picture(true);
    app.window().set_size(slint::LogicalSize::new(480., 270.));
    let drags = Rc::new(Cell::new(0));
    let output = drags.clone();
    app.on_pip_drag(move || output.set(output.get() + 1));
    let pauses = Rc::new(Cell::new(0));
    let output = pauses.clone();
    app.on_toggle_pause(move || output.set(output.get() + 1));
    settle();
    let origin = slint::LogicalPosition::new(240., 70.);
    let moved = slint::LogicalPosition::new(250., 70.);
    for event in [
        WindowEvent::PointerMoved { position: origin },
        WindowEvent::PointerPressed {
            position: origin,
            button: slint::platform::PointerEventButton::Left,
        },
        WindowEvent::PointerMoved { position: moved },
        WindowEvent::PointerReleased {
            position: moved,
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        app.window().dispatch_event(event);
    }
    settle();
    assert_eq!(drags.get(), 1);
    assert_eq!(pauses.get(), 0);
    element(&app, "Pause").mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(pauses.get(), 1);
    assert_eq!(drags.get(), 1);
}

#[test]
fn creator_avatar_and_name_share_one_keyboard_accessible_channel_action() {
    let app = app();
    app.set_page(2);
    app.set_loaded(true);
    app.set_remote_video(true);
    app.set_video_channel("TEST FIXTURE creator".into());
    app.set_watch_channel_available(true);
    let opens = Rc::new(Cell::new(0));
    let output = opens.clone();
    app.on_open_watch_channel(move || output.set(output.get() + 1));
    settle();
    let channel = element(&app, "Open TEST FIXTURE creator channel");
    channel.mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(opens.get(), 1);
    // Hit the image itself, rather than the middle of the creator action.
    let position = channel.absolute_position();
    let avatar_center = slint::LogicalPosition::new(position.x + 22., position.y + 22.);
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: avatar_center,
    });
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: avatar_center,
        button: slint::platform::PointerEventButton::Left,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: avatar_center,
        button: slint::platform::PointerEventButton::Left,
    });
    settle();
    assert_eq!(opens.get(), 2);
    app.set_watch_channel_available(false);
    settle();
    element(&app, "Open TEST FIXTURE creator channel")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(opens.get(), 2, "Unavailable creator must not navigate");
}

#[test]
fn shared_header_centers_controls_and_bounds_the_creator_hit_region() {
    let app = app();
    app.set_native_header_integrated(true);
    app.window().set_size(slint::LogicalSize::new(1440., 900.));
    settle();
    for id in ["header-menu", "header-account", "search"] {
        let control = ElementHandle::find_by_element_id(&app, &format!("App::{id}"))
            .next()
            .unwrap();
        let center = control.absolute_position().y + control.size().height / 2.;
        assert!(
            (center - app.get_native_header_height() / 2.).abs() < 0.1,
            "{id}: {center}"
        );
    }
    let search = ElementHandle::find_by_element_id(&app, "App::search")
        .next()
        .unwrap();
    assert!((search.absolute_position().x + search.size().width / 2. - 720.).abs() < 0.1);
    app.set_page(2);
    app.set_loaded(true);
    app.set_remote_video(true);
    app.set_video_channel("TEST FIXTURE creator".into());
    app.set_watch_channel_available(true);
    app.set_watch_channel_subscribers("1.2M subscribers".into());
    settle();
    let creator = element(&app, "Open TEST FIXTURE creator channel");
    assert!(
        creator.size().width < 320.,
        "Creator hover must fit its text, not the entire row"
    );
    let share = element(&app, "Share");
    assert!(creator.absolute_position().x + creator.size().width < share.absolute_position().x);
    assert!(
        (creator.absolute_position().y + creator.size().height / 2.
            - share.absolute_position().y
            - share.size().height / 2.)
            .abs()
            < 0.1
    );
}

#[test]
fn comments_setting_uses_acknowledgement_and_supports_rollback() {
    let app = app();
    app.set_page(3);
    let comments = app.global::<CommentsUi>();
    comments.set_enabled(true);
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    comments.on_set_enabled(move |enabled| output.set(Some(enabled)));
    settle();
    let label = "Show comments and load the first page automatically";
    element(&app, label).mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(attempted.get(), Some(false));
    assert!(
        comments.get_enabled(),
        "Unacknowledged setting stays authoritative"
    );
    comments.set_enabled(false);
    settle();
    assert_eq!(element(&app, label).accessible_checked(), Some(false));
    comments.set_enabled(true);
    settle();
    assert_eq!(element(&app, label).accessible_checked(), Some(true));
}

#[test]
fn history_removal_does_not_play_the_video_and_artwork_click_does() {
    let app = app();
    app.set_page(1);
    let library = app.global::<LibraryUi>();
    library.set_tab(2);
    library.set_history_enabled(true);
    library.set_rows(
        Rc::new(slint::VecModel::from(vec![LibraryRow {
            title: "TEST FIXTURE history video".into(),
            channel: "TEST FIXTURE channel".into(),
            day_heading: "Today".into(),
            duration: "12:34".into(),
            progress: 0.5,
            ..LibraryRow::default()
        }]))
        .into(),
    );
    let plays = Rc::new(Cell::new(0));
    let removes = Rc::new(Cell::new(0));
    let output = plays.clone();
    library.on_open(move |_| output.set(output.get() + 1));
    let output = removes.clone();
    library.on_remove(move |_| output.set(output.get() + 1));
    settle();
    element(&app, "Remove TEST FIXTURE history video from local history")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(removes.get(), 1);
    assert_eq!(plays.get(), 0);
    let play = element(&app, "Play TEST FIXTURE history video");
    let origin = play.absolute_position();
    let point = slint::LogicalPosition::new(origin.x + 50., origin.y + 30.);
    for event in [
        WindowEvent::PointerMoved { position: point },
        WindowEvent::PointerPressed {
            position: point,
            button: slint::platform::PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: point,
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        app.window().dispatch_event(event);
    }
    settle();
    assert_eq!(plays.get(), 1);
    assert_eq!(removes.get(), 1);
}
