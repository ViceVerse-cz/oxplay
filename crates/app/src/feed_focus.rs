// SPDX-License-Identifier: GPL-3.0-or-later
//! One surface/identity-scoped handoff for the bounded feed and related list.
//! Actual cards retain keyboard/accessibility focus; no virtual delegate index.
use crate::{App, UiState, VideoRow};
use slint::{ComponentHandle, Model, Timer, TimerMode, winit_030::WinitWindowAccessor};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    Feed,
    Related,
}
impl Surface {
    fn decode(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Feed),
            1 => Some(Self::Related),
            _ => None,
        }
    }
    fn value(self) -> i32 {
        match self {
            Self::Feed => 0,
            Self::Related => 1,
        }
    }
    fn active(app: &App) -> Option<Self> {
        if app.get_native_video_child() {
            return None;
        }
        match app.get_page() {
            0 => Some(Self::Feed),
            2 if !app.get_fullscreen_active() => Some(Self::Related),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    surface: Surface,
    kind: slint::SharedString,
    id: slint::SharedString,
}
impl Identity {
    fn from_row(row: VideoRow, surface: Surface) -> Self {
        Self {
            surface,
            kind: row.kind,
            id: row.id,
        }
    }
}
#[derive(Clone)]
struct Target {
    identity: Identity,
    index: usize,
    serial: i32,
}
#[derive(Default)]
struct Intent {
    serial: i32,
    current: Option<Identity>,
    pending: Option<Target>,
    parking: bool,
    transferring: bool,
}
impl Intent {
    fn cancel(&mut self) {
        self.pending = None;
        self.transferring = false;
    }
    fn reset(&mut self) {
        self.cancel();
        self.current = None;
        self.parking = false;
    }
    fn begin(&mut self, identity: Identity, index: usize) -> i32 {
        self.serial = self.serial.checked_add(1).unwrap_or(1);
        self.pending = Some(Target {
            identity,
            index,
            serial: self.serial,
        });
        self.transferring = true;
        self.serial
    }
    fn parked(&mut self) {
        self.parking = true;
        self.current = None;
        self.transferring = false;
    }
    fn claim(&mut self, identity: &Identity, index: usize, serial: i32) -> bool {
        let valid = self.ready(identity, index, serial);
        if valid {
            self.transferring = true;
        }
        valid
    }
    fn ready(&self, identity: &Identity, index: usize, serial: i32) -> bool {
        self.parking
            && self.pending.as_ref().is_some_and(|target| {
                target.identity == *identity && target.index == index && target.serial == serial
            })
    }
    fn card_focus(&mut self, identity: Identity, focused: bool) {
        if focused {
            if !self.transferring {
                self.cancel();
            }
            self.current = Some(identity);
            self.parking = false;
        } else if !self.transferring && self.current.as_ref() == Some(&identity) {
            self.current = None;
            self.cancel();
        }
    }
    fn parking_lost(&mut self) {
        if !self.transferring {
            self.parking = false;
            self.cancel();
        }
    }
    fn complete(&mut self, serial: i32, focused: bool) {
        if self
            .pending
            .as_ref()
            .is_some_and(|target| target.serial == serial)
        {
            self.pending = None;
            self.transferring = false;
            if !focused {
                self.current = None;
            }
        }
    }
}
#[derive(Default)]
pub struct State {
    intent: RefCell<Intent>,
    delivery: Timer,
}

// Commands 0/1 are Shift-Tab/Tab: reject at boundaries to leave the feed.
fn destination(
    index: usize,
    command: i32,
    count: usize,
    columns: usize,
    rows: usize,
) -> Option<usize> {
    if count == 0 || count > crate::model::MAX_CATALOG_ROWS || index >= count {
        return None;
    }
    let columns = columns.clamp(1, crate::groups::MAX_COLUMNS);
    let page = columns * rows.clamp(1, crate::model::MAX_CATALOG_ROWS);
    let target = match command {
        0 => index.checked_sub(1)?,
        1 => {
            if index + 1 >= count {
                return None;
            }
            index + 1
        }
        2 => index.saturating_sub(columns),
        3 => index.saturating_add(columns),
        4 => 0,
        5 => count - 1,
        6 => index.saturating_sub(page),
        7 => index.saturating_add(page),
        8 => index.saturating_sub(1),
        9 => index.saturating_add(1),
        _ => return None,
    };
    Some(target.min(count - 1))
}
fn identity(state: &UiState, surface: Surface, index: usize) -> Option<Identity> {
    state
        .model
        .row_data(index)
        .map(|row| Identity::from_row(row, surface))
}
fn parking_active(app: &App, surface: Surface) -> bool {
    match surface {
        Surface::Feed => app.get_feed_parking_active(),
        Surface::Related => app.get_related_parking_active(),
    }
}
fn clear_request(app: &App, state: &UiState) {
    state.feed_focus.delivery.stop();
    state.feed_focus.intent.borrow_mut().cancel();
    app.set_feed_focus_delivery(0);
    app.set_feed_focus_serial(0);
    app.set_feed_focus_target(-1);
    app.set_feed_focus_surface(-1);
}
/// Explicit page/model replacement must not rely on destruction delivering a
/// focus-lost callback: the window holds only a weak reference to old instances.
pub fn reset(app: &App, state: &UiState) {
    clear_request(app, state);
    state.feed_focus.intent.borrow_mut().reset();
    app.set_feed_focused_index(-1);
}
fn park(app: &App, state: &UiState, surface: Surface, index: usize) -> bool {
    if Surface::active(app) != Some(surface)
        || app.get_busy()
        || state.hidden.get()
        || index >= state.model.row_count()
        || !app
            .window()
            .with_winit_window(|window| window.has_focus())
            .unwrap_or(false)
    {
        return false;
    }
    let Some(identity) = identity(state, surface, index) else {
        return false;
    };
    // Suppress any earlier card's construction callback before parking.
    state.feed_focus.delivery.stop();
    app.set_feed_focus_delivery(0);
    app.set_feed_focus_serial(0);
    app.set_feed_focus_target(-1);
    app.set_feed_focus_surface(-1);
    state.feed_focus.intent.borrow_mut().begin(identity, index);
    app.invoke_park_feed_focus(surface.value());
    if !parking_active(app, surface) {
        clear_request(app, state);
        return false;
    }
    state.feed_focus.intent.borrow_mut().parked();
    app.set_feed_focused_index(-1);
    true
}
fn publish_request(app: &App, state: &UiState) {
    let pending = state.feed_focus.intent.borrow().pending.clone();
    let Some(target) = pending else {
        return;
    };
    match target.identity.surface {
        Surface::Feed => app.invoke_reveal_feed_item(target.index as i32),
        Surface::Related => app.invoke_reveal_related_item(target.index as i32),
    }
    app.set_feed_focus_surface(target.identity.surface.value());
    app.set_feed_focus_target(target.index as i32);
    app.set_feed_focus_serial(target.serial);
}
fn request(app: &App, state: &UiState, surface: Surface, index: usize) {
    if park(app, state, surface, index) {
        publish_request(app, state);
    }
}
/// Preserve a focused video across a same-page local membership reconciliation.
/// Parking owns no row data borrow while model notifications run.
pub fn reconcile_home(
    app: &App,
    state: &UiState,
    change: impl FnOnce() -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let current = {
        let intent = state.feed_focus.intent.borrow();
        intent.current.clone().or_else(|| {
            intent
                .pending
                .as_ref()
                .map(|target| target.identity.clone())
        })
    };
    let restore = current.filter(|key| {
        key.surface == Surface::Feed
            && app.get_page() == 0
            && (app.get_feed_focused_index() >= 0 || app.get_feed_parking_active())
    });
    let parked = restore
        .as_ref()
        .and_then(|key| {
            (0..state.model.row_count())
                .find(|&i| identity(state, Surface::Feed, i).as_ref() == Some(key))
        })
        .is_some_and(|index| park(app, state, Surface::Feed, index));
    let result = change();
    if parked {
        let index = restore.as_ref().and_then(|key| {
            (0..state.model.row_count())
                .find(|&i| identity(state, Surface::Feed, i).as_ref() == Some(key))
        });
        if let Some(index) = index {
            if let Some(target) = state.feed_focus.intent.borrow_mut().pending.as_mut() {
                target.index = index;
            }
            publish_request(app, state);
        } else {
            reset(app, state);
            app.invoke_focus_browse();
        }
    }
    result
}
/// Called before old row instances are dropped. The parked scope survives regroup.
pub fn columns_changed(app: &App, state: &UiState, columns: usize) {
    let current = {
        let intent = state.feed_focus.intent.borrow();
        intent.current.clone().or_else(|| {
            intent
                .pending
                .as_ref()
                .map(|target| target.identity.clone())
        })
    };
    let restore = current
        .and_then(|key| {
            (0..state.model.row_count())
                .find(|&index| identity(state, Surface::Feed, index).as_ref() == Some(&key))
        })
        .filter(|_| {
            (app.get_feed_focused_index() >= 0 || app.get_feed_parking_active())
                && app.get_page() == 0
                && !state.hidden.get()
        });
    let parked = restore.is_some_and(|index| park(app, state, Surface::Feed, index));
    state.groups.set_columns(columns, &state.model);
    if parked {
        publish_request(app, state);
    }
}

pub fn bind(app: &App, state: &Rc<UiState>) {
    let weak = app.as_weak();
    let state_key = state.clone();
    app.on_feed_key(move |surface, index, command| {
        let Some(app) = weak.upgrade() else {
            return false;
        };
        let Some(surface) = Surface::decode(surface) else {
            return false;
        };
        if index < 0 || Surface::active(&app) != Some(surface) || app.get_busy() {
            return false;
        }
        let Some(target) = destination(
            index as usize,
            command,
            state_key.model.row_count(),
            if surface == Surface::Feed {
                app.get_columns() as usize
            } else {
                1
            },
            if surface == Surface::Feed {
                app.get_feed_page_rows().max(1) as usize
            } else {
                app.get_related_page_rows().max(1) as usize
            },
        ) else {
            return false;
        };
        request(&app, &state_key, surface, target);
        true
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_focus_entry(move || {
        if let Some(app) = weak.upgrade()
            && let Some(surface) = Surface::active(&app)
        {
            request(&app, &s, surface, 0);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_focus_ready(move |surface, index, kind, id, serial| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let Some(surface) = Surface::decode(surface) else {
            return;
        };
        let key = Identity { kind, id, surface };
        if index < 0
            || Surface::active(&app) != Some(surface)
            || !parking_active(&app, surface)
            || !s
                .feed_focus
                .intent
                .borrow()
                .ready(&key, index as usize, serial)
            || identity(&s, surface, index as usize).as_ref() != Some(&key)
        {
            return;
        }
        // Repeated init precedes the outer group's listview_layout y assignment.
        // Schedule once after that stack unwinds; never focus from its init.
        let weak = app.as_weak();
        let weak_state = Rc::downgrade(&s);
        s.feed_focus
            .delivery
            .start(TimerMode::SingleShot, Duration::from_millis(1), move || {
                let (Some(app), Some(s)) = (weak.upgrade(), weak_state.upgrade()) else {
                    return;
                };
                if Surface::active(&app) == Some(surface)
                    && !s.hidden.get()
                    && parking_active(&app, surface)
                    && s.feed_focus
                        .intent
                        .borrow()
                        .ready(&key, index as usize, serial)
                    && identity(&s, surface, index as usize).as_ref() == Some(&key)
                {
                    app.set_feed_focus_delivery(serial);
                }
            });
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_card_focus(move |surface, index, kind, id, focused| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let Some(surface) = Surface::decode(surface) else {
            return;
        };
        let key = Identity { kind, id, surface };
        if Surface::active(&app) != Some(surface) {
            return;
        }
        if focused
            && usize::try_from(index)
                .ok()
                .and_then(|index| identity(&s, surface, index))
                .as_ref()
                != Some(&key)
        {
            return;
        }
        s.feed_focus
            .intent
            .borrow_mut()
            .card_focus(key.clone(), focused);
        if focused {
            app.set_feed_focused_index(index);
        } else if s.feed_focus.intent.borrow().current.is_none() {
            app.set_feed_focused_index(-1);
        }
        if s.feed_focus.intent.borrow().pending.is_none() {
            s.feed_focus.delivery.stop();
            app.set_feed_focus_delivery(0);
            app.set_feed_focus_serial(0);
            if focused && surface == Surface::Related {
                // Ordinary Tab entry is revealed by Slint AFTER focus-gained.
                // Its Flickable write does not emit the application's scrolled
                // callback. Save/reveal after that stack unwinds, using the same
                // single timer and identity checks as explicit row handoffs.
                let weak = app.as_weak();
                let weak_state = Rc::downgrade(&s);
                s.feed_focus.delivery.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(1),
                    move || {
                        let (Some(app), Some(s)) = (weak.upgrade(), weak_state.upgrade()) else {
                            return;
                        };
                        let still_current = {
                            let intent = s.feed_focus.intent.borrow();
                            intent.pending.is_none() && intent.current.as_ref() == Some(&key)
                        };
                        if still_current
                            && Surface::active(&app) == Some(Surface::Related)
                            && !s.hidden.get()
                            && app.get_feed_focused_index() == index
                            && identity(&s, surface, index as usize).as_ref() == Some(&key)
                        {
                            app.invoke_reveal_related_item(index);
                        }
                    },
                );
            }
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_focus_claim(move |surface, index, kind, id, serial| {
        let Some(app) = weak.upgrade() else {
            return false;
        };
        let Some(surface) = Surface::decode(surface) else {
            return false;
        };
        if Surface::active(&app) != Some(surface)
            || s.hidden.get()
            || !parking_active(&app, surface)
        {
            return false;
        }
        let key = Identity { kind, id, surface };
        if usize::try_from(index)
            .ok()
            .and_then(|index| identity(&s, surface, index))
            .as_ref()
            != Some(&key)
        {
            return false;
        }
        s.feed_focus
            .intent
            .borrow_mut()
            .claim(&key, index as usize, serial)
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_focus_complete(move |serial, focused| {
        s.feed_focus.intent.borrow_mut().complete(serial, focused);
        if s.feed_focus.intent.borrow().pending.is_none()
            && let Some(app) = weak.upgrade()
        {
            s.feed_focus.delivery.stop();
            app.set_feed_focus_delivery(0);
            app.set_feed_focus_serial(0);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_parking_lost(move || {
        s.feed_focus.intent.borrow_mut().parking_lost();
        if s.feed_focus.intent.borrow().pending.is_none()
            && let Some(app) = weak.upgrade()
        {
            s.feed_focus.delivery.stop();
            app.set_feed_focus_delivery(0);
            app.set_feed_focus_serial(0);
        }
    });
    let weak = app.as_weak();
    let s = state.clone();
    app.on_feed_focus_cancel(move || {
        if let Some(app) = weak.upgrade() {
            reset(&app, &s);
        }
    });
}

/// Native input is filtered before Slint dispatch, so an older pending target
/// cannot acquire focus after an unrelated key, click, IME action or deactivation.
pub fn window_event(
    app: &App,
    state: &UiState,
    event: &slint::winit_030::winit::event::WindowEvent,
) {
    use slint::winit_030::winit::event::{ElementState, WindowEvent};
    use slint::winit_030::winit::keyboard::{Key, NamedKey};
    // Rapid navigation is coalesced from the pending logical index by the
    // persistent parking scope, even before any requested card is constructed.
    if Surface::active(app).is_some_and(|surface| parking_active(app, surface))
        && state.feed_focus.intent.borrow().pending.is_some()
        && matches!(event, WindowEvent::KeyboardInput { event, .. }
            if event.state == ElementState::Pressed && matches!(event.logical_key,
                Key::Named(NamedKey::Tab | NamedKey::ArrowUp | NamedKey::ArrowDown
                    | NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home
                    | NamedKey::End | NamedKey::PageUp | NamedKey::PageDown)))
    {
        return;
    }
    if matches!(
        event,
        WindowEvent::Focused(false)
            | WindowEvent::Occluded(true)
            | WindowEvent::Ime(_)
            | WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
            | WindowEvent::MouseWheel { .. }
    ) || matches!(event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed)
    {
        clear_request(app, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(n: usize) -> Identity {
        Identity {
            surface: Surface::Feed,
            kind: "Video".into(),
            id: n.to_string().into(),
        }
    }
    #[test]
    fn every_bounded_page_item_is_keyboard_reachable_and_tab_can_leave() {
        for count in [1, 20, 100, 120] {
            for columns in 1..=crate::groups::MAX_COLUMNS {
                let mut index = 0;
                for expected in 1..count {
                    index = destination(index, 1, count, columns, 2).unwrap();
                    assert_eq!(index, expected);
                }
                assert_eq!(destination(index, 1, count, columns, 2), None);
                assert_eq!(destination(0, 0, count, columns, 2), None);
                assert_eq!(destination(0, 5, count, columns, 2), Some(count - 1));
                assert_eq!(destination(index, 4, count, columns, 2), Some(0));
                for command in 2..=9 {
                    assert!(destination(index, command, count, columns, 2).unwrap() < count);
                }
            }
        }
        assert_eq!(destination(0, 1, 121, 2, 2), None);
        assert_eq!(destination(100, 4, 100, 2, 2), None);
    }
    #[test]
    fn six_column_navigation_uses_real_rows_and_clamps_the_partial_final_group() {
        assert_eq!(destination(17, 2, 100, 6, 5), Some(11));
        assert_eq!(destination(17, 3, 100, 6, 5), Some(23));
        assert_eq!(destination(17, 7, 100, 6, 5), Some(47));
        assert_eq!(destination(47, 6, 100, 6, 5), Some(17));
        assert_eq!(destination(95, 3, 100, 6, 5), Some(99));
        assert_eq!(destination(99, 3, 100, 6, 5), Some(99));
        assert_eq!(destination(99, 1, 100, 6, 5), None);
        assert_eq!(destination(99, 0, 100, 6, 5), Some(98));
    }
    #[test]
    fn only_matching_live_parked_identity_can_complete_a_handoff() {
        let mut intent = Intent::default();
        let old = intent.begin(key(4), 4);
        intent.parked();
        let serial = intent.begin(key(8), 8);
        intent.parked();
        assert!(!intent.claim(&key(4), 4, old));
        assert!(!intent.claim(&key(9), 8, serial));
        assert!(!intent.claim(&key(8), 7, serial));
        assert!(intent.claim(&key(8), 8, serial));
        intent.parking_lost(); // Deliberate parking-to-card focus event.
        intent.card_focus(key(8), true);
        intent.complete(serial, true);
        assert_eq!(intent.current, Some(key(8)));
        assert!(intent.pending.is_none());
        assert!(!intent.claim(&key(8), 8, serial));
    }
    #[test]
    fn outside_focus_or_input_cancels_delayed_construction_without_stealing() {
        for loses_parking in [true, false] {
            let mut intent = Intent::default();
            let serial = intent.begin(key(99), 99);
            intent.parked();
            if loses_parking {
                intent.parking_lost();
            } else {
                intent.cancel();
            }
            assert!(!intent.claim(&key(99), 99, serial));
        }
    }
    #[test]
    fn regroup_parking_ignores_old_card_loss_but_restores_exact_identity() {
        let mut intent = Intent::default();
        intent.card_focus(key(17), true);
        let serial = intent.begin(key(17), 17);
        intent.card_focus(key(17), false); // Old card loses focus to parking.
        intent.parked();
        intent.card_focus(key(17), false); // Old instance is later destroyed.
        assert!(intent.claim(&key(17), 17, serial));
        intent.card_focus(key(17), true);
        intent.complete(serial, true);
        assert_eq!(intent.current, Some(key(17)));
    }
    #[test]
    fn rapid_navigation_coalesces_without_requiring_a_constructed_card() {
        let mut intent = Intent::default();
        let old = intent.begin(key(99), 99);
        intent.parked();
        for command in [8, 8, 9] {
            let index = intent.pending.as_ref().unwrap().index;
            let next = destination(index, command, 100, 2, 1).unwrap();
            intent.begin(key(next), next);
            intent.parked();
        }
        let target = intent.pending.clone().unwrap();
        assert_eq!(target.index, 98);
        assert!(!intent.claim(&key(99), 99, old));
        assert!(intent.claim(&key(98), 98, target.serial));
        intent.card_focus(key(98), true);
        intent.complete(target.serial, true);
        assert_eq!(intent.current, Some(key(98)));
    }
    #[test]
    fn page_replacement_explicitly_retires_current_and_pending_identity() {
        let mut intent = Intent::default();
        intent.card_focus(key(17), true);
        let serial = intent.begin(key(99), 99);
        intent.parked();
        intent.reset();
        assert!(intent.current.is_none());
        assert!(intent.pending.is_none());
        assert!(!intent.parking);
        assert!(!intent.claim(&key(99), 99, serial));
    }
    #[test]
    fn construction_readiness_does_not_grant_focus_or_survive_cancellation() {
        let mut intent = Intent::default();
        let serial = intent.begin(key(99), 99);
        intent.parked();
        assert!(intent.ready(&key(99), 99, serial));
        assert!(!intent.transferring);
        assert!(intent.current.is_none());
        let newer = intent.begin(key(98), 98);
        intent.parked();
        assert!(!intent.ready(&key(99), 99, serial));
        assert!(intent.ready(&key(98), 98, newer));
        intent.cancel();
        assert!(!intent.ready(&key(98), 98, newer));
        assert!(!intent.claim(&key(98), 98, newer));
    }

    #[test]
    fn related_navigation_reaches_every_row_without_catalog_notifications() {
        let model = crate::model::CatalogModel::<VideoRow>::default();
        model.replace(
            (0..120)
                .map(|index| VideoRow {
                    kind: "Video".into(),
                    id: index.to_string().into(),
                    ..Default::default()
                })
                .collect(),
        );
        let before = (model.changes.get(), model.resets.get());
        let mut intent = Intent::default();
        for index in 0..120 {
            let identity = Identity::from_row(model.row_data(index).unwrap(), Surface::Related);
            let serial = intent.begin(identity.clone(), index);
            intent.parked();
            assert!(intent.claim(&identity, index, serial));
            intent.card_focus(identity, true);
            intent.complete(serial, true);
            if index < 119 {
                assert_eq!(destination(index, 1, 120, 1, 3), Some(index + 1));
            }
        }
        assert_eq!(destination(119, 1, 120, 1, 3), None);
        assert_eq!(destination(0, 0, 120, 1, 3), None);
        assert_eq!(destination(0, 5, 120, 1, 3), Some(119));
        assert_eq!(destination(119, 4, 120, 1, 3), Some(0));
        assert_eq!(destination(12, 7, 120, 1, 3), Some(15));
        assert_eq!(destination(12, 6, 120, 1, 3), Some(9));
        assert_eq!((model.changes.get(), model.resets.get()), before);
    }

    #[test]
    fn identical_rows_on_other_surface_cannot_claim_or_clear_related_focus() {
        let feed = key(17);
        let related = Identity {
            surface: Surface::Related,
            ..feed.clone()
        };
        let mut intent = Intent::default();
        let old = intent.begin(feed.clone(), 17);
        intent.parked();
        intent.reset(); // navigation cancels the old surface before its callbacks
        let serial = intent.begin(related.clone(), 17);
        intent.parked();
        assert!(!intent.ready(&feed, 17, serial));
        assert!(!intent.claim(&related, 17, old));
        assert!(intent.claim(&related, 17, serial));
        intent.card_focus(related.clone(), true);
        intent.complete(serial, true);
        intent.card_focus(feed, false);
        assert_eq!(intent.current, Some(related));
    }

    #[test]
    fn related_title_thumbnail_updates_keep_identity_and_reset_retires_requests() {
        let mut row = VideoRow {
            kind: "Video".into(),
            id: "synthetic-id".into(),
            ..Default::default()
        };
        let key = Identity::from_row(row.clone(), Surface::Related);
        let mut intent = Intent::default();
        let serial = intent.begin(key.clone(), 99);
        intent.parked();
        row.title = "Changed synthetic title".into();
        row.thumbnail_ready = true;
        assert!(intent.ready(&Identity::from_row(row, Surface::Related), 99, serial));
        intent.reset(); // fullscreen, page replacement, or clear-data admission
        assert!(!intent.claim(&key, 99, serial));
    }
}
