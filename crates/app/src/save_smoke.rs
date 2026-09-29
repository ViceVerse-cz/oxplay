// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit finite real-guest/local-write diagnostic in a fresh private profile.
//! SQL remains on the ordinary worker; account connection is never admitted.
use crate::{App, LibraryUi, SaveUi, UiState, library_ui};
use serein_core::VideoId;
use serein_storage::LocalPlaylistId;
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 8] = [30, 36, 42, 48, 54, 60, 66, 70];
const NAME: &str = "TEST FIXTURE — explicit local Save diagnostic";
struct Checks {
    expected: VideoId,
    collection: Option<LocalPlaylistId>,
    read_ticket: Option<u64>,
    worker_generation: u64,
}
impl Checks {
    fn assert_page(&self, state: &UiState) -> Result<(), &'static str> {
        if self.read_ticket.is_none() || library_ui::accepted_page_read(state) != self.read_ticket {
            return Err("Save diagnostic did not receive a new committed video-page read");
        }
        let Some(page) = library_ui::acknowledged_video_page(state) else {
            return Err("Save diagnostic video-page read is still pending");
        };
        if page.len() != 1 || page[0].id != self.expected {
            return Err("Save diagnostic committed video identity or row count differs");
        }
        Ok(())
    }
    fn step(&mut self, app: &App, state: &UiState, seconds: u64) -> Result<(), &'static str> {
        let ui = app.global::<SaveUi>();
        let library = app.global::<LibraryUi>();
        if app.get_account_connected()
            || app.get_account_playback_active()
            || crate::account_playback::authorization(state).is_some()
        {
            return Err("Save diagnostic must remain guest-only");
        }
        if seconds != 30 && state.worker.borrow().generation() != self.worker_generation {
            return Err("Save diagnostic unexpectedly started another catalog operation");
        }
        match seconds {
            30 => {
                let snapshot = state.player.snapshot();
                if app.get_busy()
                    || library.get_busy()
                    || !app.get_loaded()
                    || !app.get_remote_video()
                    || state
                        .current_video
                        .borrow()
                        .as_ref()
                        .is_none_or(|video| video.id != self.expected || video.title.is_empty())
                    || !state.player.current_load_is_active()
                    || !snapshot.playback_restarted
                    || snapshot.error.is_some()
                    || snapshot.file_loads != 1
                    || !state.playlists.borrow().is_empty()
                {
                    return Err(
                        "Real guest playback or empty private library was not ready within thirty seconds",
                    );
                }
                self.worker_generation = state.worker.borrow().generation();
                // Preference hydration may defer the initial --url callback,
                // so block subsequent searches only after that real load exists.
                app.on_search(|_| {});
                if !snapshot.paused {
                    app.invoke_toggle_pause();
                }
                if !app.invoke_show_local_save() {
                    return Err("Save popup did not admit the active guest video");
                }
                ui.invoke_create_and_save("   ".into());
                if ui.get_busy()
                    || ui.get_saved()
                    || !ui.get_status().starts_with("Enter a playlist name")
                {
                    return Err("Invalid playlist name was not rejected without a write");
                }
                ui.invoke_create_and_save(NAME.into());
                if !ui.get_busy() || ui.get_saved() {
                    return Err("Create and save did not enter real pending state");
                }
            }
            36 => {
                if !ui.get_saved() || ui.get_busy() || library.get_busy() {
                    return Err("Create and save did not receive committed success");
                }
                let collections = state.playlists.borrow();
                if collections.len() != 1 || collections[0].name != NAME {
                    return Err("Committed collection page is not the single requested playlist");
                }
                self.collection = Some(collections[0].id);
                drop(collections);
                self.read_ticket = library_ui::refresh_current_page(app, state);
                if self.read_ticket.is_none() || !library.get_busy() {
                    return Err("Committed video page was not queued on the worker");
                }
            }
            42 => {
                self.assert_page(state)?;
                app.invoke_close_local_save();
                if !app.invoke_show_local_save() {
                    return Err("Save popup did not reopen for existing playlist");
                }
                app.invoke_save_to_playlist(0);
                if !ui.get_busy() || ui.get_saved() {
                    return Err("Existing-playlist save was not queued");
                }
            }
            48 => {
                if !ui.get_saved() || ui.get_busy() || library.get_busy() {
                    return Err("Existing-playlist save was not acknowledged");
                }
                self.read_ticket = library_ui::refresh_current_page(app, state);
                if self.read_ticket.is_none() {
                    return Err("Idempotent Save verification read was not admitted");
                }
            }
            54 => {
                self.assert_page(state)?; // Saving twice must not duplicate membership.
                app.invoke_close_local_save();
                if !app.invoke_show_local_save() {
                    return Err("Save popup did not capture the pre-stop guest load");
                }
                state
                    .player
                    .stop()
                    .map_err(|_| "Native stop submission failed")?;
                app.invoke_save_to_playlist(0);
                if ui.get_busy()
                    || ui.get_saved()
                    || library.get_busy()
                    || !ui.get_status().starts_with("Playback changed")
                {
                    return Err("Stopped captured load was not rejected before worker admission");
                }
            }
            60 => {
                self.read_ticket = library_ui::refresh_current_page(app, state);
                if self.read_ticket.is_none() {
                    return Err("Rejected Save verification read was not admitted");
                }
            }
            66 => {
                self.assert_page(state)?;
                let collections = state.playlists.borrow();
                if collections.len() != 1 || Some(collections[0].id) != self.collection {
                    return Err(
                        "Idempotent or rejected save changed the committed collection identity",
                    );
                }
                if ui.get_busy()
                    || ui.get_saved()
                    || !ui.get_status().starts_with("Playback changed")
                {
                    return Err("Rejected stale save received an unrelated success response");
                }
                app.invoke_close_local_save();
            }
            70 => {
                if state.player.current_load_is_active() || state.player.snapshot().stop_pending {
                    return Err("Save diagnostic native stop did not settle");
                }
                eprintln!(
                    "save diagnostic complete: real_guest=true collection_count=1 video_count=1 worker_reads=3 idempotent_save=true invalid_name_rejected=true stale_load_rejected=true account_used=false"
                );
            }
            _ => return Err("unknown Save diagnostic stage"),
        }
        Ok(())
    }
}

pub struct Smoke {
    completed: Rc<Cell<usize>>,
    failure: Rc<Cell<Option<&'static str>>>,
    _timers: Vec<slint::Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>, expected: VideoId) -> Self {
        // Credential and secondary catalog actions are not admitted. The one
        // initial search callback stays intact until deferred hydration resolves
        // --url; stage30 then disables it after verifying the actual video ID.
        macro_rules! block {
            ($callback:ident, ($($argument:ident),*)) => {{
                app.$callback(move |$($argument),*| { $(let _ = $argument;)* });
            }};
        }
        block!(on_select_video, (index));
        block!(on_more, ());
        block!(on_guest_previous, ());
        block!(on_guest_back, ());
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
        block!(on_account_rating, ());
        block!(on_open_youtube, ());
        block!(on_open_export_guide, ());
        let checks = Rc::new(RefCell::new(Checks {
            expected,
            collection: None,
            read_ticket: None,
            worker_generation: 0,
        }));
        let completed = Rc::new(Cell::new(0));
        let failure = Rc::new(Cell::new(None));
        let mut timers = Vec::new();
        for (index, seconds) in STAGES.into_iter().enumerate() {
            let weak = app.as_weak();
            let state = Rc::downgrade(state);
            let checks = checks.clone();
            let completed = completed.clone();
            let failure = failure.clone();
            let timer = slint::Timer::default();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_secs(seconds),
                move || {
                    if failure.get().is_some() {
                        return;
                    }
                    let result = (|| {
                        let app = weak.upgrade().ok_or("Save diagnostic lost its window")?;
                        let state = state.upgrade().ok_or("Save diagnostic lost its core")?;
                        if completed.get() != index {
                            return Err("Save diagnostic missed a finite stage");
                        }
                        checks.borrow_mut().step(&app, &state, seconds)
                    })();
                    eprintln!("save diagnostic stage={seconds} result={result:?}");
                    match result {
                        Ok(()) => completed.set(index + 1),
                        Err(reason) => {
                            failure.set(Some(reason));
                            let _ = slint::quit_event_loop();
                        }
                    }
                },
            );
            timers.push(timer);
        }
        Self {
            completed,
            failure,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        if let Some(reason) = self.failure.get() {
            return Err(reason);
        }
        if self.completed.get() == STAGES.len() {
            Ok(())
        } else {
            Err("Save diagnostic exited before all finite stages completed")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_or_failed_save_diagnostic_never_passes() {
        for count in 0..=STAGES.len() {
            for failure in [None, Some("synthetic failure")] {
                let smoke = Smoke {
                    completed: Rc::new(Cell::new(count)),
                    failure: Rc::new(Cell::new(failure)),
                    _timers: vec![],
                };
                assert_eq!(
                    smoke.finish().is_ok(),
                    count == STAGES.len() && failure.is_none()
                );
            }
        }
    }
}
