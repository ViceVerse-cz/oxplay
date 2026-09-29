// SPDX-License-Identifier: GPL-3.0-or-later
//! One provider operation. Optional artwork yields to foreground work.
use serein_core::{CancellationToken, ProviderError};
use std::sync::{Condvar, Mutex};

struct Active {
    background: bool,
    cancel: CancellationToken,
}
#[derive(Default)]
pub(crate) struct Gate {
    active: Mutex<Option<Active>>,
    released: Condvar,
}
pub(crate) struct Guard<'a>(&'a Gate);
impl Gate {
    pub(crate) fn acquire(
        &self,
        cancel: &CancellationToken,
        background: bool,
    ) -> Result<Guard<'_>, ProviderError> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| ProviderError::ExtractorFailed)?;
        while let Some(operation) = active.as_ref() {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            if background || !operation.background {
                return Err(ProviderError::Busy);
            }
            // Only the bounded supervised artwork helper can hold this slot.
            // Its cancellation kills/reaps the entire tree before Guard drops.
            // Wait on the worker thread without spinning or admitting overlap.
            operation.cancel.cancel();
            // Let concurrency regressions synchronize at the actual preemption
            // boundary without sleeps, polling, or launching a helper.
            #[cfg(test)]
            self.released.notify_all();
            active = self
                .released
                .wait(active)
                .map_err(|_| ProviderError::ExtractorFailed)?;
        }
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        *active = Some(Active {
            background,
            cancel: cancel.clone(),
        });
        Ok(Guard(self))
    }
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.active.lock().unwrap().take();
        self.0.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, mpsc},
        time::{Duration, Instant},
    };

    #[test]
    fn foreground_cancels_background_but_waits_for_its_guard_to_drop() {
        let gate = Arc::new(Gate::default());
        let background_cancel = CancellationToken::default();
        let background = gate.acquire(&background_cancel, true).unwrap();
        let (admitted, receive) = mpsc::channel();
        let foreground_gate = gate.clone();
        let foreground = std::thread::spawn(move || {
            let _guard = foreground_gate
                .acquire(&CancellationToken::default(), false)
                .unwrap();
            admitted.send(()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut active = gate.active.lock().unwrap();
        while !background_cancel.is_cancelled() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "foreground never requested preemption"
            );
            active = gate.released.wait_timeout(active, remaining).unwrap().0;
        }
        assert!(active.as_ref().unwrap().background);
        assert_eq!(receive.try_recv(), Err(mpsc::TryRecvError::Empty));
        drop(active);
        // This is the production supervisor's kill-and-reap completion boundary.
        drop(background);
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        foreground.join().unwrap();
        assert!(gate.active.lock().unwrap().is_none());
    }

    #[test]
    fn optional_artwork_cannot_preempt_foreground_and_cancelled_work_is_rejected() {
        let gate = Gate::default();
        let foreground_cancel = CancellationToken::default();
        let foreground = gate.acquire(&foreground_cancel, false).unwrap();
        assert!(matches!(
            gate.acquire(&CancellationToken::default(), true),
            Err(ProviderError::Busy)
        ));
        assert!(!foreground_cancel.is_cancelled());
        drop(foreground);
        let cancelled = CancellationToken::default();
        cancelled.cancel();
        assert!(matches!(
            gate.acquire(&cancelled, false),
            Err(ProviderError::Cancelled)
        ));
        assert!(gate.active.lock().unwrap().is_none());
    }
}
