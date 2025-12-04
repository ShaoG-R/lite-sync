//! Lightweight oneshot channel for State-encodable types.
//!
//! 用于 State 可编码类型的轻量级一次性通道。

use std::sync::atomic::{AtomicU8, Ordering};

use super::common::{OneshotStorage, TakeResult};

// Re-export common types
pub use super::common::error;
pub use super::common::RecvError;

// ============================================================================
// State Trait
// ============================================================================

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

// ============================================================================
// Lite Storage
// ============================================================================

/// Storage for State-encodable types using only `AtomicU8`
/// 
/// 使用 `AtomicU8` 存储 State 可编码类型
pub struct LiteStorage<S: State> {
    state: AtomicU8,
    _marker: std::marker::PhantomData<S>,
}

unsafe impl<S: State> Send for LiteStorage<S> {}
unsafe impl<S: State> Sync for LiteStorage<S> {}

impl<S: State> OneshotStorage for LiteStorage<S> {
    type Value = S;
    
    #[inline]
    fn new() -> Self {
        Self {
            state: AtomicU8::new(S::pending_value()),
            _marker: std::marker::PhantomData,
        }
    }
    
    #[inline]
    fn store(&self, value: S) {
        self.state.store(value.to_u8(), Ordering::Release);
    }
    
    #[inline]
    fn try_take(&self) -> TakeResult<S> {
        let current = self.state.load(Ordering::Acquire);
        
        if current == S::closed_value() {
            return TakeResult::Closed;
        }
        
        if current == S::pending_value() {
            return TakeResult::Pending;
        }
        
        // Value is ready
        if let Some(state) = S::from_u8(current) {
            TakeResult::Ready(state)
        } else {
            TakeResult::Pending
        }
    }
    
    #[inline]
    fn is_sender_dropped(&self) -> bool {
        self.state.load(Ordering::Acquire) == S::closed_value()
    }
    
    #[inline]
    fn mark_sender_dropped(&self) {
        self.state.store(S::closed_value(), Ordering::Release);
    }
}

// ============================================================================
// Type Aliases
// ============================================================================

/// Sender for one-shot state transfer
/// 
/// 用于一次性状态传递的发送器
pub type Sender<S> = super::common::Sender<LiteStorage<S>>;

/// Receiver for one-shot state transfer
/// 
/// 用于一次性状态传递的接收器
pub type Receiver<S> = super::common::Receiver<LiteStorage<S>>;

/// Create a new oneshot channel for State types
/// 
/// 创建一个用于 State 类型的新 oneshot 通道
#[inline]
pub fn channel<S: State>() -> (Sender<S>, Receiver<S>) {
    Sender::new()
}

// ============================================================================
// Receiver Extension Methods
// ============================================================================

impl<S: State> Receiver<S> {
    /// Receive a value asynchronously
    /// 
    /// This is equivalent to using `.await` directly on the receiver
    /// 
    /// 异步接收一个值
    /// 
    /// 这等同于直接在 receiver 上使用 `.await`
    #[inline]
    pub async fn recv(self) -> Result<S, RecvError> {
        self.await
    }
    
    /// Try to receive a value without blocking
    /// 
    /// Returns `Ok(Some(value))` if value is ready, `Ok(None)` if pending,
    /// or `Err(RecvError)` if sender was dropped.
    /// 
    /// 尝试接收值而不阻塞
    /// 
    /// 如果值就绪返回 `Ok(Some(value))`，如果待处理返回 `Ok(None)`，
    /// 如果发送器被丢弃返回 `Err(RecvError)`
    #[inline]
    pub fn try_recv(&mut self) -> Result<Option<S>, RecvError> {
        match self.inner.try_recv() {
            TakeResult::Ready(v) => Ok(Some(v)),
            TakeResult::Pending => Ok(None),
            TakeResult::Closed => Err(RecvError),
        }
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

