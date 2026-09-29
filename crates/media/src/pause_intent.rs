// SPDX-License-Identifier: GPL-3.0-or-later
//! One serialized pause write; observed pause is never substituted for intent.
use crate::{MediaError, Result};
const FIRST: u64 = 1 << 59;
const LIMIT: u64 = 1 << 60;

#[derive(Clone, Copy, Debug)]
pub(super) struct Command {
    pub token: u64,
    pub paused: bool,
    revision: u64,
    generation: u64,
}
#[derive(Default)]
pub(super) struct PauseIntent {
    user: Option<bool>,
    observed: bool,
    hidden: bool,
    applied: Option<bool>,
    revision: u64,
    serial: u64,
    pending: Option<Command>,
    blocked: bool,
    generation: u64,
    eof_seen: bool,
    intent_epoch: u64,
    eof_candidate: Option<(u64, u64)>,
    eof_probe: Option<(u64, u64)>,
}
pub(super) fn owns(token: u64) -> bool {
    (FIRST..LIMIT).contains(&token)
}
impl PauseIntent {
    pub fn desired(&self) -> bool {
        self.user.unwrap_or(self.observed)
    }
    pub fn effective(&self) -> bool {
        self.hidden || self.desired()
    }
    pub fn settled(&self, observed: bool) -> bool {
        self.pending.is_none()
            && !self.blocked
            && !self.eof_unsettled()
            && (self.user.is_none()
                || (self.applied == Some(self.effective()) && observed == self.effective()))
    }
    pub fn observe(&mut self, value: bool) {
        self.observed = value;
        // Late native observations cannot overwrite an accepted user command.
        if self.user.is_none() && self.pending.is_none() && !self.hidden {
            self.applied = Some(value);
        }
    }
    pub fn user(&mut self, value: bool) {
        // A native keep-open hold can differ from the last acknowledged write.
        // Even an explicit same-value request must reach mpv after an EOF edge.
        if self.eof_seen {
            self.applied = None;
        }
        self.user = Some(value);
        self.revision = self.revision.saturating_add(1);
        self.blocked = false;
        self.invalidate_eof();
    }
    pub fn new_load(&mut self, value: bool) {
        self.generation = self.generation.saturating_add(1);
        self.eof_seen = false;
        self.user(value);
        self.applied = None;
    }
    /// Native notifications may precede an earlier command's queued reply.
    /// Confirm EOF after that write finishes instead of guessing its ordering.
    pub fn eof(&mut self, value: bool) {
        if !value {
            if self.eof_seen {
                self.invalidate_eof();
            }
            self.eof_seen = false;
            self.eof_candidate = None;
            return;
        }
        if self.eof_seen {
            return;
        }
        self.eof_seen = true;
        if !self.blocked {
            self.eof_candidate = Some((self.generation, self.intent_epoch));
        }
    }
    pub fn invalidate_eof(&mut self) {
        self.intent_epoch = self.intent_epoch.saturating_add(1);
        self.eof_candidate = None;
        // Retain actual query ownership until its reply; newer work cannot
        // accumulate queries if cancellation repeatedly wins the race.
    }
    pub fn eof_unsettled(&self) -> bool {
        let current = Some((self.generation, self.intent_epoch));
        self.eof_candidate == current || self.eof_probe == current
    }
    pub fn prepare_eof_probe(&mut self) -> bool {
        if self.blocked || self.pending.is_some() || self.eof_probe.is_some() {
            return false;
        }
        self.eof_probe = self.eof_candidate.take();
        self.eof_probe.is_some()
    }
    /// None is stale; Some(false) is a current non-EOF reply; Some(true) adopts
    /// the native keep-open hold. A failed current read is a recoverable error.
    pub fn eof_reply(&mut self, value: Option<bool>) -> Result<Option<bool>> {
        let probe = self.eof_probe.take();
        if probe != Some((self.generation, self.intent_epoch)) || self.blocked {
            return Ok(None);
        }
        let value =
            value.ok_or_else(|| MediaError("Could not confirm native end-of-file pause".into()))?;
        if value {
            self.user = Some(true);
            self.revision = self.revision.saturating_add(1);
            self.applied = None;
        }
        Ok(Some(value))
    }
    pub fn occluded(&mut self, value: bool) -> bool {
        if self.hidden == value {
            return false;
        }
        // Freeze the last observed user value before policy changes the native
        // property. Occlusion-generated pause must not become user intent.
        if self.user.is_none() {
            self.user = Some(self.observed);
        }
        self.hidden = value;
        self.revision = self.revision.saturating_add(1);
        true
    }
    pub fn stop(&mut self) {
        self.user = Some(true);
        self.revision = self.revision.saturating_add(1);
        self.blocked = true;
        self.invalidate_eof();
        // Keep pending ownership until its actual reply; preserve hidden state.
    }
    pub fn prepare(&self) -> Result<Option<Command>> {
        if self.blocked || self.pending.is_some() || self.applied == Some(self.effective()) {
            return Ok(None);
        }
        let token = FIRST
            .checked_add(self.serial)
            .filter(|v| *v < LIMIT)
            .ok_or_else(|| MediaError("Pause request identifiers exhausted".into()))?;
        Ok(Some(Command {
            token,
            paused: self.effective(),
            revision: self.revision,
            generation: self.generation,
        }))
    }
    pub fn submitted(&mut self, command: Command) {
        self.serial += 1;
        self.pending = Some(command);
    }
    /// True only for a failure of the latest intent. Caller stops playback;
    /// there is no retry timer and no automatic retry of a failed current write.
    pub fn reply(&mut self, token: u64, success: bool) -> bool {
        let Some(command) = self.pending.filter(|c| c.token == token) else {
            return false;
        };
        self.pending = None;
        self.applied =
            (success && command.generation == self.generation && command.revision == self.revision)
                .then_some(command.paused);
        if !success && command.revision == self.revision && !self.blocked {
            self.stop();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn submit(p: &mut PauseIntent) -> Command {
        let command = p.prepare().unwrap().unwrap();
        p.submitted(command);
        command
    }
    #[test]
    fn queued_user_pause_survives_duplicate_hide_show_and_late_observation() {
        let mut p = PauseIntent::default();
        p.observe(false);
        p.user(true);
        let command = submit(&mut p);
        assert!(p.occluded(true));
        assert!(!p.occluded(true));
        p.observe(false);
        assert!(p.occluded(false));
        assert!(!p.occluded(false));
        assert!(p.desired());
        assert!(p.prepare().unwrap().is_none());
        assert!(!p.reply(command.token, true));
        assert!(submit(&mut p).paused); // latest policy revision is applied once
    }
    #[test]
    fn hidden_toggle_coalesces_and_stop_preserves_window_policy_across_reload() {
        let mut p = PauseIntent::default();
        p.observe(false);
        p.occluded(true);
        let command = submit(&mut p);
        p.user(true);
        p.user(false);
        assert!(!p.desired());
        assert!(p.effective());
        p.stop();
        p.new_load(false); // a new explicit load while still hidden
        assert!(p.effective());
        assert!(!p.reply(command.token, true));
        let reload = submit(&mut p);
        assert!(reload.paused);
        p.reply(reload.token, true);
        p.occluded(false);
        let resume = submit(&mut p);
        assert!(!resume.paused);
    }
    #[test]
    fn old_failure_cannot_cancel_new_intent_but_current_failure_blocks_retries() {
        let mut p = PauseIntent::default();
        p.user(false);
        let old = submit(&mut p);
        p.user(true);
        assert!(!p.reply(old.token, false));
        let newest = submit(&mut p);
        assert!(newest.paused);
        assert!(!p.reply(old.token, true)); // stale duplicate cannot release slot
        assert!(p.prepare().unwrap().is_none());
        assert!(p.reply(newest.token, false));
        assert!(p.desired());
        p.occluded(true);
        p.occluded(false);
        assert!(p.prepare().unwrap().is_none());
        p.user(false);
        assert!(p.prepare().unwrap().is_some());
    }
    #[test]
    fn two_rapid_toggles_are_serialized_explicit_values() {
        let mut p = PauseIntent::default();
        p.observe(false);
        p.user(!p.desired());
        let first = submit(&mut p);
        assert!(first.paused);
        p.user(!p.desired());
        assert!(!p.desired());
        assert!(p.prepare().unwrap().is_none());
        p.reply(first.token, true);
        let second = submit(&mut p);
        assert!(!second.paused);
    }
    #[test]
    fn command_reply_without_matching_observation_is_not_a_settled_pause() {
        let mut p = PauseIntent::default();
        p.observe(false);
        p.user(true);
        let command = submit(&mut p);
        assert!(!p.settled(false));
        p.reply(command.token, true);
        assert!(!p.settled(false));
        assert!(p.settled(true));
    }
    #[test]
    fn eof_hold_is_adopted_once_and_two_rapid_play_toggles_preserve_latest_intent() {
        let mut p = PauseIntent::default();
        p.new_load(false);
        let play = submit(&mut p);
        p.reply(play.token, true);
        p.eof(true);
        assert!(p.prepare_eof_probe());
        assert_eq!(p.eof_reply(Some(true)).unwrap(), Some(true));
        assert!(p.desired());
        let hold = submit(&mut p);
        p.reply(hold.token, true);
        p.user(!p.desired());
        let resume = submit(&mut p);
        assert!(!resume.paused);
        p.eof(true);
        assert!(!p.prepare_eof_probe());
        p.user(!p.desired());
        assert!(p.desired());
        p.reply(resume.token, true);
        assert!(submit(&mut p).paused);
    }
    #[test]
    fn eof_cannot_override_pending_user_write_and_old_load_reply_cannot_confirm_new_pause() {
        let mut p = PauseIntent::default();
        p.new_load(false);
        let old = submit(&mut p);
        p.eof(true);
        assert!(!p.prepare_eof_probe());
        assert!(!p.desired());
        p.new_load(false);
        p.reply(old.token, true);
        let new = submit(&mut p);
        assert!(!new.paused);
        assert_ne!(old.token, new.token);
    }
    #[test]
    fn eof_before_old_reply_is_freshly_confirmed_and_queries_survive_cancellation_bounded() {
        let mut p = PauseIntent::default();
        p.new_load(false);
        let play = submit(&mut p);
        p.eof(true);
        assert!(!p.prepare_eof_probe());
        p.reply(play.token, true);
        assert!(p.prepare_eof_probe());
        assert_eq!(p.eof_reply(Some(true)).unwrap(), Some(true));
        assert!(p.desired());
        let hold = submit(&mut p);
        p.reply(hold.token, true);
        p.eof(false);
        p.eof(true);
        assert!(p.prepare_eof_probe());
        p.user(false);
        p.eof(false);
        p.eof(true);
        assert!(!p.prepare_eof_probe());
        assert_eq!(p.eof_reply(Some(true)).unwrap(), None);
        assert!(!p.desired());
        assert!(p.prepare_eof_probe());
        assert_eq!(p.eof_reply(Some(false)).unwrap(), Some(false));
        assert!(!p.desired());
    }
    #[test]
    fn explicit_same_value_at_unconfirmed_eof_is_not_swallowed_by_applied_cache() {
        let mut p = PauseIntent::default();
        p.new_load(false);
        let play = submit(&mut p);
        p.reply(play.token, true);
        p.eof(true);
        p.user(false);
        assert!(!p.eof_unsettled());
        assert!(!submit(&mut p).paused);
    }
}
