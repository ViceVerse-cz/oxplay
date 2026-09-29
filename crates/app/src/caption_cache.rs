// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit local-data clearing: cancel admission, unload, delete on the worker,
//! then commit the library deletion. No filesystem work or waiting on Slint.
use crate::{
    App, CaptionsUi, LibraryUi, UiState, caption_files::PurgeId, caption_ui, library_ui,
    playback_ui,
};
use slint::ComponentHandle;
use std::{cell::Cell, rc::Rc, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Files(PurgeId),
    Library(PurgeId),
}
#[derive(Default)]
pub struct State {
    phase: Cell<Option<Phase>>,
    deadline: slint::Timer,
}
impl State {
    pub fn active(&self) -> bool {
        self.phase.get().is_some()
    }
}
fn status(app: &App, message: impl Into<slint::SharedString>) {
    let message = message.into();
    app.global::<LibraryUi>().set_status(message.clone());
    app.set_status(message);
}
fn finish(app: &App, state: &UiState, message: impl Into<slint::SharedString>) {
    let Some(phase) = state.caption_cache.phase.take() else {
        return;
    };
    state.caption_cache.deadline.stop();
    let (Phase::Files(id) | Phase::Library(id)) = phase;
    caption_ui::files(state).end_purge(id);
    let mut worker = state.worker.borrow_mut();
    worker.cancel(); // discard Busy/stale responses queued during the barrier
    worker.set_blocked(false);
    drop(worker);
    app.set_busy(false);
    app.global::<LibraryUi>().set_busy(false);
    status(app, message);
}
pub fn begin(app: &App, state: &Rc<UiState>) {
    if state.caption_cache.active() {
        return;
    }
    let files = caption_ui::files(state);
    let id = match files.begin_purge() {
        Ok(id) => id,
        Err(error) => {
            status(app, error);
            return;
        }
    };
    state.caption_cache.phase.set(Some(Phase::Files(id)));
    crate::home_ui::cancel(app, state);
    crate::account_playback::clear(app, state);
    state.worker.borrow_mut().set_blocked(true);
    playback_ui::clear_local(state);
    if let Err(error) = state.player.stop() {
        finish(
            app,
            state,
            format!("Playback could not stop; local library was not deleted. {error}"),
        );
        return;
    }
    // Media retains successful captions until exact END_FILE/destruction and
    // pending commands until reply. Dropping UI caches cannot delete live files.
    caption_ui::clear_local(app, state);
    state.current_video.borrow_mut().take();
    state.progress.stop();
    app.set_loaded(false);
    app.set_video_texture(slint::Image::default());
    app.set_video_title("".into());
    app.set_video_channel("".into());
    app.set_remote_video(false);
    app.set_busy(true);
    app.global::<LibraryUi>().set_busy(true);
    status(app, "Stopping playback and clearing cached captions…");
    let weak = app.as_weak();
    let state_weak = Rc::downgrade(state);
    state.caption_cache.deadline.start(slint::TimerMode::SingleShot, Duration::from_secs(15), move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state_weak.upgrade()) else { return };
        if state.caption_cache.phase.get() == Some(Phase::Files(id)) {
            finish(&app, &state, "Caption cleanup is incomplete; the local library was not deleted. Wait for playback to stop, then retry. Files still owned by playback are retained safely.");
        }
    });
}
pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    caption_ui::files(state).on_purge_complete(move || {
        let _ = weak.upgrade_in_event_loop(|app| app.invoke_caption_cache_wake());
    });
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    app.on_caption_cache_wake(move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else { return };
        let Some(Phase::Files(id)) = state.caption_cache.phase.get() else { return };
        let Some(result) = caption_ui::files(&state).purge_result(id) else { return };
        match result {
            Err(error) => finish(&app, &state, format!("Caption cleanup is incomplete; local library was not deleted. {error}")),
            Ok(()) => {
                // Accepted SQLite mutations are never timed out or claimed
                // cancelled: wait for their real terminal response.
                state.caption_cache.deadline.stop();
                state.caption_cache.phase.set(Some(Phase::Library(id)));
                if library_ui::clear_after_caption_purge(&app, &state) {
                    status(&app, "Caption cache cleared. Deleting local library data…");
                } else {
                    finish(&app, &state, "Caption cache cleared, but the library is busy. Retry to delete local library data.");
                }
            }
        }
    });
}
pub fn library_finished(app: &App, state: &UiState, result: Result<(), &str>) -> bool {
    if !matches!(state.caption_cache.phase.get(), Some(Phase::Library(_))) {
        return false;
    }
    let message = match result {
        Ok(()) => "Local library and cached captions cleared. Your YouTube account is unchanged."
            .to_owned(),
        Err(error) => format!("Caption cache cleared, but local library deletion failed. {error}"),
    };
    app.global::<CaptionsUi>()
        .set_status("Caption cache cleared. Select a video to load captions again.".into());
    finish(app, state, message);
    true
}
