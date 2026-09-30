use super::CancellationToken;
use std::{
    future::Future,
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Wake, Waker},
};

#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn cancellation_broadcasts_to_all_clones_without_periodic_wakes() {
    let token = CancellationToken::default();
    let clone = token.clone();
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    let mut first = std::pin::pin!(token.cancelled());
    let mut second = std::pin::pin!(clone.cancelled());
    assert!(first.as_mut().poll(&mut context).is_pending());
    assert!(second.as_mut().poll(&mut context).is_pending());
    assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
    token.cancel();
    assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
    assert!(first.as_mut().poll(&mut context).is_ready());
    assert!(second.as_mut().poll(&mut context).is_ready());
    assert!(clone.is_cancelled());
}

#[test]
fn cancellation_before_creation_or_first_poll_is_permanent() {
    let token = CancellationToken::default();
    let mut unpolled = std::pin::pin!(token.cancelled());
    token.cancel();
    let waker = Waker::from(Arc::new(Wakes::default()));
    let mut context = Context::from_waker(&waker);
    assert!(unpolled.as_mut().poll(&mut context).is_ready());
    assert!(
        std::pin::pin!(token.cancelled())
            .as_mut()
            .poll(&mut context)
            .is_ready()
    );
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn dropping_a_listener_removes_it_without_consuming_another_listeners_wake() {
    let token = CancellationToken::default();
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    let mut abandoned = Box::pin(token.cancelled());
    assert!(abandoned.as_mut().poll(&mut context).is_pending());
    drop(abandoned);
    let mut current = std::pin::pin!(token.cancelled());
    assert!(current.as_mut().poll(&mut context).is_pending());
    token.cancel();
    assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
    assert!(current.as_mut().poll(&mut context).is_ready());
}

#[test]
fn cancellation_racing_with_registration_never_loses_a_wake() {
    for _ in 0..100 {
        let token = CancellationToken::default();
        let clone = token.clone();
        let barrier = Arc::new(Barrier::new(2));
        let other = barrier.clone();
        let thread = std::thread::spawn(move || {
            other.wait();
            clone.cancel();
        });
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(token.cancelled());
        barrier.wait();
        let pending = future.as_mut().poll(&mut context).is_pending();
        thread.join().unwrap();
        if pending {
            assert!(wakes.0.load(Ordering::Relaxed) > 0);
            assert!(future.as_mut().poll(&mut context).is_ready());
        }
    }
}
