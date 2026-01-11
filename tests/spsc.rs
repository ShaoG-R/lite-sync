#![cfg(feature = "loom")]

use lite_sync::spsc::channel;
use loom::future::block_on;
use loom::thread;
use std::num::NonZeroUsize;

#[test]
fn loom_spsc_simple_send_recv() {
    loom::model(|| {
        let (tx, rx) = channel::<usize, 4>(NonZeroUsize::new(2).unwrap());

        thread::spawn(move || {
            block_on(async move {
                tx.send(1).await.unwrap();
                tx.send(2).await.unwrap();
            });
        });

        block_on(async move {
            assert_eq!(rx.recv().await.unwrap(), 1);
            assert_eq!(rx.recv().await.unwrap(), 2);
        });
    });
}

#[test]
fn loom_spsc_backpressure() {
    loom::model(|| {
        // Capacity 1
        let (tx, rx) = channel::<usize, 4>(NonZeroUsize::new(1).unwrap());

        let tx_thread = thread::spawn(move || {
            block_on(async move {
                tx.send(1).await.unwrap();
                // This triggers wait for space
                tx.send(2).await.unwrap();
            });
        });

        block_on(async move {
            let v1 = rx.recv().await.unwrap();
            assert_eq!(v1, 1);
            // After v1 is received, space becomes available, tx moves to send 2
            let v2 = rx.recv().await.unwrap();
            assert_eq!(v2, 2);
        });

        tx_thread.join().unwrap();
    });
}

#[test]
fn loom_spsc_close_sender() {
    loom::model(|| {
        let (tx, rx) = channel::<usize, 4>(NonZeroUsize::new(2).unwrap());

        thread::spawn(move || {
            block_on(async move {
                tx.send(100).await.unwrap();
                // tx dropped here
            });
        });

        block_on(async move {
            assert_eq!(rx.recv().await.unwrap(), 100);
            assert!(rx.recv().await.is_none());
        });
    });
}

#[test]
fn loom_spsc_close_receiver() {
    loom::model(|| {
        // Capacity 1
        let (tx, rx) = channel::<usize, 4>(NonZeroUsize::new(1).unwrap());

        // Fill the buffer so next send blocks
        tx.try_send(1).unwrap();

        let tx_thread = thread::spawn(move || {
            block_on(async move {
                // This must block because buffer is full (capacity 1, has 1)
                // When rx is dropped, this should wake up and error.
                assert!(tx.send(2).await.is_err());
            });
        });

        // Give tx thread a chance to run and block
        thread::yield_now();

        drop(rx);

        tx_thread.join().unwrap();
    });
}
