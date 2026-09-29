// SPDX-License-Identifier: GPL-3.0-or-later
//! Event-driven inactivity for the shared inline transport. No media, models,
//! cursor coordinates, or recurring redraw work belong to this controller.
use crate::{App, UiState};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{cell::Cell, rc::Rc, time::Duration};

const HIDE_AFTER: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct State {
    timer: Timer,
    playing: Cell<bool>,
    load: Cell<u64>,
}

fn may_hide(app: &App, state: &UiState) -> bool {
    state.controls_ui.playing.get()
        && app.get_loaded()
        && app.get_page() == 2
        && !app.get_paused()
        && !state.player.user_pause_intent()
        && !app.get_playback_failed()
        && !app.get_native_video_child()
        && !app.get_controls_held()
        && !state.hidden.get()
}

fn show(app: &App) {
    if !app.get_controls_visible() {
        app.set_controls_visible(true);
    }
}

fn sync(app: &App, state: &Rc<UiState>) {
    if !may_hide(app, state) {
        state.controls_ui.timer.stop();
        if app.get_page() == 2 && !state.hidden.get() {
            show(app);
        }
        return;
    }
    if !app.get_controls_visible() {
        state.controls_ui.timer.stop();
        return;
    }
    // Media events do not move the inactivity deadline. Only interaction does.
    if state.controls_ui.timer.running() {
        return;
    }
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state
        .controls_ui
        .timer
        .start(TimerMode::SingleShot, HIDE_AFTER, move || {
            let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
                return;
            };
            if may_hide(&app, &state) {
                app.set_controls_visible(false);
                // Stop progress scheduling immediately when its transport is
                // hidden; the ordinary property callback also catches layout.
                crate::update(&app, &state);
            }
        });
}

fn activity(app: &App, state: &Rc<UiState>) {
    if app.get_page() != 2 || state.hidden.get() {
        return;
    }
    show(app);
    if may_hide(app, state) && state.controls_ui.timer.running() {
        state.controls_ui.timer.restart();
    } else {
        sync(app, state);
    }
}

pub fn observe(app: &App, state: &Rc<UiState>, playing: bool, load: u64) {
    state.controls_ui.playing.set(playing);
    if state.controls_ui.load.replace(load) != load {
        // New selections and stream replacement can keep the same watch page.
        // Give their controls a fresh grace period without following the clock.
        state.controls_ui.timer.stop();
        if app.get_page() == 2 {
            show(app);
        }
    }
    sync(app, state);
}

pub fn toggle(app: &App, state: &Rc<UiState>) {
    state.controls_ui.timer.stop();
    if app.get_controls_visible() && may_hide(app, state) {
        app.set_controls_visible(false);
    } else {
        activity(app, state);
    }
}

pub fn connect(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_controls_activity(move || {
        if let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) {
            activity(&app, &state);
        }
    });
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_controls_state_changed(move || {
        if let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) {
            sync(&app, &state);
        }
    });
}
