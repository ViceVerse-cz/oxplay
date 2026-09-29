//! Nonsecret authority for one verified account generation. No credential or URL storage.
use super::AccountError;
use std::{
    fmt,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Instant, SystemTime},
};
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct Control {
    pub generation: AtomicU64,
    playback: Mutex<Option<Weak<LeaseState>>>,
}
impl Control {
    pub fn invalidate(&self) -> u64 {
        // Serialize registration with revocation: no lease can escape the
        // generation check between checking and registering its weak reference.
        let mut playback = self.playback.lock().unwrap();
        let next = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Some(lease) = playback.take().and_then(|lease| lease.upgrade()) {
            lease.revoke();
        }
        next
    }
    pub fn issue(
        &self,
        generation: u64,
        expires_at: Option<SystemTime>,
    ) -> Result<AccountPlaybackLease, AccountError> {
        let mut playback = self.playback.lock().unwrap();
        if self.generation.load(Ordering::Acquire) != generation {
            return Err(AccountError::StaleSession);
        }
        // One immutable authority per imported session. Replacement resolution
        // must not revoke the stream still playing while its native position is
        // collected. Cookie replacement requires a new account generation.
        if let Some(existing) = playback.as_ref().and_then(Weak::upgrade) {
            let lease = AccountPlaybackLease(existing);
            if !lease.is_valid() {
                return Err(AccountError::SessionExpired);
            }
            if lease.0.expires_at != expires_at {
                return Err(AccountError::InvalidInput);
            }
            return Ok(lease);
        }
        let deadline = expires_at
            .map(|expiry| {
                let remaining = expiry
                    .duration_since(SystemTime::now())
                    .map_err(|_| AccountError::SessionExpired)?;
                Instant::now()
                    .checked_add(remaining)
                    .ok_or(AccountError::InvalidCookieFile)
            })
            .transpose()?;
        let lease = AccountPlaybackLease(Arc::new(LeaseState {
            revoked: AtomicBool::new(false),
            wake: Notify::new(),
            expires_at,
            deadline,
            generation,
        }));
        *playback = Some(Arc::downgrade(&lease.0));
        Ok(lease)
    }
}
struct LeaseState {
    generation: u64,
    revoked: AtomicBool,
    wake: Notify,
    expires_at: Option<SystemTime>,
    deadline: Option<Instant>,
}
impl LeaseState {
    fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
}
/// Revocable, nonsecret authorization. It authorizes no cookie/header forwarding.
/// The media transport must retain its independent URL/header/redirect policies.
#[derive(Clone)]
pub struct AccountPlaybackLease(Arc<LeaseState>);
impl fmt::Debug for AccountPlaybackLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountPlaybackLease([nonsecret revocable authority])")
    }
}
impl AccountPlaybackLease {
    pub fn generation(&self) -> u64 {
        self.0.generation
    }
    /// Remaining known lifetime for an event-loop single-shot timer. None means
    /// a currently valid session cookie has no known expiry, never an expired
    /// or revoked lease. No runtime, filesystem access or polling is required.
    pub fn valid_for(&self) -> Option<std::time::Duration> {
        if !self.is_valid() {
            return Some(std::time::Duration::ZERO);
        }
        self.0
            .deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }
    pub fn is_valid(&self) -> bool {
        if self
            .0
            .expires_at
            .is_some_and(|expiry| SystemTime::now() >= expiry)
            || self
                .0
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.revoke();
        }
        !self.0.revoked.load(Ordering::Acquire)
    }
    /// No task or timer exists unless a consumer awaits revocation. Register
    /// before checking state so synchronous invalidate cannot lose a wakeup.
    pub async fn revoked(&self) {
        let notified = self.0.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_valid() {
            return;
        }
        if let Some(deadline) = self.0.deadline {
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep_until(deadline.into()) => self.revoke(),
            }
        } else {
            notified.await;
        }
    }
    pub(super) fn revoke(&self) {
        self.0.revoke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    #[test]
    fn lease_is_nonsecret_send_sync_and_stale_generation_cannot_issue() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<AccountPlaybackLease>();
        let control = Control::default();
        let lease = control.issue(0, None).unwrap();
        assert!(lease.is_valid());
        assert_eq!(control.invalidate(), 1);
        assert!(!lease.is_valid());
        assert!(matches!(
            control.issue(0, None),
            Err(AccountError::StaleSession)
        ));
        let next = control.issue(1, None).unwrap();
        assert!(next.is_valid());
        assert!(!lease.is_valid());
        assert!(!format!("{next:?}").contains("https"));
    }
    #[test]
    fn remaining_lifetime_distinguishes_live_unknown_known_and_revoked() {
        let control = Control::default();
        let unknown = control.issue(0, None).unwrap();
        assert_eq!(unknown.valid_for(), None);
        control.invalidate();
        let known = control
            .issue(1, Some(SystemTime::now() + Duration::from_secs(5)))
            .unwrap();
        let first = known.valid_for().unwrap();
        assert!(first > Duration::ZERO && first <= Duration::from_secs(5));
        assert!(known.valid_for().unwrap() <= first);
        assert_eq!(unknown.valid_for(), Some(Duration::ZERO));
        control.invalidate();
        assert_eq!(known.valid_for(), Some(Duration::ZERO));
    }
    #[test]
    fn repeated_issue_preserves_playing_authority_and_cannot_extend_its_expiry() {
        let control = Control::default();
        let expiry = Some(SystemTime::now() + Duration::from_secs(30));
        let playing = control.issue(0, expiry).unwrap();
        let replacement = control.issue(0, expiry).unwrap();
        assert!(Arc::ptr_eq(&playing.0, &replacement.0));
        assert!(playing.is_valid() && replacement.is_valid());
        assert!(matches!(
            control.issue(0, None),
            Err(AccountError::InvalidInput)
        ));
        assert!(playing.is_valid() && replacement.is_valid());
        control.invalidate();
        assert!(!playing.is_valid() && !replacement.is_valid());
        assert!(matches!(
            control.issue(0, expiry),
            Err(AccountError::StaleSession)
        ));
    }
    #[test]
    fn invalidation_wakes_every_registered_waiter_and_late_waiter() {
        runtime().block_on(async {
            let control = Control::default();
            let lease = control.issue(0, None).unwrap();
            let mut waiters = Vec::new();
            for _ in 0..4 {
                let lease = lease.clone();
                waiters.push(tokio::spawn(async move {
                    lease.revoked().await;
                }));
            }
            tokio::task::yield_now().await;
            control.invalidate();
            for waiter in waiters {
                tokio::time::timeout(Duration::from_secs(1), waiter)
                    .await
                    .unwrap()
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(1), lease.revoked())
                .await
                .unwrap();
        });
    }
    #[test]
    fn revoke_between_future_creation_and_first_poll_is_not_lost() {
        runtime().block_on(async {
            let control = Control::default();
            let lease = control.issue(0, None).unwrap();
            let future = lease.revoked();
            control.invalidate();
            tokio::time::timeout(Duration::from_secs(1), future)
                .await
                .unwrap();
        });
    }
    #[test]
    fn finite_expiry_wakes_without_polling_or_changing_account_generation() {
        runtime().block_on(async {
            let control = Control::default();
            let lease = control
                .issue(0, Some(SystemTime::now() + Duration::from_millis(20)))
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), lease.revoked())
                .await
                .unwrap();
            assert!(!lease.is_valid());
            assert_eq!(lease.valid_for(), Some(Duration::ZERO));
            assert_eq!(control.generation.load(Ordering::Acquire), 0);
            lease.revoked().await;
        });
    }
    #[test]
    fn weak_registration_does_not_keep_a_session_lease_alive() {
        let control = Control::default();
        let lease = control.issue(0, None).unwrap();
        let weak = Arc::downgrade(&lease.0);
        drop(lease);
        assert!(weak.upgrade().is_none());
    }
}
