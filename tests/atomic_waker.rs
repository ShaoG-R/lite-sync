#![cfg(feature = "loom")]

use lite_sync::atomic_waker::AtomicWaker;
use loom::sync::Arc;
use loom::thread;
use std::sync::Arc as StdArc;
use std::task::{Wake, Waker};

// Basic no-op waker for testing
struct NoopWaker;
impl Wake for NoopWaker {
    fn wake(self: StdArc<Self>) {}
}

fn noop_waker() -> Waker {
    Waker::from(StdArc::new(NoopWaker))
}

#[test]
fn loom_atomic_waker_register_wake() {
    loom::model(|| {
        let atomic_waker = Arc::new(AtomicWaker::new());
        let aw_rx = atomic_waker.clone();
        let aw_tx = atomic_waker.clone();

        let t1 = thread::spawn(move || {
            let waker = noop_waker();
            aw_rx.register(&waker);

            // Try to take it back to verify registration consistency
            // Note: In real usage, usually different threads register and take.
            // But here we just exercise the code paths.
        });

        let t2 = thread::spawn(move || {
            aw_tx.wake();
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}

#[test]
fn loom_atomic_waker_concurrent_register() {
    loom::model(|| {
        let atomic_waker = Arc::new(AtomicWaker::new());
        let aw1 = atomic_waker.clone();
        let aw2 = atomic_waker.clone();

        let t1 = thread::spawn(move || {
            let waker = noop_waker();
            aw1.register(&waker);
        });

        let t2 = thread::spawn(move || {
            let waker = noop_waker();
            aw2.register(&waker);
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}

#[test]
fn loom_atomic_waker_wake_take_race() {
    loom::model(|| {
        let atomic_waker = Arc::new(AtomicWaker::new());
        // Pre-register
        atomic_waker.register(&noop_waker());

        let aw1 = atomic_waker.clone();
        let aw2 = atomic_waker.clone();

        let t1 = thread::spawn(move || {
            aw1.wake();
        });

        let t2 = thread::spawn(move || {
            // take() is akin to wake() but returns the waker
            let _ = aw2.take();
        });

        t1.join().unwrap();
        t2.join().unwrap();
    });
}
