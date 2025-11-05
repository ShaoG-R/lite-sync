use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::atomic_waker::AtomicWaker;

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
/// use lite_sync::oneshot::{State, Sender};
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
/// }
/// 
/// # tokio_test::block_on(async {
/// // Usage:
/// let (notifier, receiver) = Sender::<CustomState>::new();
/// tokio::spawn(async move {
///     notifier.notify(CustomState::Success);
/// });
/// let result = receiver.await; // Direct await
/// assert_eq!(result, CustomState::Success);
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
    
    /// Notify completion with the given state
    /// 
    /// 使用给定状态通知完成
    #[inline]
    pub fn notify(&self, state: T) {
        self.inner.send(state);
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
/// use lite_sync::oneshot::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (notifier, receiver) = Sender::<()>::new();
/// 
/// tokio::spawn(async move {
///     // ... do work ...
///     notifier.notify(());  // Signal completion
/// });
/// 
/// // Two equivalent ways to await:
/// let result = receiver.await;               // Direct await via Future impl
/// assert_eq!(result, ());
/// # });
/// ```
/// 
/// ## Awaiting on mutable reference
/// 
/// ```
/// use lite_sync::oneshot::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (notifier, mut receiver) = Sender::<()>::new();
/// 
/// tokio::spawn(async move {
///     // ... do work ...
///     notifier.notify(());
/// });
/// 
/// // Can also await on &mut receiver
/// let result = (&mut receiver).await;
/// assert_eq!(result, ());
/// # });
/// ```
/// 
/// ## Using custom state
/// 
/// ```
/// use lite_sync::oneshot::{State, Sender};
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
/// }
/// 
/// # tokio_test::block_on(async {
/// let (notifier, receiver) = Sender::<CustomState>::new();
/// 
/// tokio::spawn(async move {
///     notifier.notify(CustomState::Success);
/// });
/// 
/// match receiver.await {
///     CustomState::Success => { /* Success! */ },
///     CustomState::Failure => { /* Failed */ },
///     CustomState::Timeout => { /* Timed out */ },
/// }
/// # });
/// ```
pub struct Receiver<T: State> {
    inner: Arc<Inner<T>>,
}

// Receiver is Unpin because all its fields are Unpin
impl<T: State> Unpin for Receiver<T> {}

// Receiver drop is automatically handled by AtomicWaker's drop implementation
// No need for explicit drop implementation
//
// Receiver 的 drop 由 AtomicWaker 的 drop 实现自动处理
// 无需显式的 drop 实现

impl<T: State> Receiver<T> {
    /// Wait for task completion asynchronously
    /// 
    /// This is equivalent to using `.await` directly on the receiver
    /// 
    /// 异步等待任务完成
    /// 
    /// 这等同于直接在 receiver 上使用 `.await`
    /// 
    /// # Returns
    /// Returns the completion state
    /// 
    /// # 返回值
    /// 返回完成状态
    #[inline]
    pub async fn wait(self) -> T {
        self.await
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
/// 
/// 为 Receiver 直接实现 Future
/// 
/// 这允许 `receiver.await` 和 `(&mut receiver).await` 都能工作
/// 
/// 优化实现：
/// - 快速路径：如已完成则立即返回（无分配）
/// - 慢速路径：直接注册 waker（无 Box 分配，只复制两个指针）
/// - 无需中间 future 状态
impl<T: State> Future for Receiver<T> {
    type Output = T;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Receiver is Unpin, so we can safely get a mutable reference
        let this = self.get_mut();
        
        // Fast path: check if already completed
        let current = this.inner.state.load(Ordering::Acquire);
        if let Some(state) = T::from_u8(current)
            && current != T::pending_value() {
                return Poll::Ready(state);
            }
        
        // Slow path: register waker for notification
        this.inner.register_waker(cx.waker());
        
        // Check again after registering waker to avoid race condition
        // The sender might have completed between our first check and waker registration
        let current = this.inner.state.load(Ordering::Acquire);
        if let Some(state) = T::from_u8(current)
            && current != T::pending_value() {
                return Poll::Ready(state);
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
    }
    
    #[tokio::test]
    async fn test_oneshot_called() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(TestCompletion::Called);
        });
        
        let result = receiver.wait().await;
        assert_eq!(result, TestCompletion::Called);
    }
    
    #[tokio::test]
    async fn test_oneshot_cancelled() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(TestCompletion::Cancelled);
        });
        
        let result = receiver.wait().await;
        assert_eq!(result, TestCompletion::Cancelled);
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate_called() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Notify before waiting (fast path)
        notifier.notify(TestCompletion::Called);
        
        let result = receiver.wait().await;
        assert_eq!(result, TestCompletion::Called);
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate_cancelled() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Notify before waiting (fast path)
        notifier.notify(TestCompletion::Cancelled);
        
        let result = receiver.wait().await;
        assert_eq!(result, TestCompletion::Cancelled);
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
    }
    
    #[tokio::test]
    async fn test_oneshot_custom_state() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(CustomState::Success);
        });
        
        let result = receiver.wait().await;
        assert_eq!(result, CustomState::Success);
    }
    
    #[tokio::test]
    async fn test_oneshot_custom_state_timeout() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        // Immediate notification
        notifier.notify(CustomState::Timeout);
        
        let result = receiver.wait().await;
        assert_eq!(result, CustomState::Timeout);
    }
    
    #[tokio::test]
    async fn test_oneshot_unit_type() {
        let (notifier, receiver) = Sender::<()>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(());
        });
        
        let result = receiver.wait().await;
        assert_eq!(result, ());
    }
    
    #[tokio::test]
    async fn test_oneshot_unit_type_immediate() {
        let (notifier, receiver) = Sender::<()>::new();
        
        // Immediate notification (fast path)
        notifier.notify(());
        
        let result = receiver.wait().await;
        assert_eq!(result, ());
    }
    
    // Tests for Future implementation (direct await)
    #[tokio::test]
    async fn test_oneshot_into_future_called() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(TestCompletion::Called);
        });
        
        // Direct await without .wait()
        let result = receiver.await;
        assert_eq!(result, TestCompletion::Called);
    }
    
    #[tokio::test]
    async fn test_oneshot_into_future_immediate() {
        let (notifier, receiver) = Sender::<TestCompletion>::new();
        
        // Notify before awaiting (fast path)
        notifier.notify(TestCompletion::Cancelled);
        
        // Direct await
        let result = receiver.await;
        assert_eq!(result, TestCompletion::Cancelled);
    }
    
    #[tokio::test]
    async fn test_oneshot_into_future_unit_type() {
        let (notifier, receiver) = Sender::<()>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(());
        });
        
        // Direct await with unit type
        let result = receiver.await;
        assert_eq!(result, ());
    }
    
    #[tokio::test]
    async fn test_oneshot_into_future_custom_state() {
        let (notifier, receiver) = Sender::<CustomState>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(CustomState::Failure);
        });
        
        // Direct await with custom state
        let result = receiver.await;
        assert_eq!(result, CustomState::Failure);
    }
    
    // Test awaiting on &mut receiver
    #[tokio::test]
    async fn test_oneshot_await_mut_reference() {
        let (notifier, mut receiver) = Sender::<TestCompletion>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            notifier.notify(TestCompletion::Called);
        });
        
        // Await on mutable reference
        let result = (&mut receiver).await;
        assert_eq!(result, TestCompletion::Called);
    }
    
    #[tokio::test]
    async fn test_oneshot_await_mut_reference_unit_type() {
        let (notifier, mut receiver) = Sender::<()>::new();
        
        // Immediate notification
        notifier.notify(());
        
        // Await on mutable reference (fast path)
        let result = (&mut receiver).await;
        assert_eq!(result, ());
    }
}

