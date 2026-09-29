// SPDX-License-Identifier: GPL-3.0-or-later
//! One request-scoped startup or playback focus transfer. Native input invalidates
//! it before Slint dispatches that input; clocks and cursor motion never touch it.
use crate::{App, UiState};
use slint::{
    ComponentHandle, Timer, TimerMode,
    winit_030::{WinitWindowAccessor, winit},
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    BrowseStartup,
    LocalStartup,
    Guest(u64),
    Account { selection: u64, session: u64 },
}
impl Scope {
    fn is_startup(self) -> bool {
        matches!(self, Self::BrowseStartup | Self::LocalStartup)
    }
}
#[derive(Clone, Copy)]
struct Intent {
    scope: Scope,
    epoch: u64,
    search_active: bool,
}
#[derive(Default)]
pub struct State {
    epoch: Cell<u64>,
    pending: Cell<Option<Intent>>,
    window_focused: Cell<bool>,
    startup_handoff: Timer,
}
impl State {
    pub fn epoch(&self) -> u64 {
        self.epoch.get()
    }
    pub fn arm(&self, scope: Scope, search_active: bool) {
        self.pending
            .set(self.window_focused.get().then_some(Intent {
                scope,
                epoch: self.epoch.get(),
                search_active,
            }));
    }
    pub fn arm_local_startup(&self, search_active: bool) {
        self.arm_startup(Scope::LocalStartup, search_active);
    }
    pub fn arm_browse_startup(&self, search_active: bool) {
        self.arm_startup(Scope::BrowseStartup, search_active);
    }
    fn arm_startup(&self, scope: Scope, search_active: bool) {
        self.pending.set(Some(Intent {
            scope,
            epoch: self.epoch.get(),
            search_active,
        }));
    }
    pub fn invalidate(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.pending.set(None);
    }
    fn search_focus_changed(&self, active: bool) {
        if self.pending.get().is_some_and(|intent| {
            intent.scope != Scope::LocalStartup && intent.search_active != active
        }) {
            self.invalidate();
        }
    }
    pub fn cancel(&self, scope: Scope) {
        if self
            .pending
            .get()
            .is_some_and(|intent| intent.scope == scope)
        {
            self.pending.set(None);
        }
    }
    fn cancel_guest(&self) {
        if self
            .pending
            .get()
            .is_some_and(|intent| matches!(intent.scope, Scope::Guest(_)))
        {
            self.pending.set(None);
        }
    }
    pub fn cancel_account(&self) {
        if self
            .pending
            .get()
            .is_some_and(|intent| matches!(intent.scope, Scope::Account { .. }))
        {
            self.pending.set(None);
        }
    }
    fn consume(&self, scope: Scope, visible: bool) -> bool {
        let Some(intent) = self.pending.get() else {
            return false;
        };
        // A stale result must not consume the newer selection's intention.
        if intent.scope != scope {
            return false;
        }
        self.pending.set(None);
        visible && self.window_focused.get() && intent.epoch == self.epoch.get()
    }
    pub fn window_event(&self, event: &winit::event::WindowEvent) {
        use winit::event::{ElementState, Ime, TouchPhase, WindowEvent};
        let lost_focus = if let WindowEvent::Focused(focused) = event {
            let was_focused = self.window_focused.replace(*focused);
            // An initially inactive native window has not lost focus. Preserve
            // explicit startup intent until its first activation, unless input
            // independently cancels it. A real focus loss still invalidates it.
            !focused
                && (was_focused
                    || !self
                        .pending
                        .get()
                        .is_some_and(|intent| intent.scope.is_startup()))
        } else {
            false
        };
        if matches!(
            event,
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed
        ) || matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } | WindowEvent::MouseWheel { .. }
                | WindowEvent::Touch(winit::event::Touch {
                    phase: TouchPhase::Started,
                    ..
                })
                | WindowEvent::Ime(Ime::Preedit(..) | Ime::Commit(..))
                | WindowEvent::Occluded(true)
        ) || lost_focus
        {
            self.invalidate();
        }
    }
}

pub fn apply(app: &App, state: &UiState, scope: Scope) {
    let visible =
        app.get_page() == 2 && app.get_loaded() && app.window().is_visible() && !state.hidden.get();
    if state.focus_intent.consume(scope, visible) {
        app.invoke_focus_player();
    }
}

/// Either rendering or native activation may happen first. Incomplete startup
/// does not consume intent; intervening user input still cancels it.
pub fn startup_ready(app: &App, state: &Rc<UiState>) {
    let Some(intent) = state
        .focus_intent
        .pending
        .get()
        .filter(|intent| intent.scope.is_startup())
    else {
        return;
    };
    // The native filter runs before Slint applies activation/forward-focus.
    // Defer this one request-scoped handoff until that event has been dispatched.
    let weak = app.as_weak();
    let weak_state = Rc::downgrade(state);
    state.focus_intent.startup_handoff.start(
        TimerMode::SingleShot,
        std::time::Duration::ZERO,
        move || {
            let (Some(app), Some(state)) = (weak.upgrade(), weak_state.upgrade()) else {
                return;
            };
            let focused = app
                .window()
                .with_winit_window(|window| window.has_focus())
                .unwrap_or(false);
            state.focus_intent.window_focused.set(focused);
            if !focused || !app.window().is_visible() || state.hidden.get() {
                return;
            }
            match intent.scope {
                Scope::LocalStartup
                    if app.get_loaded() && app.get_page() == 2 && !app.get_remote_video() =>
                {
                    apply(&app, &state, Scope::LocalStartup);
                }
                Scope::BrowseStartup
                    if matches!(app.get_page(), 0 | 1)
                        && state.focus_intent.consume(Scope::BrowseStartup, true) =>
                {
                    app.invoke_focus_browse();
                }
                _ => {}
            }
        },
    );
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = Rc::downgrade(state);
    app.on_search_focus_changed(move |active| {
        if let Some(state) = weak.upgrade() {
            state.focus_intent.search_focus_changed(active);
        }
    });
    let weak = Rc::downgrade(state);
    app.on_cancel_playback_focus(move || {
        if let Some(state) = weak.upgrade() {
            state.focus_intent.invalidate();
        }
    });
    let weak = Rc::downgrade(state);
    state.worker.borrow_mut().on_generation_changed(move |_| {
        if let Some(state) = weak.upgrade() {
            state.focus_intent.cancel_guest();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::{Ime, WindowEvent};

    fn focused() -> State {
        let state = State::default();
        state.window_event(&WindowEvent::Focused(true));
        state
    }
    #[test]
    fn neutral_browse_startup_is_single_use_and_survives_only_initial_inactivity() {
        let state = State::default();
        state.arm_browse_startup(false);
        state.window_event(&WindowEvent::Focused(false));
        state.search_focus_changed(false);
        state.window_event(&WindowEvent::Focused(true));
        assert!(state.consume(Scope::BrowseStartup, true));
        assert!(!state.consume(Scope::BrowseStartup, true));

        state.arm_browse_startup(false);
        state.window_event(&WindowEvent::Focused(false));
        state.window_event(&WindowEvent::Focused(true));
        assert!(!state.consume(Scope::BrowseStartup, true));
    }
    #[test]
    fn explicit_search_edit_and_new_playback_supersede_neutral_startup() {
        let state = focused();
        state.arm_browse_startup(false);
        state.search_focus_changed(true);
        assert!(!state.consume(Scope::BrowseStartup, true));

        state.arm_browse_startup(false);
        state.window_event(&WindowEvent::Ime(Ime::Commit("later input".into())));
        assert!(!state.consume(Scope::BrowseStartup, true));

        state.arm_browse_startup(false);
        state.arm(Scope::Guest(4), false);
        assert!(!state.consume(Scope::BrowseStartup, true));
        assert!(state.consume(Scope::Guest(4), true));

        state.arm_browse_startup(false);
        state.arm_local_startup(false);
        assert!(!state.consume(Scope::BrowseStartup, true));
        assert!(state.consume(Scope::LocalStartup, true));
    }
    #[test]
    fn neutral_startup_cannot_override_a_later_click_or_occlusion() {
        for event in [
            WindowEvent::MouseInput {
                device_id: winit::event::DeviceId::dummy(),
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
            },
            WindowEvent::Occluded(true),
        ] {
            let state = focused();
            state.arm_browse_startup(false);
            state.window_event(&event);
            assert!(!state.consume(Scope::BrowseStartup, true));
        }
    }
    #[test]
    fn local_startup_can_wait_for_activation_without_accepting_later_user_input() {
        let state = State::default();
        state.arm_local_startup(false);
        state.search_focus_changed(true);
        state.window_event(&WindowEvent::Focused(false));
        state.window_event(&WindowEvent::Focused(true));
        assert!(state.consume(Scope::LocalStartup, true));
        assert!(!state.consume(Scope::LocalStartup, true));
        state.arm_local_startup(true);
        state.window_event(&WindowEvent::Ime(Ime::Commit("new input".into())));
        assert!(!state.consume(Scope::LocalStartup, true));
        state.arm_local_startup(false);
        state.window_event(&WindowEvent::Focused(false));
        state.window_event(&WindowEvent::Focused(true));
        assert!(!state.consume(Scope::LocalStartup, true));
    }
    #[test]
    fn only_the_matching_current_selection_can_consume_focus_once() {
        let state = focused();
        state.arm(Scope::Guest(8), false);
        state.arm(Scope::Guest(9), false);
        assert!(!state.consume(Scope::Guest(8), true));
        assert!(state.consume(Scope::Guest(9), true));
        assert!(!state.consume(Scope::Guest(9), true));
        let account = Scope::Account {
            selection: 4,
            session: 2,
        };
        state.arm(account, false);
        assert!(!state.consume(
            Scope::Account {
                selection: 4,
                session: 3
            },
            true
        ));
        assert!(!state.consume(Scope::Guest(4), true));
        assert!(state.consume(account, true));
    }
    #[test]
    fn later_editing_focus_loss_and_occlusion_prevent_a_delayed_transfer() {
        for event in [
            WindowEvent::Ime(Ime::Preedit(
                "synthetic composing text".into(),
                Some((0, 0)),
            )),
            WindowEvent::Ime(Ime::Commit("synthetic text".into())),
            WindowEvent::Focused(false),
            WindowEvent::Occluded(true),
        ] {
            let state = focused();
            state.arm(Scope::Guest(1), false);
            state.window_event(&event);
            state.window_event(&WindowEvent::Focused(true));
            assert!(!state.consume(Scope::Guest(1), true));
        }
    }
    #[test]
    fn redraw_and_unrelated_cancellation_do_not_steal_the_current_request() {
        let state = focused();
        state.arm(Scope::Guest(2), false);
        state.window_event(&WindowEvent::RedrawRequested);
        state.window_event(&WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(42., 80.),
        });
        state.cancel(Scope::Guest(1));
        state.cancel(Scope::Account {
            selection: 2,
            session: 1,
        });
        assert!(state.consume(Scope::Guest(2), true));
        state.arm(Scope::Guest(3), false);
        state.cancel_guest();
        assert!(!state.consume(Scope::Guest(3), true));
    }
    #[test]
    fn click_and_wheel_invalidate_but_releasing_the_selection_click_does_not() {
        use winit::event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase};
        let device_id = DeviceId::dummy();
        let state = focused();
        state.arm(Scope::Guest(1), false);
        state.window_event(&WindowEvent::MouseInput {
            device_id,
            state: ElementState::Released,
            button: MouseButton::Left,
        });
        assert!(state.consume(Scope::Guest(1), true));
        for event in [
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(0., 1.),
                phase: TouchPhase::Moved,
            },
        ] {
            state.arm(Scope::Guest(2), false);
            state.window_event(&event);
            assert!(!state.consume(Scope::Guest(2), true));
        }
    }
    #[test]
    fn guest_and_account_cancellation_keep_the_other_scopes_intent() {
        let state = focused();
        let account = Scope::Account {
            selection: 7,
            session: 10,
        };
        state.arm(account, false);
        state.cancel_guest();
        assert!(state.consume(account, true));
        state.arm(Scope::Guest(8), false);
        state.cancel_account();
        assert!(state.consume(Scope::Guest(8), true));
        state.arm(account, false);
        state.cancel_account();
        assert!(!state.consume(account, true));
    }
    #[test]
    fn hidden_or_failed_selections_cannot_later_claim_focus() {
        let state = focused();
        state.arm(Scope::Guest(1), false);
        assert!(!state.consume(Scope::Guest(1), false));
        assert!(!state.consume(Scope::Guest(1), true));
        state.arm(Scope::Guest(2), false);
        state.cancel(Scope::Guest(2));
        assert!(!state.consume(Scope::Guest(2), true));
        let inactive = State::default();
        inactive.arm(Scope::Guest(1), false);
        inactive.window_event(&WindowEvent::Focused(true));
        assert!(!inactive.consume(Scope::Guest(1), true));
    }
    #[test]
    fn deferred_focus_notification_from_the_selection_itself_is_not_new_input() {
        let state = focused();
        state.arm(Scope::Guest(1), false);
        state.search_focus_changed(false);
        assert!(state.consume(Scope::Guest(1), true));
        state.arm(Scope::Guest(2), true);
        state.search_focus_changed(false);
        assert!(!state.consume(Scope::Guest(2), true));
        state.arm(Scope::Guest(3), false);
        state.search_focus_changed(true);
        assert!(!state.consume(Scope::Guest(3), true));
    }
}
