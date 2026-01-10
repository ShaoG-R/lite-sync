#![cfg(feature = "loom")]

use lite_sync::oneshot::lite::channel;
use loom::future::block_on;
use loom::thread;

#[test]
fn loom_oneshot_lite_success() {
    loom::model(|| {
        let (tx, rx) = channel::<()>();

        thread::spawn(move || {
            let _ = tx.send(());
        });

        block_on(async move {
            let _ = rx.await;
        });
    });
}

#[test]
fn loom_oneshot_lite_drop_sender() {
    loom::model(|| {
        let (tx, rx) = channel::<()>();

        thread::spawn(move || {
            drop(tx);
        });

        block_on(async move {
            assert!(rx.await.is_err());
        });
    });
}

#[test]
fn loom_oneshot_lite_close_receiver() {
    loom::model(|| {
        let (tx, mut rx) = channel::<()>();

        thread::spawn(move || {
            // We might fail sending if receiver is closed
            let _ = tx.send(());
        });

        rx.close();
        block_on(async move {
            // Receiver was closed, but we can still await it?
            // Logic in recv: check inner.
            // If closed, returns error hopefully, or pending?
            // Receiver::close() marks closed.
            // Receiver::poll handles closed.
            let _ = rx.await;
        });
    });
}
