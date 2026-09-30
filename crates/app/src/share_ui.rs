// SPDX-License-Identifier: GPL-3.0-or-later
//! User-initiated sharing contains only a canonical video ID and optional time.
use crate::{App, UiState, account_playback};
use oxplay_core::VideoId;
use oxplay_media::Snapshot;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, PartialEq, Eq)]
struct Selection {
    video: VideoId,
    load: u64,
    session: Option<u64>,
}

#[derive(Default)]
pub struct State {
    selection: RefCell<Option<Selection>>,
}

fn context(app: &App, state: &UiState, snapshot: &Snapshot) -> Option<(u64, Option<u64>)> {
    if !app.get_loaded()
        || !app.get_remote_video()
        || app.get_page() != 2
        || app.get_busy()
        || app.get_native_video_child()
        || snapshot.stop_pending
        || snapshot.load_request_id == 0
        || snapshot.load_request_id != snapshot.active_load_request_id
        || !state.player.current_load_is_active()
    {
        return None;
    }
    let lease = account_playback::authorization(state);
    if lease.as_ref().is_some_and(|lease| !lease.is_valid()) {
        return None;
    }
    state.current_video.borrow().as_ref()?;
    Some((
        snapshot.load_request_id,
        lease.map(|lease| lease.generation()),
    ))
}

fn selection(app: &App, state: &UiState, snapshot: &Snapshot) -> Option<Selection> {
    let (load, session) = context(app, state, snapshot)?;
    Some(Selection {
        video: state.current_video.borrow().as_ref()?.id.clone(),
        load,
        session,
    })
}

pub fn clear(app: &App, state: &UiState) {
    state.share_ui.selection.borrow_mut().take();
    app.set_share_link("".into());
    app.set_share_status("".into());
    app.invoke_close_share();
}

pub fn clear_account(app: &App, state: &UiState) {
    let private = state
        .share_ui
        .selection
        .borrow()
        .as_ref()
        .is_some_and(|selection| selection.session.is_some());
    if private {
        clear(app, state);
    }
}

pub fn observe(app: &App, state: &UiState, snapshot: &Snapshot) {
    // Availability observation allocates no URL/ID; URLs are constructed only
    // by explicit user requests, never from media progress notifications.
    let current = context(app, state, snapshot);
    if app.get_share_available() != current.is_some() {
        app.set_share_available(current.is_some());
    }
    let stale = state
        .share_ui
        .selection
        .borrow()
        .as_ref()
        .is_some_and(|pinned| {
            current != Some((pinned.load, pinned.session))
                || state.current_video.borrow().as_ref().map(|video| &video.id)
                    != Some(&pinned.video)
        });
    if stale {
        clear(app, state);
    }
}

pub fn connect(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_share_begin(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return false;
        };
        let Some(current) = selection(&app, &state, &state.player.snapshot()) else {
            clear(&app, &state);
            return false;
        };
        app.set_share_link(current.video.watch_url().into());
        app.set_share_status("Choose a link to copy, or select the address below.".into());
        *state.share_ui.selection.borrow_mut() = Some(current);
        true
    });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_share_copy_request(move |include_time| {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return false;
        };
        let snapshot = state.player.snapshot();
        let current = selection(&app, &state, &snapshot);
        let pinned = state.share_ui.selection.borrow().clone();
        let Some(pinned) = pinned.filter(|pinned| current.as_ref() == Some(pinned)) else {
            clear(&app, &state);
            return false;
        };
        let mut url = pinned.video.watch_url();
        if include_time {
            if !snapshot.position.is_finite()
                || !snapshot.duration.is_finite()
                || snapshot.duration <= 0.
                || state.player.clock_identity().is_none()
            {
                app.set_share_status(
                    "Playback time is not available yet. Copy the video link or try after seeking finishes."
                        .into(),
                );
                return false;
            }
            let seconds = snapshot
                .position
                .max(0.)
                .min(snapshot.duration)
                .min(u32::MAX as f64)
                .floor() as u32;
            url.push_str(&format!("&t={seconds}s"));
        }
        app.set_share_link(url.into());
        // Slint's public LineEdit.copy() sends to the platform clipboard but
        // does not return success/error. Never claim a verified clipboard write.
        app.set_share_status("Copy requested. Paste to verify, or select and copy the address.".into());
        true
    });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_share_closed(move || {
        if let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) {
            state.share_ui.selection.borrow_mut().take();
            app.set_share_link("".into());
            app.set_share_status("".into());
        }
    });
}
