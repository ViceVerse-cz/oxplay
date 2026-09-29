// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded serialized seek confirmation. A queued-command reply alone cannot
//! prove that mpv actually sought; only SEEK followed by RESTART can do that.
use crate::{MediaError, Result};
pub(super) const FIRST: u64 = 1 << 60;
const LIMIT: u64 = 1 << 61;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Context {
    pub load: u64,
    pub entry: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Stale,
    Waiting,
    Completed,
    Failed,
}
struct Pending {
    token: u64,
    context: Context,
    replied: bool,
    seek: bool,
    restarted: bool,
}
pub(super) struct Seeks {
    next: u64,
    pending: Option<Pending>,
}
impl Default for Seeks {
    fn default() -> Self {
        Self {
            next: FIRST,
            pending: None,
        }
    }
}
pub(super) fn owns(token: u64) -> bool {
    (FIRST..LIMIT).contains(&token)
}
impl Seeks {
    pub fn token(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.token)
    }
    pub fn begin(&mut self, context: Context) -> Result<u64> {
        if self.pending.is_some() {
            return Err(MediaError(
                "A seek is still settling; try again shortly".into(),
            ));
        }
        let token = self.next;
        self.next = token
            .checked_add(1)
            .filter(|next| *next < LIMIT)
            .ok_or_else(|| MediaError("Seek identifiers exhausted".into()))?;
        self.pending = Some(Pending {
            token,
            context,
            replied: false,
            seek: false,
            restarted: false,
        });
        Ok(token)
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    pub fn timeout(&mut self, token: u64) -> bool {
        if self.token() != Some(token) {
            return false;
        }
        self.cancel();
        true
    }
    pub fn reply(&mut self, token: u64, success: bool, context: Context) -> Decision {
        let Some(p) = &mut self.pending else {
            return Decision::Stale;
        };
        if p.token != token || p.context != context {
            return Decision::Stale;
        }
        if !success {
            self.cancel();
            return Decision::Failed;
        }
        p.replied = true;
        self.finish()
    }
    pub fn seek(&mut self, context: Context) {
        if let Some(p) = &mut self.pending
            && p.context == context
        {
            p.seek = true;
            p.restarted = false;
        }
    }
    pub fn restart(&mut self, context: Context) -> Decision {
        let Some(p) = &mut self.pending else {
            return Decision::Stale;
        };
        if p.context != context || !p.seek {
            return Decision::Stale;
        }
        p.restarted = true;
        self.finish()
    }
    fn finish(&mut self) -> Decision {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.replied && p.seek && p.restarted)
        {
            self.cancel();
            Decision::Completed
        } else {
            Decision::Waiting
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> Context {
        Context {
            load: 7,
            entry: Some(4),
        }
    }
    #[test]
    fn completion_requires_exact_reply_and_ordered_seek_restart_in_either_reply_order() {
        for reply_first in [false, true] {
            let mut seeks = Seeks::default();
            let c = context();
            let token = seeks.begin(c).unwrap();
            assert!(seeks.begin(c).is_err());
            assert_eq!(seeks.restart(c), Decision::Stale);
            if reply_first {
                assert_eq!(seeks.reply(token, true, c), Decision::Waiting);
            }
            seeks.seek(c);
            assert_eq!(
                seeks.restart(c),
                if reply_first {
                    Decision::Completed
                } else {
                    Decision::Waiting
                }
            );
            if !reply_first {
                assert_eq!(seeks.reply(token, true, c), Decision::Completed);
            }
            assert_eq!(seeks.token(), None);
            assert!(!seeks.timeout(token));
        }
    }
    #[test]
    fn command_failure_no_seek_timeout_and_stale_load_reply_never_wedge_or_settle_a_new_request() {
        let mut seeks = Seeks::default();
        let c = context();
        let failed = seeks.begin(c).unwrap();
        assert_eq!(seeks.reply(failed, false, c), Decision::Failed);
        let timeout = seeks.begin(c).unwrap();
        assert_eq!(seeks.reply(timeout, true, c), Decision::Waiting);
        assert!(seeks.timeout(timeout));
        assert!(!seeks.timeout(timeout));
        let stale = seeks.begin(c).unwrap();
        seeks.cancel();
        let newer = Context {
            load: 8,
            entry: Some(5),
        };
        let token = seeks.begin(newer).unwrap();
        assert_eq!(seeks.reply(stale, false, c), Decision::Stale);
        seeks.seek(c);
        assert_eq!(seeks.restart(c), Decision::Stale);
        assert!(!seeks.timeout(stale));
        assert_eq!(seeks.token(), Some(token));
        assert!(seeks.timeout(token));
    }
    #[test]
    fn overlap_is_rejected_and_cancelled_transports_cannot_complete_or_stop_new_work() {
        let mut seeks = Seeks::default();
        let c = context();
        let first = seeks.begin(c).unwrap();
        for _ in 0..100 {
            assert!(seeks.begin(c).is_err());
        }
        assert_eq!(seeks.token(), Some(first));
        // begin also rejects an absolute request while a relative owns admission.
        // Only load/stop/END cancellation relinquishes that transport early.
        // Old events/replies/timers then have no owner until new work is admitted.
        seeks.cancel();
        seeks.seek(c);
        assert_eq!(seeks.restart(c), Decision::Stale);
        assert_eq!(seeks.reply(first, true, c), Decision::Stale);
        assert!(!seeks.timeout(first));
        let second = seeks.begin(c).unwrap();
        assert_ne!(first, second);
        assert_eq!(seeks.reply(first, false, c), Decision::Stale);
        assert!(!seeks.timeout(first));
        assert_eq!(seeks.token(), Some(second));
    }
}
