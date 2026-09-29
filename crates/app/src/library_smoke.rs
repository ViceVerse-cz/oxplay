// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit offline callback regression, confined to a newly created data root.
use crate::{App, LibraryUi, UiState};
use slint::{ComponentHandle, Model};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const STAGES: [u64; 8] = [3, 6, 9, 12, 15, 18, 21, 25];

pub struct Smoke {
    completed: Rc<Cell<usize>>,
    _timers: Vec<slint::Timer>,
}

impl Smoke {
    pub fn start(app: &App, state: &Rc<UiState>) -> Self {
        let completed = Rc::new(Cell::new(0));
        let query_status = Rc::new(RefCell::new(slint::SharedString::default()));
        let initial_changes = state.model.changes.get();
        let initial_resets = state.model.resets.get();
        let timers = STAGES
            .into_iter()
            .enumerate()
            .map(|(index, stage)| {
                let weak = app.as_weak();
                let state = Rc::downgrade(state);
                let completed = completed.clone();
                let query_status = query_status.clone();
                let timer = slint::Timer::default();
                timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_secs(stage),
                    move || {
                        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else {
                            return;
                        };
                        assert_eq!(completed.get(), index, "library smoke skipped a stage");
                        let library = app.global::<LibraryUi>();
                        assert!(
                            !library.get_busy(),
                            "local page did not settle before next stage"
                        );
                        match stage {
                            3 => {
                                assert_eq!(app.get_page(), 0);
                                assert!(
                                    app.get_browse_active(),
                                    "initial browsing focus was not neutral"
                                );
                                assert!(
                                    !app.get_search_active(),
                                    "untouched startup focused the search editor"
                                );
                                let tab: slint::SharedString = slint::platform::Key::Tab.into();
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyPressed { text: tab.clone() },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyReleased { text: tab },
                                );
                                assert!(
                                    !app.get_browse_active(),
                                    "Tab did not leave the neutral startup scope"
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyPressed { text: "/".into() },
                                );
                                app.window().dispatch_event(
                                    slint::platform::WindowEvent::KeyReleased { text: "/".into() },
                                );
                                assert!(
                                    app.get_search_active(),
                                    "slash did not explicitly focus Search after Tab"
                                );
                                assert!(!library.get_history_enabled());
                                assert!(!state.preferences.get().privacy.local_history);
                                assert!(state.playlists.borrow().is_empty());
                                *query_status.borrow_mut() = library.get_status();
                                app.invoke_open_local_tab(1);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 1));
                                assert!(
                                    library.get_busy(),
                                    "subscriptions request was not admitted"
                                );
                                // Result publication cannot interleave this UI callback.
                                app.invoke_open_local_tab(0);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 1));
                                assert!(library.get_busy());
                            }
                            6 => {
                                assert_eq!(
                                    library.get_status(),
                                    *query_status.borrow(),
                                    "subscriptions query reported an error"
                                );
                                assert_eq!(library.get_rows().row_count(), 0);
                                app.invoke_open_local_tab(1);
                                assert!(
                                    !library.get_busy(),
                                    "same-tab selection reloaded the page"
                                );
                                app.invoke_open_local_tab(0);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 0));
                                assert!(
                                    !library.get_busy(),
                                    "empty playlists must not query an invented ID"
                                );
                            }
                            9 => {
                                app.invoke_navigate(3);
                                library.invoke_history(true);
                                assert_eq!(app.get_page(), 3);
                                assert!(
                                    !library.get_history_enabled(),
                                    "history was enabled optimistically"
                                );
                                assert!(!state.preferences.get().privacy.local_history);
                            }
                            12 => {
                                assert!(
                                    library.get_history_enabled(),
                                    "history opt-in was not acknowledged"
                                );
                                assert!(state.preferences.get().privacy.local_history);
                                *query_status.borrow_mut() = library.get_status();
                                app.invoke_open_local_tab(2);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 2));
                                assert!(library.get_busy());
                                app.invoke_open_local_tab(0);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 2));
                            }
                            15 => {
                                assert_eq!(
                                    library.get_status(),
                                    *query_status.borrow(),
                                    "history query reported an error"
                                );
                                assert_eq!(library.get_rows().row_count(), 0);
                                library.set_confirmation(2);
                                app.invoke_open_local_tab(0);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 2));
                                assert!(!library.get_busy());
                                app.invoke_navigate(0);
                                assert_eq!(library.get_confirmation(), 0);
                                app.invoke_open_local_tab(1);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 1));
                                assert!(library.get_busy());
                            }
                            18 => {
                                assert_eq!(
                                    library.get_status(),
                                    *query_status.borrow(),
                                    "subscriptions query reported an error"
                                );
                                app.invoke_navigate(3);
                                library.invoke_history(false);
                                assert!(
                                    library.get_history_enabled(),
                                    "history changed before storage acknowledgement"
                                );
                            }
                            21 => {
                                assert!(!library.get_history_enabled());
                                assert!(!state.preferences.get().privacy.local_history);
                                app.invoke_open_local_tab(2);
                                assert_eq!(
                                    app.get_page(),
                                    3,
                                    "disabled History remained navigable"
                                );
                                assert!(!library.get_busy());
                                app.invoke_open_local_tab(0);
                                assert_eq!((app.get_page(), library.get_tab()), (1, 0));
                            }
                            25 => {
                                assert_eq!(library.get_rows().row_count(), 0);
                                assert_eq!(library.get_confirmation(), 0);
                                assert!(!state.preferences.get().privacy.local_history);
                                assert_eq!(state.player.snapshot().file_loads, 0);
                                assert!(!app.get_account_connected());
                                assert_eq!(state.model.changes.get(), initial_changes);
                                assert_eq!(state.model.resets.get(), initial_resets);
                            }
                            _ => unreachable!(),
                        }
                        completed.set(index + 1);
                        eprintln!(
                            "library smoke stage={stage} page={} tab={} busy={} history={}",
                            app.get_page(),
                            library.get_tab(),
                            library.get_busy(),
                            library.get_history_enabled()
                        );
                    },
                );
                timer
            })
            .collect();
        Self {
            completed,
            _timers: timers,
        }
    }
    pub fn finish(self) -> Result<(), &'static str> {
        (self.completed.get() == STAGES.len())
            .then_some(())
            .ok_or("local-navigation smoke ended before all assertions completed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_must_complete_before_reporting_success() {
        for completed in 0..=STAGES.len() {
            let smoke = Smoke {
                completed: Rc::new(Cell::new(completed)),
                _timers: Vec::new(),
            };
            assert_eq!(smoke.finish().is_ok(), completed == STAGES.len());
        }
    }
}
