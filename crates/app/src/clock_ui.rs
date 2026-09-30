// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in cosmetic-clock scheduling experiment. No timers or redraw requests.
//!
//! The host samples at its existing cadence and admits staging only for visible
//! progress on an actively playing, presentation-ready native display clock.
//! BeforeRendering takes ownership before setting fixed-width UI properties.
//! Slint cf3b07d's OpenGL texture example likewise sets a generated property in
//! this callback; FemtoVG invokes it inside the draw dependency tracker before
//! item traversal. This module never changes layout, transport state or models.
use oxplay_media::ClockIdentity;
use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Values {
    pub position: f64,
    /// Used to compute remaining text; the host still updates the duration
    /// property immediately because it defines the slider's range.
    pub duration: f64,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    values: Values,
    identity: ClockIdentity,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub staged: u64,
    pub overwritten: u64,
    pub applied_in_render: u64,
    pub invalidated: u64,
}

pub struct State {
    enabled: bool,
    pending: Cell<Option<Pending>>,
    stats: Cell<Stats>,
}

impl State {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            pending: Cell::new(None),
            stats: Cell::default(),
        }
    }

    /// Return values for immediate application unless the host's complete
    /// admission predicate and current native identity permit staging. Always
    /// replace a prior sample; memory is bounded to one copy of scalar data.
    pub fn stage_or_immediate(
        &self,
        values: Values,
        identity: Option<ClockIdentity>,
        may_stage: bool,
    ) -> Option<Values> {
        if !self.enabled {
            return Some(values);
        }
        if may_stage && let Some(identity) = identity {
            let previous = self.pending.replace(Some(Pending { values, identity }));
            let mut stats = self.stats.get();
            stats.staged = stats.staged.saturating_add(1);
            if previous.is_some() {
                stats.overwritten = stats.overwritten.saturating_add(1);
            }
            self.stats.set(stats);
            None
        } else {
            self.invalidate();
            Some(values)
        }
    }

    /// No identity lookup or native snapshot access when the slot is empty.
    /// The value is removed before the closure or any host property setters,
    /// so reentrancy cannot borrow/overwrite an in-flight sample.
    pub fn take_for_render(
        &self,
        current_identity: impl FnOnce() -> Option<ClockIdentity>,
    ) -> Option<Values> {
        let pending = self.pending.take()?;
        let valid = current_identity() == Some(pending.identity);
        let mut stats = self.stats.get();
        if valid {
            stats.applied_in_render = stats.applied_in_render.saturating_add(1);
        } else {
            stats.invalidated = stats.invalidated.saturating_add(1);
        }
        self.stats.set(stats);
        valid.then_some(pending.values)
    }

    /// Use before accepted load/stop/account clear/teardown. Immediate updates
    /// on pause, buffering, seeking, hidden controls or navigation also discard
    /// pending samples through stage_or_immediate(..., false).
    pub fn invalidate(&self) {
        if self.pending.take().is_some() {
            let mut stats = self.stats.get();
            stats.invalidated = stats.invalidated.saturating_add(1);
            self.stats.set(stats);
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(load: u64, epoch: u64) -> Option<ClockIdentity> {
        Some(ClockIdentity {
            load_request_id: load,
            transport_epoch: epoch,
        })
    }
    fn values(position: f64) -> Values {
        Values {
            position,
            duration: 90.,
        }
    }

    #[test]
    fn default_path_is_immediate_and_never_reads_identity_during_render() {
        let state = State::new(false);
        assert_eq!(
            state.stage_or_immediate(values(1.), identity(1, 1), true),
            Some(values(1.))
        );
        assert_eq!(state.take_for_render(|| panic!("empty lookup")), None);
        assert_eq!(state.stats(), Stats::default());
    }

    #[test]
    fn newest_sample_replaces_bounded_slot_and_is_taken_before_host_callback() {
        let state = State::new(true);
        for position in 0..1000 {
            assert_eq!(
                state.stage_or_immediate(values(position as f64), identity(1, 1), true),
                None
            );
        }
        assert_eq!(
            state.take_for_render(|| {
                assert_eq!(state.take_for_render(|| panic!("already taken")), None);
                identity(1, 1)
            }),
            Some(values(999.))
        );
        assert_eq!(state.stats().overwritten, 999);
        assert_eq!(state.stats().applied_in_render, 1);
    }

    #[test]
    fn replacements_transport_changes_and_stop_cannot_publish_prior_clock() {
        for current in [identity(2, 1), identity(1, 2), None] {
            let state = State::new(true);
            state.stage_or_immediate(values(50.), identity(1, 1), true);
            assert_eq!(state.take_for_render(|| current), None);
            assert_eq!(state.stats().invalidated, 1);
            assert_eq!(state.take_for_render(|| panic!("discarded")), None);
        }
    }

    #[test]
    fn transition_applies_latest_value_immediately_and_leaves_no_deferred_work() {
        let state = State::new(true);
        state.stage_or_immediate(values(3.), identity(1, 1), true);
        assert_eq!(
            state.stage_or_immediate(values(20.), identity(1, 2), false),
            Some(values(20.))
        );
        assert_eq!(state.take_for_render(|| panic!("transition drained")), None);
        state.stage_or_immediate(values(21.), identity(1, 2), true);
        state.invalidate();
        assert_eq!(
            state.take_for_render(|| panic!("account clear drained")),
            None
        );
        assert_eq!(
            state.stage_or_immediate(values(0.), None, true),
            Some(values(0.))
        );
    }
}
