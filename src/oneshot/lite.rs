use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::atomic_waker::AtomicWaker;

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
            write!(f, "sender dropped")
        }
    }

    impl std::error::Error for RecvError {}
}

use self::error::RecvError;

/// Trait for types that can be used as oneshot state
/// 
/// Types implementing this trait can be converted to/from u8 for atomic storage.
/// This allows for zero-allocation, lock-free state transitions.
/// 
/// 可用作 oneshot 状态的类型的 trait
/// 
/// 实现此 trait 的类型可以与 u8 互相转换以进行原子存储。
/// 这允许零分配、无锁的状态转换。
/// 
/// # Built-in Implementations
/// 
/// - `()`: Simple completion notification without state
/// 
/// # Example: Custom State
/// 
/// ```
/// use lite_sync::oneshot::lite::{State, Sender};
/// 
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// enum CustomState {
///     Success,
///     Failure,
///     Timeout,
/// }
/// 
/// impl State for CustomState {
///     fn to_u8(&self) -> u8 {
///         match self {
///             CustomState::Success => 1,
///             CustomState::Failure => 2,
///             CustomState::Timeout => 3,
///         }
///     }
///     
///     fn from_u8(value: u8) -> Option<Self> {
///         match value {
///             1 => Some(CustomState::Success),
///             2 => Some(CustomState::Failure),
///             3 => Some(CustomState::Timeout),
///             _ => None,
///         }
///     }
///     
///     fn pending_value() -> u8 {
///         0
///     }
///     
///     fn closed_value() -> u8 {
///         255
///     }
/// }
/// 
/// # tokio_test::block_on(async {
/// // Usage:
/// let (notifier, receiver) = Sender::<CustomState>::new();
/// tokio::spawn(async move {
///     notifier.send(CustomState::Success);
/// });
/// let result = receiver.await; // Direct await
/// assert_eq!(result, Ok(CustomState::Success));
/// # });
/// ```
pub trait State: Sized + Send + Sync + 'static {
    /// Convert the state to u8 for atomic storage
    /// 
    /// 将状态转换为 u8 以进行原子存储
    fn to_u8(&self) -> u8;
    
    /// Convert u8 back to the state type
    /// 
    /// Returns None if the value doesn't represent a valid state
    /// 
    /// 将 u8 转换回状态类型
    /// 
    /// 如果值不代表有效状态则返回 None
    fn from_u8(value: u8) -> Option<Self>;
    
    /// The pending state value (before completion)
    /// 
    /// 待处理状态值（完成前）
    fn pending_value() -> u8;
    
    /// The closed state value (sender was dropped without sending)
    /// 
    /// 已关闭状态值（发送器被丢弃而未发送）
    fn closed_value() -> u8;
}

/// Implementation for unit type () - simple completion notification without state
/// 
/// 为单元类型 () 实现 - 简单的完成通知，无需状态信息
impl State for () {
    #[inline]
    fn to_u8(&self) -> u8 {
        1 // Completed
    }
    
    #[inline]
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(()),
            _ => None,
        }
    }
    
    #[inline]
    fn pending_value() -> u8 {
        0 // Pending
    }
    
    #[inline]
    fn closed_value() -> u8 {
        255 // Closed
    }
}

#[inline]
pub fn channel<T: State>() -> (Sender<T>, Receiver<T>) {
    let (notifier, receiver) = Sender::<T>::new();
    (notifier, receiver)
}

/// Inner state for one-shot completion notification
/// 
/// Uses AtomicWaker for zero Box allocation waker storage:
/// - Waker itself is just 2 pointers (16 bytes on 64-bit), no additional heap allocation
/// - Atomic state machine ensures safe concurrent access
/// - Reuses common AtomicWaker implementation
/// 
/// 一次性完成通知的内部状态
/// 
/// 使用 AtomicWaker 实现零 Box 分配的 waker 存储：
/// - Waker 本身只是 2 个指针（64 位系统上 16 字节），无额外堆分配
/// - 原子状态机确保安全的并发访问
/// - 复用通用的 AtomicWaker 实现
pub(crate) struct Inner<T: State> {
    waker: AtomicWaker,
    state: AtomicU8,
    _marker: std::marker::PhantomData<T>,
}

impl<T: State> std::fmt::Debug for Inner<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.load(std::sync::atomic::Ordering::Acquire);
        let is_pending = state == T::pending_value();
        f.debug_struct("Inner")
            .field("state", &state)
            .field("is_pending", &is_pending)
            .finish()
    }
}

impl<T: State> Inner<T> {
    /// Create a new oneshot inner state
    /// 
    /// Extremely fast: just initializes empty waker and pending state
    /// 
    /// 创建一个新的 oneshot 内部状态
    /// 
    /// 极快：仅初始化空 waker 和待处理状态
    #[inline]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            waker: AtomicWaker::new(),
            state: AtomicU8::new(T::pending_value()),
            _marker: std::marker::PhantomData,
        })
    }
    
    /// Send a completion notification (set state and wake)
    /// 
    /// 发送完成通知（设置状态并唤醒）
    #[inline]
    pub(crate) fn send(&self, state: T) {
        // Store completion state first with Release ordering
        self.state.store(state.to_u8(), Ordering::Release);
        
        // Wake the registered waker if any
        self.waker.wake();
    }
    
    /// Register a waker to be notified on completion
    /// 
    /// 注册一个 waker 以在完成时收到通知
    #[inline]
    fn register_waker(&self, waker: &std::task::Waker) {
        self.waker.register(waker);
    }
}

// PERFORMANCE OPTIMIZATION: No Drop implementation for Inner
// Waker cleanup is handled by Receiver::drop instead, which is more efficient because:
// 1. In the common case (sender notifies before receiver drops), waker is already consumed
// 2. Only Receiver creates wakers, so only Receiver needs to clean them up
// 3. This makes Inner::drop a complete no-op, eliminating atomic load overhead
//
// 性能优化：Inner 不实现 Drop
// Waker 清理由 Receiver::drop 处理，这更高效因为：
// 1. 在常见情况下（发送方在接收方 drop 前通知），waker 已被消费
// 2. 只有 Receiver 创建 waker，所以只有 Receiver 需要清理它们
// 3. 这使得 Inner::drop 完全成为 no-op，消除了原子加载开销

/// Completion notifier for one-shot tasks
/// 
/// 一次性任务完成通知器
pub struct Sender<T: State> {
    inner: Arc<Inner<T>>,
}

impl<T: State> std::fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.state.load(std::sync::atomic::Ordering::Acquire);
        let is_pending = state == T::pending_value();
        f.debug_struct("Sender")
            .field("state", &state)
            .field("is_pending", &is_pending)
            .finish()
    }
}

impl<T: State> Sender<T> {
    /// Create a new oneshot completion notifier with receiver
    /// 
    /// 创建一个新的 oneshot 完成通知器和接收器
    /// 
    /// # Returns
    /// Returns a tuple of (notifier, receiver)
    /// 
    /// 返回 (通知器, 接收器) 元组
    #[inline]
    pub fn new() -> (Self, Receiver<T>) {
        let inner = Inner::new();
        
        let notifier = Sender {
            inner: inner.clone(),
        };
        let receiver = Receiver {
            inner,
        };
        
        (notifier, receiver)
    }
    
    /// Send a completion notification with the given state
    /// 
    /// Returns `Err(state)` if the receiver was already dropped.
    /// This method consumes self, guaranteeing single-use.
    /// 
    /// 使用给定状态发送完成通知
    /// 
    /// 如果接收器已被丢弃则返回 `Err(state)`。
    /// 此方法消耗 self，保证单次使用。
    #[inline]
    pub fn send(self, state: T) -> Result<(), T> {
        // Check if receiver is still alive via Arc reference count
        // If count == 1, only Sender holds a reference, meaning Receiver was dropped
        if Arc::strong_count(&self.inner) == 1 {
            return Err(state);
        }
        self.send_unchecked(state);
        Ok(())
    }
    
    /// Send a value without checking if receiver is dropped
    /// 
    /// This is faster than `send()` as it skips the Arc reference count check.
    /// Use this when you know the receiver is still alive, or don't care if
    /// the value is dropped.
    /// 
    /// 发送值而不检查接收器是否已被丢弃
    /// 
    /// 这比 `send()` 更快，因为它跳过了 Arc 引用计数检查。
    /// 当你知道接收器仍然存活，或者不关心值是否被丢弃时使用。
    #[inline]
    pub fn send_unchecked(self, state: T) {
        self.inner.send(state);
        // Prevent drop from setting CLOSED state
        std::mem::forget(self);
    }
}

impl<T: State> Drop for Sender<T> {
    fn drop(&mut self) {
        // Mark the channel as closed when sender is dropped
        // Since send() consumes self, if drop is called, send was never called.
        // No CAS needed - simple store is sufficient.
        //
        // 当发送器被丢弃时标记通道为已关闭
        // 由于 send() 消耗 self，如果 drop 被调用，说明 send 从未被调用。
        // 无需 CAS - 简单 store 即可。
        self.inner.state.store(T::closed_value(), Ordering::Release);
        self.inner.waker.wake();
    }
}

/// Completion receiver for one-shot tasks
/// 
/// Implements `Future` directly, allowing direct `.await` usage on both owned values and mutable references
/// 
/// 一次性任务完成通知接收器
/// 
/// 直接实现了 `Future`，允许对拥有的值和可变引用都直接使用 `.await`
/// 
/// # Examples
/// 
/// ## Using unit type for simple completion
/// 
/// ```
/// use lite_sync::oneshot::lite::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (notifier, receiver) = Sender::<()>::new();
/// 
/// tokio::spawn(async move {
///     // ... do work ...
///     notifier.send(());  // Signal completion
/// });
/// 
/// // Two equivalent ways to await:
/// let result = receiver.await;               // Direct await via Future impl
/// assert_eq!(result, Ok(()));
/// # });
/// ```
/// 
/// ## Awaiting on mutable reference
/// 
/// ```
/// use lite_sync::oneshot::lite::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (notifier, mut receiver) = Sender::<()>::new();
/// 
/// tokio::spawn(async move {
///     // ... do work ...
///     notifier.send(());
/// });
/// 
/// // Can also await on &mut receiver
/// let result = (&mut receiver).await;
/// assert_eq!(result, Ok(()));
/// # });
/// ```
/// 
/// ## Using custom state
/// 
/// ```
/// use lite_sync::oneshot::lite::{State, Sender};
/// 
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// enum CustomState {
///     Success,
///     Failure,
///     Timeout,
/// }
/// 
/// impl State for CustomState {
///     fn to_u8(&self) -> u8 {
///         match self {
///             CustomState::Success => 1,
///             CustomState::Failure => 2,
///             CustomState::Timeout => 3,
///         }
///     }
///     
///     fn from_u8(value: u8) -> Option<Self> {
///         match value {
///             1 => Some(CustomState::Success),
///             2 => Some(CustomState::Failure),
///             3 => Some(CustomState::Timeout),
///             _ => None,
///         }
///     }
///     
///     fn pending_value() -> u8 {
///         0
///     }
///     
///     fn closed_value() -> u8 {
///         255
///     }
/// }
/// 
/// # tokio_test::block_on(async {
/// let (notifier, receiver) = Sender::<CustomState>::new();
/// 
/// tokio::spawn(async move {
///     notifier.send(CustomState::Success);
/// });
/// 
/// match receiver.await {
///     Ok(CustomState::Success) => { /* Success! */ },
///     Ok(CustomState::Failure) => { /* Failed */ },
///     Ok(CustomState::Timeout) => { /* Timed out */ },
///     Err(_) => { /* Sender dropped */ },
/// }
/// # });
/// ```
pub struct Receiver<T: State> {
    inner: Arc<Inner<T>>,
}

impl<T: State> std::fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.state.load(std::sync::atomic::Ordering::Acquire);
        let is_pending = state == T::pending_value();
        f.debug_struct("Receiver")
            .field("state", &state)
            .field("is_pending", &is_pending)
            .finish()
    }
}

// Receiver is Unpin because all its fields are Unpin
impl<T: State> Unpin for Receiver<T> {}

// Receiver drop is automatically handled by AtomicWaker's drop implementation
// No need for explicit drop implementation
//
// Receiver 的 drop 由 AtomicWaker 的 drop 实现自动处理
// 无需显式的 drop 实现

impl<T: State> Receiver<T> {
    /// Receive a value asynchronously
    /// 
    /// This is equivalent to using `.await` directly on the receiver
    /// 
    /// Returns `Err(RecvError)` if the sender was dropped before sending a value
    /// 
    /// 异步接收一个值
    /// 
    /// 这等同于直接在 receiver 上使用 `.await`
    /// 
    /// 如果发送器在发送值之前被丢弃则返回 `Err(RecvError)`
    /// 
    /// # Returns
    /// Returns the completion state or error if sender was dropped
    /// 
    /// # 返回值
    /// 返回完成状态或发送器被丢弃时的错误
    #[inline]
    pub async fn recv(self) -> Result<T, RecvError> {
        self.await
    }
    
    /// Try to receive a value without blocking
    /// 
    /// Returns `None` if no value has been sent yet
    /// Returns `Err(RecvError)` if the sender was dropped
    /// 
    /// 尝试接收值而不阻塞
    /// 
    /// 如果还没有发送值则返回 `None`
    /// 如果发送器被丢弃则返回 `Err(RecvError)`
    /// 
    /// # Returns
    /// Returns `Some(value)` if value is ready, `None` if pending, or `Err(RecvError)` if sender dropped
    /// 
    /// # 返回值
    /// 如果值已就绪返回 `Some(value)`，如果待处理返回 `None`，如果发送器被丢弃返回 `Err(RecvError)`
    #[inline]
    pub fn try_recv(&mut self) -> Result<Option<T>, RecvError> {
        let current = self.inner.state.load(Ordering::Acquire);
        
        // Check if sender was dropped
        if current == T::closed_value() {
            return Err(RecvError);
        }
        
        // Check if value is ready
        if let Some(state) = T::from_u8(current)
            && current != T::pending_value() {
                return Ok(Some(state));
            }
        
        Ok(None)
    }
}

/// Direct Future implementation for Receiver
/// 
/// This allows both `receiver.await` and `(&mut receiver).await` to work
/// 
/// Optimized implementation:
/// - Fast path: Immediate return if already completed (no allocation)
/// - Slow path: Direct waker registration (no Box allocation, just copy two pointers)
/// - No intermediate future state needed
/// - Detects when sender is dropped and returns error
/// 
/// 为 Receiver 直接实现 Future
/// 
/// 这允许 `receiver.await` 和 `(&mut receiver).await` 都能工作
/// 
/// 优化实现：
/// - 快速路径：如已完成则立即返回（无分配）
/// - 慢速路径：直接注册 waker（无 Box 分配，只复制两个指针）
/// - 无需中间 future 状态
/// - 检测发送器何时被丢弃并返回错误
impl<T: State> Future for Receiver<T> {
    type Output = Result<T, RecvError>;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Receiver is Unpin, so we can safely get a mutable reference
        let this = self.get_mut();
        
        // Fast path: check if already completed or closed
        let current = this.inner.state.load(Ordering::Acquire);
        
        // Check if sender was dropped
        if current == T::closed_value() {
            return Poll::Ready(Err(RecvError));
        }
        
        if let Some(state) = T::from_u8(current)
            && current != T::pending_value() {
                return Poll::Ready(Ok(state));
            }
        
        // Slow path: register waker for notification
        this.inner.register_waker(cx.waker());
        
        // Check again after registering waker to avoid race condition
        // The sender might have completed between our first check and waker registration
        let current = this.inner.state.load(Ordering::Acquire);
        
        // Check if sender was dropped
        if current == T::closed_value() {
            return Poll::Ready(Err(RecvError));
        }
        
        if let Some(state) = T::from_u8(current)
            && current != T::pending_value() {
                return Poll::Ready(Ok(state));
            }
        
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    /// Test-only state type for completion notification
    /// 
    /// 测试专用的完成通知状态类型
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestCompletion {
        /// Task was called/completed successfully
        /// 
        /// 任务被调用/成功完成
        Called,
        
        /// Task was cancelled
        /// 
        /// 任务被取消
        Cancelled,
    }
    
    impl State for TestCompletion {
        fn to_u8(&self) -> u8 {
            match self {
                TestCompletion::Called => 1,
                TestCompletion::Cancelled => 2,
            }
        }
        
        fn from_u8(value: u8) -> Option<Self> {
            match value {
                1 => Some(TestCompletion::Called),
                2 => Some(TestCompletion::Cancelled),
                _ => None,
            }
        }
        
        fn pending_value() -> u8 {
            0
        }
        
        fn closed_value() -> u8 {
            255
        }
    }
    
    #[tokio::test]
    async fn test_oneshot_called() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(TestCompletion::Called).unwrap();
        });
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(TestCompletion::Called));
    }
    
    #[tokio::test]
    async fn test_oneshot_cancelled() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(TestCompletion::Cancelled).unwrap();
        });
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(TestCompletion::Cancelled));
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate_called() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Notify before waiting (fast path)
        notifier.send(TestCompletion::Called).unwrap();
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(TestCompletion::Called));
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate_cancelled() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Notify before waiting (fast path)
        notifier.send(TestCompletion::Cancelled).unwrap();
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(TestCompletion::Cancelled));
    }
    
    // Test with custom state type
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CustomState {
        Success,
        Failure,
        Timeout,
    }
    
    impl State for CustomState {
        fn to_u8(&self) -> u8 {
            match self {
                CustomState::Success => 1,
                CustomState::Failure => 2,
                CustomState::Timeout => 3,
            }
        }
        
        fn from_u8(value: u8) -> Option<Self> {
            match value {
                1 => Some(CustomState::Success),
                2 => Some(CustomState::Failure),
                3 => Some(CustomState::Timeout),
                _ => None,
            }
        }
        
        fn pending_value() -> u8 {
            0
        }
        
        fn closed_value() -> u8 {
            255
        }
    }
    
    #[tokio::test]
    async fn test_oneshot_custom_state() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(CustomState::Success).unwrap();
        });
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(CustomState::Success));
    }
    
    #[tokio::test]
    async fn test_oneshot_custom_state_timeout() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        // Immediate notification
        notifier.send(CustomState::Timeout).unwrap();
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(CustomState::Timeout));
    }
    
    #[tokio::test]
    async fn test_oneshot_unit_type() {
        let (notifier, receiver) = Sender::<()>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(()).unwrap();
        });
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(()));
    }
    
    #[tokio::test]
    async fn test_oneshot_unit_type_immediate() {
        let (notifier, receiver) = Sender::<()>::new();
        
        // Immediate notification (fast path)
        notifier.send(()).unwrap();
        
        let result = receiver.recv().await;
        assert_eq!(result, Ok(()));
    }
    
    // Tests for Future implementation (direct await)
    #[tokio::test]
    async fn test_oneshot_into_future_called() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(TestCompletion::Called).unwrap();
        });
        
        // Direct await without .wait()
        let result = receiver.await;
        assert_eq!(result, Ok(TestCompletion::Called));
    }
    
    #[tokio::test]
    async fn test_oneshot_into_future_immediate() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Notify before awaiting (fast path)
        notifier.send(TestCompletion::Cancelled).unwrap();
        
        // Direct await
        let result = receiver.await;
        assert_eq!(result, Ok(TestCompletion::Cancelled));
    }
    
    #[tokio::test]
    async fn test_oneshot_into_future_unit_type() {
        let (notifier, receiver) = Sender::<()>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(()).unwrap();
        });
        
        // Direct await with unit type
        let result = receiver.await;
        assert_eq!(result, Ok(()));
    }
    
    #[tokio::test]
    async fn test_oneshot_into_future_custom_state() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(CustomState::Failure).unwrap();
        });
        
        // Direct await with custom state
        let result = receiver.await;
        assert_eq!(result, Ok(CustomState::Failure));
    }
    
    // Test awaiting on &mut receiver
    #[tokio::test]
    async fn test_oneshot_await_mut_reference() {
        let (notifier, mut receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.send(TestCompletion::Called).unwrap();
        });
        
        // Await on mutable reference
        let result = (&mut receiver).await;
        assert_eq!(result, Ok(TestCompletion::Called));
    }
    
    #[tokio::test]
    async fn test_oneshot_await_mut_reference_unit_type() {
        let (notifier, mut receiver) = Sender::<()>::new();
        
        // Immediate notification
        notifier.send(()).unwrap();
        
        // Await on mutable reference (fast path)
        let result = (&mut receiver).await;
        assert_eq!(result, Ok(()));
    }
    
    // Tests for try_recv
    #[tokio::test]
    async fn test_oneshot_try_recv_pending() {
        let (_notifier, mut receiver) = Sender::<TestCompletion>::new();
        
        // Try receive before sending
        let result = receiver.try_recv();
        assert_eq!(result, Ok(None));
    }
    
    #[tokio::test]
    async fn test_oneshot_try_recv_ready() {
        let (notifier, mut receiver) = Sender::<TestCompletion>::new();
        
        // Send value
        notifier.send(TestCompletion::Called).unwrap();
        
        // Try receive after sending
        let result = receiver.try_recv();
        assert_eq!(result, Ok(Some(TestCompletion::Called)));
    }
    
    #[tokio::test]
    async fn test_oneshot_try_recv_sender_dropped() {
        let (notifier, mut receiver) = Sender::<TestCompletion>::new();
        
        // Drop sender without sending
        drop(notifier);
        
        // Try receive should return error
        let result = receiver.try_recv();
        assert_eq!(result, Err(RecvError));
    }
    
    // Tests for sender dropped behavior
    #[tokio::test]
    async fn test_oneshot_sender_dropped_before_recv() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Drop sender without sending
        drop(notifier);
        
        // Recv should return error
        let result = receiver.recv().await;
        assert_eq!(result, Err(RecvError));
    }
    
    #[tokio::test]
    async fn test_oneshot_sender_dropped_unit_type() {
        let (notifier, receiver) = Sender::<()>::new();
        
        // Drop sender without sending
        drop(notifier);
        
        // Recv should return error
        let result = receiver.recv().await;
        assert_eq!(result, Err(RecvError));
    }
    
    #[tokio::test]
    async fn test_oneshot_sender_dropped_custom_state() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        // Drop sender without sending
        drop(notifier);
        
        // Recv should return error
        let result = receiver.recv().await;
        assert_eq!(result, Err(RecvError));
    }
}

