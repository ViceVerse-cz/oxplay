// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit offline failure diagnostic. The real supervised /usr/bin/false
//! helper fails extraction; no provider response or metadata is fabricated.
use crate::{App, UiState};
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
const FIXTURE_LABEL: &str =
    "TEST FIXTURE — local failing helper; no real search results or playback.";
const STAGES: [u64; 11] = [2, 3, 5, 6, 8, 9, 10, 12, 13, 16, 18];
#[derive(Default)]
struct Checks {
    serial: i32,
    generation: u64,
}
impl Checks {
    fn failed(
        &mut self,
        app: &App,
        state: &UiState,
        expected_title: &str,
    ) -> Result<(), &'static str> {
        if app.get_busy()
            || !app.get_recovery_available()
            || app.get_recovery_serial() <= self.serial
            || !app.get_status().contains("YouTube extraction failed.")
        {
            return Err(
                "Synthetic helper failure did not publish a fresh classified recovery action",
            );
        }
        if app.get_catalog_subtitle().as_str() != expected_title
            || app.get_catalog_empty_title().as_str() != expected_title
            || app.get_catalog_empty_message().as_str()
                != oxplay_core::ProviderError::ExtractorFailed.to_string()
            || (expected_title == "Couldn’t open this video"
                && app.get_catalog_title().as_str() != "YouTube video")
        {
            return Err(
                "Terminal guest failure retained loading text or an unrelated empty catalog context",
            );
        }
        self.serial = app.get_recovery_serial();
        self.generation = state.worker.borrow().generation();
        Ok(())
    }
    fn cooling(&self, app: &App, state: &UiState) -> Result<(), &'static str> {
        if app.get_recovery_ready()
            || app.invoke_retry_request(self.serial)
            || state.worker.borrow().generation() != self.generation
        {
            return Err("Recovery admitted a request during its monotonic cooldown");
        }
        Ok(())
    }
    fn retry(&self, app: &App, state: &UiState) -> Result<(), &'static str> {
        if !app.get_recovery_ready()
            || !app.invoke_retry_request(self.serial)
            || !app.get_busy()
            || app.get_recovery_available()
            || state.worker.borrow().generation() <= self.generation
            || app.invoke_retry_request(self.serial)
        {
            return Err("Explicit current recovery did not submit exactly once");
        }
        Ok(())
    }
    fn navigate(&self, app: &App) -> Result<(), &'static str> {
        app.invoke_navigate(3);
        if app.get_page() != 3
            || app.get_recovery_available()
            || app.invoke_retry_request(self.serial)
        {
            return Err("Navigation retained or revived the retired recovery action");
        }
        Ok(())
    }
    fn step(
        &mut self,
        app: &App,
        state: &UiState,
        seconds: u64,
        admitted: u64,
    ) -> Result<(), &'static str> {
        if app.get_diagnostic_fixture_label().as_str() != FIXTURE_LABEL {
            return Err("Recovery fixture label was lost during ordinary application updates");
        }
        if app.get_account_connected()
            || app.get_account_playback_active()
            || app.get_loaded()
            || state.model.row_count() != 0
            || state.current_video.borrow().is_some()
            || state.player.snapshot().file_loads != 0
        {
            return Err(
                "Synthetic failure diagnostic unexpectedly published account, media or catalog content",
            );
        }
        match seconds {
            2 => {
                if !state.playback_preferences.ready() {
                    return Err("Local settings did not hydrate before recovery diagnostic");
                }
                app.invoke_search("Oxplay synthetic failure diagnostic".into());
                if !app.get_busy() {
                    return Err("Diagnostic search was not admitted");
                }
            }
            3 | 10 => {
                self.failed(
                    app,
                    state,
                    if seconds == 3 {
                        "Couldn’t load search results"
                    } else {
                        "Couldn’t open this video"
                    },
                )?;
                self.cooling(app, state)?;
            }
            5 | 12 => self.retry(app, state)?,
            6 => {
                let old = self.serial;
                self.failed(app, state, "Couldn’t load search results")?;
                if app.invoke_retry_request(old) {
                    return Err("Old message retried a newer failure");
                }
                self.navigate(app)?;
            }
            8 => {
                app.invoke_navigate(0);
                app.invoke_search(
                    "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv".into(),
                );
            }
            9 => {
                self.failed(app, state, "Couldn’t load this channel")?;
                self.cooling(app, state)?;
                self.navigate(app)?;
                app.invoke_navigate(0);
                app.invoke_search("https://www.youtube.com/watch?v=aqz-KE-bpKQ".into());
            }
            13 => {
                let old = self.serial;
                self.failed(app, state, "Couldn’t open this video")?;
                self.cooling(app, state)?;
                if app.invoke_retry_request(old) {
                    return Err("Old video failure serial retried a newer request");
                }
                app.invoke_show_status_message();
                if !app.get_status_message_open() {
                    return Err("Full error message did not open");
                }
                let message = app.get_status_message_text();
                if !message.contains(FIXTURE_LABEL) || !message.contains(app.get_status().as_str())
                {
                    return Err(
                        "Full error message did not retain the fixture label and actual error",
                    );
                }
            }
            16 => {
                self.navigate(app)?;
                self.generation = state.worker.borrow().generation();
            }
            18 => {
                if app.get_recovery_available()
                    || app.get_recovery_ready()
                    || app.invoke_retry_request(self.serial)
                    || state.worker.borrow().generation() != self.generation
                    || admitted != 5
                {
                    return Err(
                        "Retired recovery resurrected or diagnostic admitted an unexpected request count",
                    );
                }
                eprintln!(
                    "recovery diagnostic admitted_guest_requests={admitted} os_launch_count=not_instrumented network_fixture=false_helper"
                );
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
pub struct Smoke {
    completed: Rc<Cell<usize>>,
    failure: Rc<Cell<Option<&'static str>>>,
    _timers: Vec<Timer>,
}
impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        // Decoder observations own diagnostic-warning. This immutable fixture
        // identity must survive those observations and captured message dialogs.
        app.set_diagnostic_fixture_label(FIXTURE_LABEL.into());
        macro_rules! block { ($callback:ident, ($($arg:ident),*)) => { app.$callback(move |$($arg),*| { $(let _ = $arg;)* }); }; }
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
        let admitted = Rc::new(Cell::new(0_u64));
        let count = admitted.clone();
        state
            .worker
            .borrow_mut()
            .on_submitted(move |_, _| count.set(count.get().saturating_add(1)));
        let checks = Rc::new(RefCell::new(Checks::default()));
        let completed = Rc::new(Cell::new(0));
        let failure = Rc::new(Cell::new(None));
        let mut timers = Vec::new();
        for (index, seconds) in STAGES.into_iter().enumerate() {
            let app = app.as_weak();
            let state = Rc::downgrade(state);
            let checks = checks.clone();
            let completed = completed.clone();
            let failure = failure.clone();
            let admitted = admitted.clone();
            let timer = Timer::default();
            timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(seconds),
                move || {
                    if failure.get().is_some() {
                        return;
                    }
                    let result = (|| {
                        let app = app.upgrade().ok_or("Recovery diagnostic lost its window")?;
                        let state = state.upgrade().ok_or("Recovery diagnostic lost its core")?;
                        if completed.get() != index {
                            return Err("Recovery diagnostic missed a finite stage");
                        }
                        checks
                            .borrow_mut()
                            .step(&app, &state, seconds, admitted.get())
                    })();
                    eprintln!("recovery diagnostic stage={seconds} result={result:?}");
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
        if self.completed.get() != STAGES.len() {
            return Err("Recovery diagnostic ended before all eleven stages completed");
        }
        eprintln!("recovery diagnostic complete");
        Ok(())
    }
}
