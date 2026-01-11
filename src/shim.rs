//! Shim module to abstract over std and loom primitives.
//!
//! This module provides a unified interface for synchronization primitives that transparently
//! switches between `std` implementation (for production) and `loom` implementation (for testing).

#[cfg(not(feature = "loom"))]
pub mod atomic {
    pub use std::sync::atomic::*;
}

#[cfg(feature = "loom")]
pub mod atomic {
    pub use loom::sync::atomic::*;
}

#[cfg(not(feature = "loom"))]
pub mod cell {
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct UnsafeCell<T: ?Sized>(std::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        #[inline]
        pub const fn new(data: T) -> UnsafeCell<T> {
            UnsafeCell(std::cell::UnsafeCell::new(data))
        }
    }

    impl<T: ?Sized> UnsafeCell<T> {
        #[inline]
        pub fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*const T) -> R,
        {
            f(self.0.get())
        }

        #[inline]
        pub fn with_mut<F, R>(&self, f: F) -> R
        where
            F: FnOnce(*mut T) -> R,
        {
            f(self.0.get())
        }
    }
}

#[cfg(feature = "loom")]
pub mod cell {
    pub use loom::cell::UnsafeCell;
}

#[cfg(not(feature = "loom"))]
pub mod sync {
    pub use std::sync::Arc;
}

#[cfg(feature = "loom")]
pub mod sync {
    pub use loom::sync::Arc;
}

#[cfg(not(feature = "loom"))]
pub mod thread {
    pub use std::thread::{Thread, current, park};
}

#[cfg(feature = "loom")]
pub mod thread {
    pub use loom::thread::{Thread, current, park};
}

#[cfg(not(feature = "loom"))]
pub mod notify {
    pub use crate::notify::SingleWaiterNotify;
}

#[cfg(feature = "loom")]
pub mod notify {
    use loom::sync::Mutex;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    #[derive(Debug, Default)]
    struct State {
        notified: bool,
        waker: Option<Waker>,
    }

    #[derive(Debug)]
    pub struct SingleWaiterNotify {
        inner: Mutex<State>,
    }

    impl Default for SingleWaiterNotify {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SingleWaiterNotify {
        pub fn new() -> Self {
            Self {
                inner: Mutex::new(State::default()),
            }
        }

        pub fn notify_one(&self) {
            let mut state = self.inner.lock().unwrap();
            state.notified = true;
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }

        pub fn notified(&self) -> Notified<'_> {
            Notified { notify: self }
        }
    }

    pub struct Notified<'a> {
        notify: &'a SingleWaiterNotify,
    }

    impl Future for Notified<'_> {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let mut state = self.notify.inner.lock().unwrap();
            if state.notified {
                state.notified = false;
                Poll::Ready(())
            } else {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}
