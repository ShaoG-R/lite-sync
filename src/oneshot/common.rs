//! Common types and traits shared between oneshot implementations.
//!
//! 一次性通道实现之间共享的公共类型和 trait。

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::atomic_waker::AtomicWaker;

// ============================================================================
// Error Types
// ============================================================================

pub mod error {
    //! Oneshot error types.

    use std::fmt;

    /// Error returned when the sender is dropped before sending a value
    /// 
    /// 当发送器在发送值之前被丢弃时返回的错误
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RecvError;

    impl fmt::Display for RecvError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "channel closed")
        }
    }

    impl std::error::Error for RecvError {}

    /// Error returned from `try_recv` when no value has been sent yet or channel is closed
    /// 
    /// 当尚未发送值或通道已关闭时，`try_recv` 返回的错误
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TryRecvError {
        /// The channel is empty (no value sent yet)
        /// 
        /// 通道为空（尚未发送值）
        Empty,
        /// The sender was dropped without sending a value
        /// 
        /// 发送器在发送值之前被丢弃
        Closed,
    }

    impl fmt::Display for TryRecvError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                TryRecvError::Empty => write!(f, "channel empty"),
                TryRecvError::Closed => write!(f, "channel closed"),
            }
        }
    }

    impl std::error::Error for TryRecvError {}
}

pub use self::error::RecvError;
pub use self::error::TryRecvError;

// ============================================================================
// Storage Trait
// ============================================================================

/// Result of trying to take a value from storage
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeResult<T> {
    /// Value is ready
    Ready(T),
    /// Value is not ready yet (pending)
    Pending,
    /// Sender was dropped without sending
    Closed,
}

impl<T> TakeResult<T> {
    #[inline]
    pub fn ok(self) -> Option<T> {
        match self {
            TakeResult::Ready(v) => Some(v),
            _ => None,
        }
    }
    
    #[inline]
    pub fn is_closed(&self) -> bool {
        matches!(self, TakeResult::Closed)
    }
}

/// Trait for oneshot value storage mechanisms
/// 
/// 一次性值存储机制的 trait
pub trait OneshotStorage: Send + Sync + Sized {
    /// The value type that can be stored
    type Value: Send;
    
    /// Create a new storage instance
    fn new() -> Self;
    
    /// Store a value (called by sender)
    fn store(&self, value: Self::Value);
    
    /// Try to take the stored value (called by receiver)
    fn try_take(&self) -> TakeResult<Self::Value>;
    
    /// Check if the sender was dropped without sending
    fn is_sender_dropped(&self) -> bool;
    
    /// Mark as sender dropped (called in Sender::drop)
    fn mark_sender_dropped(&self);
}

// ============================================================================
// Inner State
// ============================================================================

/// Inner state for oneshot channel
pub struct Inner<S: OneshotStorage> {
    pub(crate) waker: AtomicWaker,
    pub(crate) storage: S,
}

impl<S: OneshotStorage> Inner<S> {
    #[inline]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            waker: AtomicWaker::new(),
            storage: S::new(),
        })
    }
    
    #[inline]
    pub fn send(&self, value: S::Value) {
        self.storage.store(value);
        self.waker.wake();
    }
    
    #[inline]
    pub fn try_recv(&self) -> TakeResult<S::Value> {
        self.storage.try_take()
    }
    
    #[inline]
    pub fn register_waker(&self, waker: &std::task::Waker) {
        self.waker.register(waker);
    }
    
    #[inline]
    pub fn is_sender_dropped(&self) -> bool {
        self.storage.is_sender_dropped()
    }
}

impl<S: OneshotStorage> fmt::Debug for Inner<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inner").finish_non_exhaustive()
    }
}

// ============================================================================
// Sender
// ============================================================================

/// Sender for one-shot value transfer
/// 
/// 一次性值传递的发送器
pub struct Sender<S: OneshotStorage> {
    pub(crate) inner: Arc<Inner<S>>,
}

impl<S: OneshotStorage> fmt::Debug for Sender<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish_non_exhaustive()
    }
}

impl<S: OneshotStorage> Sender<S> {
    /// Create a new oneshot sender with receiver
    #[inline]
    pub fn new() -> (Self, Receiver<S>) {
        let inner = Inner::new();
        let sender = Sender { inner: inner.clone() };
        let receiver = Receiver { inner };
        (sender, receiver)
    }
    
    /// Send a value through the channel
    /// 
    /// Returns `Err(value)` if the receiver was already dropped.
    #[inline]
    pub fn send(self, value: S::Value) -> Result<(), S::Value> {
        if Arc::strong_count(&self.inner) == 1 {
            return Err(value);
        }
        self.send_unchecked(value);
        Ok(())
    }
    
    /// Send a value without checking if receiver is dropped
    /// 
    /// This is faster than `send()` as it skips the Arc reference count check.
    #[inline]
    pub fn send_unchecked(self, value: S::Value) {
        self.inner.send(value);
        std::mem::forget(self);
    }
}

impl<S: OneshotStorage> Drop for Sender<S> {
    fn drop(&mut self) {
        self.inner.storage.mark_sender_dropped();
        self.inner.waker.wake();
    }
}

// ============================================================================
// Receiver
// ============================================================================

/// Receiver for one-shot value transfer
/// 
/// Implements `Future` directly for `.await` usage.
/// 
/// 一次性值传递的接收器
/// 
/// 直接实现了 `Future`，可直接使用 `.await`
pub struct Receiver<S: OneshotStorage> {
    pub(crate) inner: Arc<Inner<S>>,
}

impl<S: OneshotStorage> fmt::Debug for Receiver<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish_non_exhaustive()
    }
}

impl<S: OneshotStorage> Unpin for Receiver<S> {}

impl<S: OneshotStorage> Receiver<S> {
    /// Wait for value asynchronously
    #[inline]
    pub async fn wait(self) -> Result<S::Value, RecvError> {
        self.await
    }
}

impl<S: OneshotStorage> Future for Receiver<S> {
    type Output = Result<S::Value, RecvError>;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        
        // Fast path: check if value ready or closed
        match this.inner.try_recv() {
            TakeResult::Ready(value) => return Poll::Ready(Ok(value)),
            TakeResult::Closed => return Poll::Ready(Err(RecvError)),
            TakeResult::Pending => {}
        }
        
        // Slow path: register waker
        this.inner.register_waker(cx.waker());
        
        // Check again after registering waker
        match this.inner.try_recv() {
            TakeResult::Ready(value) => return Poll::Ready(Ok(value)),
            TakeResult::Closed => return Poll::Ready(Err(RecvError)),
            TakeResult::Pending => {}
        }
        
        // Also check via Arc count (for generic storage)
        if Arc::strong_count(&this.inner) == 1 && this.inner.is_sender_dropped() {
            return Poll::Ready(Err(RecvError));
        }
        
        Poll::Pending
    }
}

/// Create a oneshot channel with the given storage type
#[inline]
pub fn channel<S: OneshotStorage>() -> (Sender<S>, Receiver<S>) {
    Sender::new()
}
