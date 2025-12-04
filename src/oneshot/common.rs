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
    
    /// Check if the receiver was closed
    /// 
    /// 检查接收器是否已关闭
    fn is_receiver_closed(&self) -> bool;
    
    /// Mark as receiver closed (called in Receiver::close)
    /// 
    /// 标记接收器已关闭
    fn mark_receiver_closed(&self);
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
    /// Returns `Err(value)` if the receiver was already dropped or closed.
    #[inline]
    pub fn send(self, value: S::Value) -> Result<(), S::Value> {
        if self.is_closed() {
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
    
    /// Check if the receiver has been closed or dropped
    /// 
    /// 检查接收器是否已关闭或丢弃
    #[inline]
    pub fn is_closed(&self) -> bool {
        // Receiver is closed if it was explicitly closed or dropped (Arc count == 1)
        self.inner.storage.is_receiver_closed() || Arc::strong_count(&self.inner) == 1
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
    
    /// Close the receiver, preventing any future messages from being sent.
    /// 
    /// Any `send` operation which happens after this method returns is guaranteed
    /// to fail. After calling `close`, `try_recv` should be called to receive
    /// a value if one was sent before the call to `close` completed.
    /// 
    /// 关闭接收器，阻止任何将来的消息发送。
    /// 
    /// 在此方法返回后发生的任何 `send` 操作都保证失败。
    /// 调用 `close` 后，应调用 `try_recv` 来接收在 `close` 完成之前发送的值。
    #[inline]
    pub fn close(&mut self) {
        self.inner.storage.mark_receiver_closed();
    }
    
    /// Blocking receive, waiting for a value to be sent.
    /// 
    /// This method is intended for use in synchronous code.
    /// 
    /// # Panics
    /// 
    /// This function panics if called within an asynchronous execution context.
    /// 
    /// 阻塞接收，等待值被发送。
    /// 
    /// 此方法用于同步代码中。
    /// 
    /// # Panics
    /// 
    /// 如果在异步执行上下文中调用此函数，则会 panic。
    #[inline]
    pub fn blocking_recv(self) -> Result<S::Value, RecvError> {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::task::{RawWaker, RawWakerVTable, Waker};
        
        // Fast path: check if value is already ready
        match self.inner.storage.try_take() {
            TakeResult::Ready(value) => return Ok(value),
            TakeResult::Closed => return Err(RecvError),
            TakeResult::Pending => {}
        }
        
        // Create a thread-parker waker
        struct ThreadParker {
            thread: std::thread::Thread,
            notified: AtomicBool,
        }
        
        const VTABLE: RawWakerVTable = RawWakerVTable::new(
            |ptr| unsafe { 
                Arc::increment_strong_count(ptr as *const ThreadParker);
                RawWaker::new(ptr, &VTABLE)
            },
            |ptr| unsafe {
                let parker = Arc::from_raw(ptr as *const ThreadParker);
                parker.notified.store(true, Ordering::Release);
                parker.thread.unpark();
            },
            |ptr| unsafe {
                let parker = &*(ptr as *const ThreadParker);
                parker.notified.store(true, Ordering::Release);
                parker.thread.unpark();
            },
            |ptr| unsafe { Arc::decrement_strong_count(ptr as *const ThreadParker); },
        );
        
        let parker = Arc::new(ThreadParker {
            thread: std::thread::current(),
            notified: AtomicBool::new(false),
        });
        
        let raw_waker = RawWaker::new(Arc::into_raw(parker.clone()) as *const (), &VTABLE);
        let waker = unsafe { Waker::from_raw(raw_waker) };
        
        // Register waker and poll
        self.inner.register_waker(&waker);
        
        loop {
            match self.inner.storage.try_take() {
                TakeResult::Ready(value) => return Ok(value),
                TakeResult::Closed => return Err(RecvError),
                TakeResult::Pending => {}
            }
            
            // Check if sender dropped
            if Arc::strong_count(&self.inner) == 1 && self.inner.is_sender_dropped() {
                return Err(RecvError);
            }
            
            // Park if not notified
            if !parker.notified.swap(false, Ordering::Acquire) {
                std::thread::park();
            }
        }
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
