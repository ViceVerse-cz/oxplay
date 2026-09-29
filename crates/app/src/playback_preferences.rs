// SPDX-License-Identifier: GPL-3.0-or-later
//! Playback defaults are local preferences. Native speed acknowledgement and
//! SQLite acknowledgement are separate, bounded, event-driven operations.
use crate::{App, UiState, library_ui};
use serein_core::{PlaybackPreferences, PlaybackSpeed};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Copy)]
struct PendingSpeed {
    serial: u64,
    desired: PlaybackSpeed,
    after_updates: u64,
    persist: bool,
}
impl PendingSpeed {
    fn acknowledged(self, snapshot: &serein_media::Snapshot) -> bool {
        snapshot.speed_updates > self.after_updates
            && snapshot.speed_observed
            && PlaybackSpeed::from_rate(snapshot.speed) == Some(self.desired)
    }
}
#[derive(Default)]
pub struct State {
    ready: Cell<bool>,
    startup: RefCell<Option<(slint::SharedString, u64)>>,
    pending: Cell<Option<PendingSpeed>>,
    serial: Cell<u64>,
    deadline: Timer,
}
impl State {
    pub fn ready(&self) -> bool {
        self.ready.get()
    }
}

pub fn policy(state: &UiState) -> serein_youtube::ResolutionPolicy {
    policy_for(library_ui::desired_preferences(state).playback)
}
fn policy_for(prefs: PlaybackPreferences) -> serein_youtube::ResolutionPolicy {
    serein_youtube::ResolutionPolicy {
        max_height: prefs.quality.height(),
        prefer_h264: true,
    }
}
pub fn startup(app: &App, state: &Rc<UiState>, input: slint::SharedString) {
    if state.playback_preferences.ready() {
        app.invoke_search(input);
    } else {
        *state.playback_preferences.startup.borrow_mut() =
            Some((input, state.focus_intent.epoch()));
    }
}
/// A manual submission supersedes deferred startup work. Do not resolve with
/// default quality while an existing user's preferences are still loading.
pub fn admit_search(app: &App, state: &UiState) -> bool {
    state
        .focus_intent
        .cancel(crate::focus_intent::Scope::LocalStartup);
    state.playback_preferences.startup.borrow_mut().take();
    if !state.playback_preferences.ready() {
        app.set_status("Local settings are still loading. Try the search again shortly.".into());
        return false;
    }
    true
}
pub fn hydrate(app: &App, state: &Rc<UiState>, prefs: PlaybackPreferences) {
    if state.playback_preferences.ready.replace(true) {
        return;
    }
    reset(app, state, prefs);
    let pending = state.playback_preferences.startup.borrow_mut().take();
    if let Some((input, epoch)) = pending {
        app.invoke_search(input);
        if epoch != state.focus_intent.epoch() {
            state.focus_intent.invalidate();
        }
    }
}
pub fn reset(app: &App, state: &Rc<UiState>, prefs: PlaybackPreferences) {
    state.playback_preferences.deadline.stop();
    state.playback_preferences.pending.set(None);
    app.set_speed_busy(false);
    app.set_default_quality_index(prefs.quality.index());
    state.quality_index.set(prefs.quality.index() as usize);
    app.set_quality_index(prefs.quality.index());
    request_native(app, state, prefs.speed, false);
}
pub fn cancel_for_clear(app: &App, state: &Rc<UiState>) {
    state.playback_preferences.startup.borrow_mut().take();
    state.playback_preferences.deadline.stop();
    state.playback_preferences.pending.set(None);
    app.set_speed_busy(false);
    request_native(app, state, state.preferences.get().playback.speed, false);
}
pub fn request(app: &App, state: &Rc<UiState>, rate: f64) {
    sync_speed(app, &state.player.snapshot());
    if state.caption_cache.active() || !state.playback_preferences.ready() {
        app.set_status("Wait for local settings to finish loading or clearing.".into());
        return;
    }
    if state.playback_preferences.pending.get().is_some() {
        app.set_status("The player is still applying the previous speed change.".into());
        return;
    }
    let Some(speed) = PlaybackSpeed::from_rate(rate) else {
        app.set_status("Choose one of the supported playback speeds.".into());
        return;
    };
    request_native(app, state, speed, true);
}
fn sync_speed(app: &App, snapshot: &serein_media::Snapshot) {
    if snapshot.speed_observed {
        let index = PlaybackSpeed::from_rate(snapshot.speed).map_or(-1, PlaybackSpeed::index);
        if app.get_speed_index() != index {
            app.set_speed_index(index);
        }
    }
}
fn request_native(app: &App, state: &Rc<UiState>, desired: PlaybackSpeed, persist: bool) {
    let snapshot = state.player.snapshot();
    if snapshot.speed_observed && PlaybackSpeed::from_rate(snapshot.speed) == Some(desired) {
        sync_speed(app, &snapshot);
        if persist && !library_ui::save_speed(app, state, desired) {
            restore_speed(app, state);
        }
        return;
    }
    if let Err(error) = state.player.set_speed(desired.rate()) {
        app.set_status(error.to_string().into());
        sync_speed(app, &snapshot);
        return;
    }
    let s = &state.playback_preferences;
    let serial = s.serial.get().wrapping_add(1);
    s.serial.set(serial);
    s.pending.set(Some(PendingSpeed {
        serial,
        desired,
        after_updates: snapshot.speed_updates,
        persist,
    }));
    app.set_speed_busy(true);
    if persist {
        app.set_status("Applying playback speed…".into());
    }
    let weak = app.as_weak();
    let state = Rc::downgrade(state);
    s.deadline.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
        let (Some(app), Some(state)) = (weak.upgrade(), state.upgrade()) else { return };
        if state.playback_preferences.pending.get().is_some_and(|pending| pending.serial == serial) {
            state.playback_preferences.pending.set(None);
            app.set_speed_busy(false);
            sync_speed(&app, &state.player.snapshot());
            // No retry loop: a failed restoration also ends here.
            if persist { restore_speed(&app, &state); }
            app.set_status("The player did not confirm the speed change. The saved preference is unchanged.".into());
        }
    });
}
pub fn observe(app: &App, state: &Rc<UiState>, snapshot: &serein_media::Snapshot) {
    sync_speed(app, snapshot);
    let s = &state.playback_preferences;
    let Some(pending) = s
        .pending
        .get()
        .filter(|pending| pending.acknowledged(snapshot))
    else {
        return;
    };
    s.pending.set(None);
    s.deadline.stop();
    app.set_speed_busy(false);
    if pending.persist && !library_ui::save_speed(app, state, pending.desired) {
        restore_speed(app, state);
    }
}
fn restore_speed(app: &App, state: &Rc<UiState>) {
    request_native(app, state, state.preferences.get().playback.speed, false);
}
pub fn save_failed(app: &App, state: &Rc<UiState>, failed: PlaybackPreferences) {
    if failed.speed != state.preferences.get().playback.speed
        && state.playback_preferences.pending.get().is_none()
    {
        restore_speed(app, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_ack_requires_a_new_observed_matching_speed() {
        let pending = PendingSpeed {
            serial: 1,
            desired: PlaybackSpeed::OneAndHalf,
            after_updates: 10,
            persist: true,
        };
        let mut snapshot = serein_media::Snapshot {
            speed: 1.5,
            speed_observed: true,
            speed_updates: 10,
            ..Default::default()
        };
        assert!(!pending.acknowledged(&snapshot));
        snapshot.speed_updates = 11;
        assert!(pending.acknowledged(&snapshot));
        snapshot.speed = 2.;
        assert!(!pending.acknowledged(&snapshot));
        snapshot.speed = 1.5;
        snapshot.speed_observed = false;
        assert!(!pending.acknowledged(&snapshot));
    }
    #[test]
    fn every_supported_ceiling_builds_the_same_explicit_initial_policy() {
        for quality in serein_core::QualityCeiling::ALL {
            let policy = policy_for(PlaybackPreferences {
                quality,
                ..Default::default()
            });
            assert_eq!(policy.max_height, quality.height());
            assert!(policy.prefer_h264);
        }
    }
}
