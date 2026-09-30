// SPDX-License-Identifier: GPL-3.0-or-later
//! Immediate, selection-scoped watch navigation. Extraction stays on its worker;
//! this adapter only clears stale presentation and publishes pending UI state.
use crate::{App, UiState};
use serein_core::VideoId;
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Guest(u64),
    Account { selection: u64, session: u64 },
}
#[derive(Default)]
pub struct State {
    scope: Cell<Option<Scope>>,
    account_retry: RefCell<Option<(VideoId, u64)>>,
    cancel_ui: Timer,
    stop_failed: Cell<bool>,
}

fn begin(app: &App, state: &Rc<UiState>, scope: Scope, title: &str, channel: &str) {
    state.watch_loading.cancel_ui.stop();
    // Stop the accepted old file before changing its displayed identity. Stop is
    // an asynchronous media command; no helper join or image decoding occurs here.
    let snapshot = state.player.snapshot();
    let retry_stop = state.watch_loading.stop_failed.get() || state.player.stop_failed();
    let stop_error = if retry_stop
        || ((app.get_loaded() || snapshot.load_request_id != 0) && !snapshot.stop_pending)
    {
        match state.player.stop() {
            Ok(()) => {
                state.watch_loading.stop_failed.set(false);
                None
            }
            Err(error) => {
                state.watch_loading.stop_failed.set(true);
                Some(format!(
                    "Previous playback could not stop: {error}. Select the video again to retry."
                ))
            }
        }
    } else {
        None
    };
    state.progress.stop();
    state.clock_ui.invalidate();
    crate::playback_ui::clear_local(state);
    crate::caption_ui::clear_local(app, state);
    crate::chapters_ui::clear(app, state);
    crate::comments_ui::clear_local(app, state);
    crate::share_ui::clear(app, state);
    crate::channel_avatar::clear(app, state);
    state.current_video.borrow_mut().take();
    app.set_loaded(false);
    app.set_remote_video(false);
    app.set_video_texture(slint::Image::default());
    app.set_video_title(title.into());
    app.set_video_channel(channel.into());
    crate::rating_ui::clear(app, state);
    app.set_playback_status("".into());
    app.set_playback_failed(false);
    app.set_can_retry_playback(false);
    crate::apply_clock(
        app,
        state,
        crate::clock_ui::Values {
            position: 0.,
            duration: 0.,
        },
    );
    app.set_duration(1.);
    state.watch_loading.scope.set(Some(scope));
    app.set_watch_load_error(stop_error.unwrap_or_default().into());
    app.set_watch_loading(true);
    app.set_watch_load_account(matches!(scope, Scope::Account { .. }));
    app.set_page(2);
    app.invoke_reset_watch_scroll();
}

/// Preserve a failed account-boundary stop after its private UI was cleared.
/// Only a subsequent explicit selection retries dispatch, never a polling loop.
pub fn stop_failed(state: &UiState) {
    state.watch_loading.stop_failed.set(true);
}

pub fn prepare_guest(app: &App, state: &Rc<UiState>) {
    // Retire account authority before recording the new guest generation/retry
    // scope; clearing it afterwards would invalidate that just-created job.
    if crate::account_playback::authorization(state).is_some() {
        crate::account_playback::clear(app, state);
    }
}

pub fn guest_begin(app: &App, state: &Rc<UiState>, generation: u64, id: &VideoId) {
    // Existing failed-file retries preserve their native ownership/pause intent.
    // They already show the watch page and use the transport's retry state.
    let snapshot = state.player.snapshot();
    if app.get_loaded()
        && crate::guest_playback::failed_load(&snapshot)
        && crate::playback_ui::guest_retry_video(state, snapshot.load_request_id).as_ref()
            == Some(id)
    {
        return;
    }
    crate::watch_context::capture_guest(app, state, id);
    // The native watch page (related, metadata, chapters, first comments
    // continuation) is read on its own worker while the extractor resolves.
    crate::watch_meta::request(state, id);
    let video = crate::guest_ui::video_summary(state, id);
    begin(
        app,
        state,
        Scope::Guest(generation),
        video.as_ref().map_or("", |v| &v.title),
        video.as_ref().map_or("", |v| &v.channel),
    );
}

pub fn account_begin(app: &App, state: &Rc<UiState>, id: &VideoId) {
    crate::watch_context::clear(app, state);
    let selection = crate::account_playback::generation(state);
    let session = state.account_ui.session_generation();
    let title = state.account_ui.video_title(id).unwrap_or_default();
    begin(
        app,
        state,
        Scope::Account { selection, session },
        &title,
        "",
    );
    *state.watch_loading.account_retry.borrow_mut() = Some((id.clone(), session));
}

pub fn cancel_guest(app: &App, state: &UiState) {
    if matches!(state.watch_loading.scope.get(), Some(Scope::Guest(_))) {
        cancel(app, state);
    }
}
pub fn cancel_account(app: &App, state: &UiState) {
    if matches!(state.watch_loading.scope.get(), Some(Scope::Account { .. })) {
        // Initial extraction has no installed media lease yet. Its known title
        // is still private account metadata and must disappear synchronously on
        // sign-out/cancellation, even when clear() has no active file to retire.
        // Account begin already cleared artwork, captions and the old texture.
        app.set_video_title("".into());
        app.set_video_channel("".into());
        cancel(app, state);
    }
    state.watch_loading.account_retry.borrow_mut().take();
}
fn cancel(app: &App, state: &UiState) {
    state.watch_loading.scope.set(None);
    // Generation observers run while the catalog RefCell is borrowed. Changing
    // visibility can invoke Slint callbacks that inspect it, so defer only this
    // cancellation paint. A newer selection stops/replaces this one-shot timer.
    let weak = app.as_weak();
    state
        .watch_loading
        .cancel_ui
        .start(TimerMode::SingleShot, Duration::ZERO, move || {
            if let Some(app) = weak.upgrade() {
                app.set_watch_loading(false);
                app.set_watch_load_account(false);
                app.set_watch_load_error(
                    "Video loading cancelled. Choose a video to continue.".into(),
                );
            }
        });
}
pub fn guest_failed(app: &App, state: &UiState, generation: u64, message: &str) {
    if state.watch_loading.scope.get() == Some(Scope::Guest(generation)) {
        app.set_watch_loading(false);
        app.set_watch_load_error(message.into());
    }
}
pub fn guest_finished(app: &App, state: &UiState, generation: u64) {
    if state.watch_loading.scope.get() == Some(Scope::Guest(generation)) {
        finish(app, state);
    }
}
pub fn account_error(app: &App, state: &UiState, message: &str) {
    state.watch_loading.cancel_ui.stop();
    app.set_watch_loading(false);
    app.set_watch_load_account(
        state
            .watch_loading
            .account_retry
            .borrow()
            .as_ref()
            .is_some_and(|(_, session)| *session == state.account_ui.session_generation()),
    );
    app.set_watch_load_error(message.into());
}
pub fn account_finished(app: &App, state: &UiState) {
    if matches!(state.watch_loading.scope.get(), Some(Scope::Account { .. })) {
        finish(app, state);
    }
}
pub fn local_finished(app: &App, state: &UiState) {
    state.watch_loading.stop_failed.set(false);
    finish(app, state);
}
fn finish(app: &App, state: &UiState) {
    state.watch_loading.cancel_ui.stop();
    state.watch_loading.scope.set(None);
    state.watch_loading.account_retry.borrow_mut().take();
    app.set_watch_loading(false);
    app.set_watch_load_account(false);
    app.set_watch_load_error("".into());
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    app.on_watch_retry(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else {
            return;
        };
        if app.get_page() != 2
            || app.get_watch_loading()
            || app.get_busy()
            || app.get_account_busy()
        {
            return;
        }
        let account = state.watch_loading.account_retry.borrow().clone();
        if let Some((id, session)) = account {
            if session != state.account_ui.session_generation() {
                cancel_account(&app, &state);
                account_error(
                    &app,
                    &state,
                    "The account session changed. Choose the video again after connecting.",
                );
            } else if let Err(message) = crate::account_playback::request_initial(&app, &state, &id)
            {
                app.set_watch_load_error(message.into());
            }
        } else if app.get_recovery_available() && app.get_recovery_ready() {
            app.invoke_retry_request(app.get_recovery_serial());
        }
    });
}
