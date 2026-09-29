// SPDX-License-Identifier: GPL-3.0-or-later
//! One Off barrier: pending additions -> set reply -> fresh sid query.
//! libmpv client.h permits async reordering; submission alone proves nothing.
use crate::{MediaError, Result};
pub(super) const FIRST: u64 = 1 << 58;
const LIMIT: u64 = 1 << 59;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Context {
    pub load: u64,
    pub generation: u64,
    pub entry: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Adds,
    Command,
    Query,
}
struct Pending {
    token: u64,
    context: Context,
    phase: Phase,
    cancelled: bool,
}
pub(super) struct Barrier {
    next: u64,
    pending: Option<Pending>,
    reply: Option<(u64, bool)>,
}
impl Default for Barrier {
    fn default() -> Self {
        Self {
            next: FIRST,
            pending: None,
            reply: None,
        }
    }
}
pub(super) fn owns(token: u64) -> bool {
    (FIRST..LIMIT).contains(&token)
}
impl Barrier {
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn reply(&self) -> Option<(u64, bool)> {
        self.reply
    }
    pub fn begin(&mut self, context: Context) -> Result<u64> {
        if let Some(pending) = &self.pending {
            return if !pending.cancelled && pending.context == context {
                Ok(pending.token)
            } else {
                Err(MediaError("A caption change is still settling".into()))
            };
        }
        let token = self.next;
        self.next = token
            .checked_add(2)
            .filter(|next| *next < LIMIT)
            .ok_or_else(|| MediaError("Caption identifiers exhausted".into()))?;
        self.pending = Some(Pending {
            token,
            context,
            phase: Phase::Adds,
            cancelled: false,
        });
        self.reply = None;
        Ok(token)
    }
    pub fn invalidate(&mut self) {
        self.reply = None;
        if let Some(pending) = &mut self.pending {
            pending.cancelled = true;
            if pending.phase == Phase::Adds {
                self.pending = None;
            }
        }
    }
    pub fn command(&mut self, context: Context, adds_pending: bool) -> Option<u64> {
        let pending = self.pending.as_mut()?;
        if pending.context != context {
            self.invalidate();
            return None;
        }
        if pending.cancelled || pending.phase != Phase::Adds || adds_pending {
            return None;
        }
        pending.phase = Phase::Command;
        Some(pending.token)
    }
    pub fn command_reply(&mut self, token: u64, success: bool, context: Context) -> Option<u64> {
        let pending = self.pending.as_mut()?;
        if pending.token != token || pending.phase != Phase::Command {
            return None;
        }
        if pending.cancelled || pending.context != context {
            self.pending = None;
            return None;
        }
        if !success {
            self.fail(token);
            return None;
        }
        pending.phase = Phase::Query;
        Some(token + 1)
    }
    pub fn query_reply(&mut self, token: u64, off: bool, context: Context) {
        let Some(pending) = &self.pending else { return };
        if pending.token + 1 != token || pending.phase != Phase::Query {
            return;
        }
        if !pending.cancelled && pending.context == context {
            self.reply = Some((pending.token, off));
        }
        self.pending = None;
    }
    pub fn fail(&mut self, token: u64) {
        if let Some(pending) = &self.pending
            && (pending.token == token || pending.token + 1 == token)
        {
            if !pending.cancelled {
                self.reply = Some((pending.token, false));
            }
            self.pending = None;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn context(load: u64) -> Context {
        Context {
            load,
            generation: load,
            entry: Some(load as i64),
        }
    }
    #[test]
    fn pending_addition_and_old_off_observation_cannot_complete_barrier() {
        let mut b = Barrier::default();
        let c = context(1);
        let token = b.begin(c).unwrap();
        assert_eq!(b.begin(c).unwrap(), token);
        assert_eq!(b.command(c, true), None);
        assert_eq!(b.reply(), None);
        b.query_reply(token + 1, true, c);
        assert!(b.pending());
        assert_eq!(b.command(c, false), Some(token));
        assert_eq!(b.command(c, false), None);
        assert_eq!(b.command_reply(token, true, c), Some(token + 1));
        assert_eq!(b.reply(), None);
        b.query_reply(token + 1, true, c);
        assert_eq!(b.reply(), Some((token, true)));
        assert!(!b.pending());
    }
    #[test]
    fn cancellation_keeps_native_slot_until_reply_and_rejects_stale_completion() {
        for at_query in [false, true] {
            let mut b = Barrier::default();
            let c = context(1);
            let token = b.begin(c).unwrap();
            b.command(c, false);
            if at_query {
                b.command_reply(token, true, c);
            }
            b.invalidate();
            assert!(b.begin(context(2)).is_err());
            if at_query {
                b.query_reply(token + 1, true, context(2));
            } else {
                assert_eq!(b.command_reply(token, true, context(2)), None);
            }
            assert!(!b.pending());
            assert_eq!(b.reply(), None);
            let next = b.begin(context(2)).unwrap();
            assert_ne!(next, token);
            b.query_reply(token + 1, true, c);
            assert!(b.pending());
            assert_eq!(b.reply(), None);
        }
    }
    #[test]
    fn submission_failure_command_failure_and_nonoff_query_are_terminal() {
        for stage in 0..3 {
            let mut b = Barrier::default();
            let c = context(1);
            let token = b.begin(c).unwrap();
            b.command(c, false);
            match stage {
                0 => b.fail(token),
                1 => {
                    b.command_reply(token, false, c);
                }
                _ => {
                    b.command_reply(token, true, c);
                    b.query_reply(token + 1, false, c);
                }
            }
            assert_eq!(b.reply(), Some((token, false)));
            assert!(!b.pending());
        }
        let mut b = Barrier::default();
        b.begin(context(1)).unwrap();
        b.invalidate();
        assert!(!b.pending());
        assert_eq!(b.reply(), None);
    }
}
