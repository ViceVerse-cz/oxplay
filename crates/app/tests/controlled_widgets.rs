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

fn search_text(app: &App) -> String {
    ElementHandle::find_by_accessible_label(app, "Search YouTube")
        .find(|element| {
            element.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::TextInput)
        })
        .and_then(|element| element.accessible_value())
        .expect("search editor")
        .to_string()
}

#[test]
fn search_suggestions_open_below_the_field_with_keyboard_preview_removal_and_click() {
    let app = app();
    app.window().set_size(slint::LogicalSize::new(1440., 900.));
    let ui = app.global::<SearchSuggestionsUi>();
    let queries = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let output = queries.clone();
    ui.on_query(move |text| output.borrow_mut().push(text.into()));
    let removed = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let output = removed.clone();
    ui.on_remove(move |text| output.borrow_mut().push(text.into()));
    let closes = Rc::new(Cell::new(0));
    let output = closes.clone();
    ui.on_closed(move || output.set(output.get() + 1));
    let searched = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let output = searched.clone();
    app.on_search(move |text| output.borrow_mut().push(text.into()));
    ui.set_rows(
        Rc::new(slint::VecModel::from(vec![
            SearchSuggestion {
                text: "TEST FIXTURE book".into(),
                prefix: "TEST FIXTURE book".into(),
                rest: "".into(),
                history: true,
            },
            SearchSuggestion {
                text: "TEST FIXTURE lang".into(),
                prefix: "".into(),
                rest: "TEST FIXTURE lang".into(),
                history: false,
            },
        ]))
        .into(),
    );
    settle();
    // The testing query skips invisible elements, so a missing panel is closed.
    let panel = || ElementHandle::find_by_element_id(&app, "App::suggestions").next();
    assert!(panel().is_none(), "Closed until the field is focused");
    app.invoke_focus_search();
    settle();
    assert!(app.get_search_active());
    assert_eq!(
        *queries.borrow(),
        [""],
        "Focus asks for current suggestions"
    );
    let search = ElementHandle::find_by_element_id(&app, "App::search")
        .next()
        .unwrap();
    let panel = || {
        ElementHandle::find_by_element_id(&app, "App::suggestions")
            .next()
            .filter(|panel| panel.size().height > 0.)
    };
    let open = panel().expect("open below the focused field");
    assert_eq!(open.size().height, 2. * 36. + 16.);
    assert_eq!(open.size().width, search.size().width);
    assert_eq!(open.absolute_position().x, search.absolute_position().x);
    assert_eq!(
        open.absolute_position().y,
        search.absolute_position().y + search.size().height + 4.
    );
    // Arrow keys preview rows in the field and wrap back to the typed text.
    key(&app, Key::DownArrow);
    assert_eq!(search_text(&app), "TEST FIXTURE book");
    key(&app, Key::DownArrow);
    assert_eq!(search_text(&app), "TEST FIXTURE lang");
    key(&app, Key::DownArrow);
    assert_eq!(search_text(&app), "");
    key(&app, Key::UpArrow);
    assert_eq!(search_text(&app), "TEST FIXTURE lang");
    assert_eq!(queries.borrow().len(), 1, "Previews are not edits");
    // Escape closes only the panel; focus stays in the editor.
    key(&app, Key::Escape);
    settle();
    assert!(panel().is_none());
    assert!(app.get_search_active());
    assert_eq!(closes.get(), 1);
    key(&app, Key::DownArrow);
    settle();
    assert!(panel().is_some(), "Down reopens the panel");
    element(&app, "Remove TEST FIXTURE book from search history")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(*removed.borrow(), ["TEST FIXTURE book"]);
    assert!(searched.borrow().is_empty());
    assert!(app.get_search_active(), "Removal keeps focus in the editor");
    element(&app, "Search suggestion: TEST FIXTURE lang")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(*searched.borrow(), ["TEST FIXTURE lang"]);
    assert_eq!(search_text(&app), "TEST FIXTURE lang");
    assert!(panel().is_none(), "Submission closes the panel");
    // Blur closes it as well, even without a dismissal.
    app.invoke_focus_browse();
    settle();
    assert!(!app.get_search_active());
    ui.set_dismissed(false);
    settle();
    assert!(panel().is_none());
    assert_eq!(closes.get(), 3);
}

#[test]
fn youtube_suggestion_setting_uses_acknowledgement() {
    let app = app();
    // Tall enough that the privacy section is inside the Settings viewport.
    app.window().set_size(slint::LogicalSize::new(1000., 2000.));
    app.set_page(3);
    let ui = app.global::<SearchSuggestionsUi>();
    let attempted = Rc::new(Cell::new(None));
    let output = attempted.clone();
    ui.on_set_youtube_enabled(move |enabled| output.set(Some(enabled)));
    settle();
    let label = "Show search suggestions from YouTube";
    assert_eq!(element(&app, label).accessible_checked(), Some(true));
    element(&app, label).mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(attempted.get(), Some(false));
    assert!(ui.get_youtube_enabled(), "Unacknowledged setting stays on");
    ui.set_youtube_enabled(false);
    settle();
    assert_eq!(element(&app, label).accessible_checked(), Some(false));
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

fn controls(app: &App, label: &str) -> usize {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|element| element.accessible_enabled().is_some())
        .count()
}
fn absent(app: &App, label: &str) -> bool {
    controls(app, label) == 0
}

#[test]
fn home_source_chips_exist_only_for_a_connected_account_and_follow_rust_acknowledgement() {
    let app = app();
    app.set_page(0);
    app.set_home_active(true);
    app.set_account_connected(false);
    let requested = Rc::new(Cell::new(None));
    let output = requested.clone();
    app.on_home_source_changed(move |index| output.set(Some(index)));
    settle();
    // Guests keep the unchanged local Home: no account chips.
    assert!(absent(&app, "Recommended") && absent(&app, "Saved"));
    // The Home toolbar shortcut is in addition to any navigation entry.
    let saved_shortcuts = controls(&app, "Local playlists");
    assert!(saved_shortcuts >= 1);
    app.set_account_connected(true);
    app.set_home_source(1);
    settle();
    assert_eq!(
        element(&app, "Recommended").accessible_checked(),
        Some(true)
    );
    assert_eq!(element(&app, "Saved").accessible_checked(), Some(false));
    assert_eq!(
        controls(&app, "Local playlists"),
        saved_shortcuts - 1,
        "the local shortcut belongs to the Saved view"
    );
    element(&app, "Saved").invoke_accessible_default_action();
    settle();
    assert_eq!(requested.take(), Some(0));
    // The chip reflects only the source Rust acknowledged.
    assert_eq!(
        element(&app, "Recommended").accessible_checked(),
        Some(true)
    );
    app.set_home_source(0);
    settle();
    assert_eq!(element(&app, "Saved").accessible_checked(), Some(true));
    assert_eq!(controls(&app, "Local playlists"), saved_shortcuts);
    element(&app, "Recommended").invoke_accessible_default_action();
    settle();
    assert_eq!(requested.take(), Some(1));
    // Another account operation or busy playback resolution blocks switching.
    app.set_account_busy(true);
    settle();
    assert_eq!(
        element(&app, "Recommended").accessible_enabled(),
        Some(false)
    );
    element(&app, "Recommended").invoke_accessible_default_action();
    app.set_account_busy(false);
    app.set_busy(true);
    settle();
    element(&app, "Saved").invoke_accessible_default_action();
    settle();
    assert_eq!(requested.take(), None);
    app.set_busy(false);
    app.set_home_active(false);
    settle();
    assert!(absent(&app, "Recommended"), "chips are Home-only");
}

#[test]
fn close_player_exists_only_in_the_mini_player_and_requests_a_stop() {
    let app = app();
    let closes = Rc::new(Cell::new(0));
    let output = closes.clone();
    app.on_close_player(move || output.set(output.get() + 1));
    app.set_loaded(true);
    app.set_controls_visible(true);
    app.set_page(2);
    settle();
    assert!(
        ElementHandle::find_by_accessible_label(&app, "Close player")
            .all(|element| element.accessible_enabled().is_none()),
        "The full watch page has no close control"
    );
    app.set_page(0);
    settle();
    assert!(app.get_mini_player_active());
    element(&app, "Close player").mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(closes.get(), 1);
}

#[test]
fn question_mark_opens_the_shortcut_sheet_and_number_keys_seek_by_tenths() {
    let app = app();
    app.set_page(2);
    app.set_loaded(true);
    app.set_duration(200.);
    let seeks = Rc::new(std::cell::RefCell::new(Vec::new()));
    let output = seeks.clone();
    app.on_seek(move |value| output.borrow_mut().push(value));
    app.invoke_focus_browse();
    settle();
    key_text(&app, "3");
    settle();
    assert_eq!(seeks.borrow().as_slice(), &[60.]);
    key_text(&app, "?");
    settle();
    element(&app, "Close keyboard shortcuts")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert!(
        ElementHandle::find_by_accessible_label(&app, "Close keyboard shortcuts")
            .all(|element| element.accessible_enabled().is_none())
    );
}
fn key_text(app: &App, text: &str) {
    let text: SharedString = text.into();
    app.window()
        .dispatch_event_with_result(WindowEvent::KeyPressed { text: text.clone() })
        .unwrap();
    app.window()
        .dispatch_event_with_result(WindowEvent::KeyReleased { text })
        .unwrap();
}

#[test]
fn paginated_grid_keeps_end_and_rapid_navigation_inside_the_thumbnail_viewport() {
    let app = app();
    app.set_page(0);
    app.set_has_more(true);
    let columns = app.get_columns().max(1) as usize;
    let rows: Vec<_> = (0..100)
        .map(|index| VideoRow {
            kind: "Video".into(),
            title: format!("Card {index}").into(),
            id: index.to_string().into(),
            ..Default::default()
        })
        .collect();
    let groups: Vec<_> = rows
        .chunks(columns)
        .enumerate()
        .map(|(index, rows)| VideoGroup {
            start: (index * columns) as i32,
            items: Rc::new(slint::VecModel::from(rows.to_vec())).into(),
        })
        .collect();
    app.set_videos(Rc::new(slint::VecModel::from(rows)).into());
    app.set_groups(Rc::new(slint::VecModel::from(groups)).into());
    for target in [99, 98, 99, 0, 99] {
        app.invoke_reveal_feed_item(target);
        settle();
        // Traverse the real compiled card trees so virtualized layout runs.
        let card = element(&app, &format!("Video, Card {target}, "));
        assert!(card.accessible_enabled().unwrap());
        assert!(app.get_feed_thumbnail_first() <= target);
        assert!(app.get_feed_thumbnail_end() > target);
        assert!(app.get_feed_thumbnail_end() - app.get_feed_thumbnail_first() < 30);
        let offscreen = if target == 0 { 99 } else { 0 };
        assert!(
            ElementHandle::find_by_accessible_label(&app, &format!("Video, Card {offscreen}, "))
                .all(|element| element.accessible_enabled().is_none())
        );
    }
}

#[test]
fn clipped_transport_background_does_not_keep_offscreen_clock_controls_active() {
    let app = app();
    app.window().set_size(slint::LogicalSize::new(1000., 600.));
    app.set_page(2);
    app.set_loaded(true);
    app.set_controls_visible(true);
    app.set_watch_videos(Rc::new(slint::VecModel::from(vec![VideoRow::default(); 30])).into());
    settle();
    assert!(app.get_progress_visible());
    app.window().dispatch_event(WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(
            app.get_video_window_x() + 50.,
            app.get_video_window_y() + 50.,
        ),
        delta_x: 0.,
        delta_y: -800.,
    });
    mock_elapsed_time(Duration::from_millis(500));
    settle();
    assert!(app.get_visible_related_count() > 0);
    assert!(app.get_watch_offset() < 0.);
    assert!(app.get_video_window_y() + app.get_video_height() > 0.);
    assert!(!app.get_progress_visible());
    app.invoke_focus_player();
    settle();
    assert!(app.get_progress_visible());
}

fn modified_key(app: &App, modifiers: &[Key], text: &str) {
    for modifier in modifiers {
        let modifier: SharedString = (*modifier).into();
        app.window()
            .dispatch_event(WindowEvent::KeyPressed { text: modifier });
    }
    key_text(app, text);
    for modifier in modifiers.iter().rev() {
        let modifier: SharedString = (*modifier).into();
        app.window()
            .dispatch_event(WindowEvent::KeyReleased { text: modifier });
    }
}

#[test]
fn watch_tab_strip_routes_switch_close_background_open_and_shortcuts() {
    let app = app();
    app.set_page(2);
    app.set_loaded(true);
    let ui = app.global::<TabsUi>();
    let events = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let output = events.clone();
    ui.on_activate(move |index| output.borrow_mut().push(format!("activate {index}")));
    let output = events.clone();
    ui.on_close(move |index| output.borrow_mut().push(format!("close {index}")));
    let output = events.clone();
    ui.on_cycle(move |forward| output.borrow_mut().push(format!("cycle {forward}")));
    let output = events.clone();
    ui.on_close_active(move || output.borrow_mut().push("close-active".into()));
    let output = events.clone();
    ui.on_open(move |surface, index| output.borrow_mut().push(format!("open {surface} {index}")));
    ui.set_tabs(
        Rc::new(slint::VecModel::from(vec![
            WatchTab {
                title: "TEST FIXTURE first tab".into(),
                channel: "TEST FIXTURE creator".into(),
                active: true,
            },
            WatchTab {
                title: "TEST FIXTURE second tab".into(),
                ..WatchTab::default()
            },
        ]))
        .into(),
    );
    ui.set_active_index(0);
    settle();
    // Rust decides visibility; a hidden strip reserves no header space.
    assert!(absent(&app, "TEST FIXTURE second tab"));
    let video_y = app.get_video_window_y();
    ui.set_strip_visible(true);
    settle();
    assert_eq!(app.get_native_header_height(), 48.);
    assert!((app.get_video_window_y() - video_y - 36.).abs() < 1.);
    let first = element(&app, "TEST FIXTURE first tab, TEST FIXTURE creator");
    assert_eq!(first.accessible_item_selected(), Some(true));
    element(&app, "TEST FIXTURE second tab")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    element(&app, "Close tab: TEST FIXTURE second tab")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    first.mock_single_click(slint::platform::PointerEventButton::Middle);
    settle();
    assert_eq!(
        events.take(),
        ["activate 1", "close 1", "close 0"].map(String::from)
    );

    // Ctrl+Tab / Ctrl+Shift+Tab cycle and Cmd/Ctrl+W closes the active tab,
    // even while an editor has focus.
    app.invoke_focus_browse();
    modified_key(&app, &[Key::Control], "\t");
    modified_key(&app, &[Key::Control, Key::Shift], "\t");
    app.invoke_focus_search();
    modified_key(&app, &[Key::Control], "w");
    settle();
    assert_eq!(
        events.take(),
        ["cycle true", "cycle false", "close-active"].map(String::from)
    );
    // Fullscreen/PiP hide the strip, and its shortcuts stay inactive there.
    app.set_fullscreen_active(true);
    settle();
    assert!(absent(&app, "TEST FIXTURE second tab"));
    app.invoke_focus_browse();
    modified_key(&app, &[Key::Control], "w");
    app.set_fullscreen_active(false);

    // Cards open videos in a background tab by middle click or Ctrl/Cmd+click;
    // a plain click still selects in the current tab.
    app.set_page(0);
    app.set_home_active(false);
    let rows = vec![
        VideoRow {
            kind: "Video".into(),
            title: "TEST FIXTURE card".into(),
            id: "fixtureCard".into(),
            ..VideoRow::default()
        },
        VideoRow {
            kind: "Channel".into(),
            title: "TEST FIXTURE channel".into(),
            id: "fixtureChan".into(),
            ..VideoRow::default()
        },
    ];
    app.set_videos(Rc::new(slint::VecModel::from(rows.clone())).into());
    app.set_groups(
        Rc::new(slint::VecModel::from(vec![VideoGroup {
            start: 0,
            items: Rc::new(slint::VecModel::from(rows)).into(),
        }]))
        .into(),
    );
    let selected = Rc::new(Cell::new(0));
    let output = selected.clone();
    app.on_select_video(move |_| output.set(output.get() + 1));
    settle();
    let card = element(&app, "Video, TEST FIXTURE card, ");
    card.mock_single_click(slint::platform::PointerEventButton::Middle);
    app.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    card.mock_single_click(slint::platform::PointerEventButton::Left);
    app.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    settle();
    assert_eq!(selected.get(), 0);
    card.mock_single_click(slint::platform::PointerEventButton::Left);
    // Channels and playlists are not watch tabs.
    element(&app, "Channel, TEST FIXTURE channel, ")
        .mock_single_click(slint::platform::PointerEventButton::Middle);
    settle();
    assert_eq!(selected.get(), 1);
    assert_eq!(events.take(), ["open 0 0", "open 0 0"].map(String::from));
    // The same action is offered from the card's context menu.
    card.mock_single_click(slint::platform::PointerEventButton::Right);
    settle();
    ElementHandle::find_by_accessible_label(&app, "Open in new tab")
        .next()
        .expect("context menu item")
        .mock_single_click(slint::platform::PointerEventButton::Left);
    settle();
    assert_eq!(selected.get(), 1);
    assert_eq!(events.take(), ["open 0 0"].map(String::from));
}
