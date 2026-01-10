#![cfg(feature = "loom")]

use lite_sync::oneshot::lite::channel;
use loom::thread;

#[test]
fn loom_oneshot_blocking_recv_success() {
    loom::model(|| {
        let (tx, rx) = channel::<()>();

        thread::spawn(move || {
            let _ = tx.send(());
        });

        assert_eq!(rx.blocking_recv(), Ok(()));
    });
}

#[test]
fn loom_oneshot_blocking_recv_drop_sender() {
    loom::model(|| {
        let (tx, rx) = channel::<()>();

        thread::spawn(move || {
            drop(tx);
        });

        assert!(rx.blocking_recv().is_err());
    });
}

#[test]
fn loom_sender_is_closed_concurrent() {
    loom::model(|| {
        let (tx, rx) = channel::<()>();

        let tx = loom::sync::Arc::new(tx);
        let t_rx = thread::spawn(move || {
            drop(rx);
        });

        // Concurrent check
        let is_closed = tx.is_closed();

        t_rx.join().unwrap();
        // Just ensuring no panic and partial orderings.
        // is_closed result depends on interleaving.
        let _ = is_closed;
    });
}
