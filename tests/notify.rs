#![cfg(feature = "loom")]

use lite_sync::notify::SingleWaiterNotify;
use loom::future::block_on;
use loom::sync::Arc;
use loom::thread;

#[test]
fn loom_notify_basic() {
    loom::model(|| {
        let notify = Arc::new(SingleWaiterNotify::new());
        let notify_rx = notify.clone();
        let notify_tx = notify.clone();

        thread::spawn(move || {
            notify_tx.notify_one();
        });

        block_on(async move {
            notify_rx.notified().await;
        });
    });
}

#[test]
fn loom_notify_concurrent_notify() {
    loom::model(|| {
        let notify = Arc::new(SingleWaiterNotify::new());
        let rx = notify.clone();
        let tx1 = notify.clone();
        let tx2 = notify.clone();

        thread::spawn(move || {
            tx1.notify_one();
        });

        thread::spawn(move || {
            tx2.notify_one();
        });

        block_on(async move {
            rx.notified().await;
        });
    });
}

#[test]
fn loom_notify_drop_safety() {
    loom::model(|| {
        let notify = Arc::new(SingleWaiterNotify::new());
        let rx = notify.clone();

        let tx = notify.clone();
        thread::spawn(move || {
            tx.notify_one();
        });

        block_on(async move {
            // Start waiting but cancel (by dropping future indirectly/completing)
            // Here we just complete, but model checks memory safety.
            rx.notified().await;
        });
    });
}
