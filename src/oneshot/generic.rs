use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;

use crate::atomic_waker::AtomicWaker;

// States for the value cell
const EMPTY: u8 = 0;   // No value stored
const READY: u8 = 1;   // Value is ready

/// Inner state for one-shot value transfer
/// 
/// Uses UnsafeCell<MaybeUninit<T>> for direct value storage without any overhead:
/// - Waker itself is just 2 pointers (16 bytes on 64-bit), no additional heap allocation
/// - Value is stored directly with zero overhead (no Option discriminant)
/// - Atomic state machine (AtomicU8) tracks initialization state
/// - UnsafeCell allows interior mutability with proper synchronization
/// 
/// 一次性值传递的内部状态
/// 
/// 使用 UnsafeCell<MaybeUninit<T>> 实现零开销的直接值存储：
/// - Waker 本身只是 2 个指针（64 位系统上 16 字节），无额外堆分配
/// - 值直接存储，零开销（无 Option 判别标记）
/// - 原子状态机（AtomicU8）跟踪初始化状态
/// - UnsafeCell 允许通过适当同步实现内部可变性
pub(crate) struct Inner<T> {
    waker: AtomicWaker,
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Inner<T> {
    /// Create a new oneshot inner state
    /// 
    /// Extremely fast: just initializes empty waker and uninitialized memory
    /// 
    /// 创建一个新的 oneshot 内部状态
    /// 
    /// 极快：仅初始化空 waker 和未初始化内存
    #[inline]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            waker: AtomicWaker::new(),
            state: AtomicU8::new(EMPTY),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        })
    }
    
    /// Send a value (store value and wake)
    /// 
    /// 发送一个值（存储值并唤醒）
    #[inline]
    pub(crate) fn send(&self, value: T) -> Result<(), T> {
        // Try to transition from EMPTY to READY
        match self.state.compare_exchange(
            EMPTY,
            READY,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // Successfully transitioned, store the value
                // SAFETY: We have exclusive access via the state transition
                // No other thread can access the value until state is READY
                // We're initializing previously uninitialized memory
                unsafe {
                    (*self.value.get()).write(value);
                }
                
                // Wake the registered waker if any
                self.waker.wake();
                Ok(())
            }
            Err(_) => {
                // State was not EMPTY, value already sent
                Err(value)
            }
        }
    }
    
    /// Try to receive the value without blocking
    /// 
    /// 尝试接收值而不阻塞
    #[inline]
    fn try_recv(&self) -> Option<T> {
        // Check if value is ready
        if self.state.load(Ordering::Acquire) == READY {
            // Try to transition from READY back to EMPTY to claim the value
            match self.state.compare_exchange(
                READY,
                EMPTY,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Successfully claimed the value
                    // SAFETY: We have exclusive access via the state transition
                    // State was READY, so value must be initialized
                    unsafe {
                        Some((*self.value.get()).assume_init_read())
                    }
                }
                Err(_) => {
                    // Someone else already claimed it
                    None
                }
            }
        } else {
            None
        }
    }
    
    /// Register a waker to be notified on completion
    /// 
    /// 注册一个 waker 以在完成时收到通知
    #[inline]
    fn register_waker(&self, waker: &std::task::Waker) {
        self.waker.register(waker);
    }
}

// SAFETY: Inner<T> is Send + Sync as long as T is Send
// - UnsafeCell<MaybeUninit<T>> is protected by atomic state transitions
// - Only one thread can access the value at a time (enforced by state machine)
// - Value is only accessed when state is READY (initialized)
// - Waker is properly synchronized via AtomicWaker
//
// SAFETY: 只要 T 是 Send，Inner<T> 就是 Send + Sync
// - UnsafeCell<MaybeUninit<T>> 由原子状态转换保护
// - 每次只有一个线程可以访问值（由状态机强制执行）
// - 只有在状态为 READY（已初始化）时才访问值
// - Waker 通过 AtomicWaker 正确同步
unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        // Clean up the value if it was sent but not received
        // SAFETY: We have exclusive access in drop (&mut self)
        // Only drop if state is READY (value was initialized)
        if *self.state.get_mut() == READY {
            unsafe {
                // Value is initialized, must drop it
                (*self.value.get()).assume_init_drop();
            }
        }
        // If state is EMPTY, value is uninitialized, nothing to drop
    }
}

/// Error returned when the receiver is dropped before receiving a value
/// 
/// 当接收器在接收值之前被丢弃时返回的错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvError;

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "channel closed")
    }
}

impl std::error::Error for RecvError {}

#[inline]
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    Sender::new()
}

/// Sender for one-shot value transfer
/// 
/// 一次性值传递的发送器
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Sender<T> {
    /// Create a new oneshot sender with receiver
    /// 
    /// 创建一个新的 oneshot 发送器和接收器
    /// 
    /// # Returns
    /// Returns a tuple of (sender, receiver)
    /// 
    /// 返回 (发送器, 接收器) 元组
    #[inline]
    pub fn new() -> (Self, Receiver<T>) {
        let inner = Inner::new();
        
        let sender = Sender {
            inner: inner.clone(),
        };
        let receiver = Receiver {
            inner,
        };
        
        (sender, receiver)
    }
    
    /// Send a value through the channel
    /// 
    /// Returns `Err(value)` if the receiver was already dropped
    /// 
    /// 通过通道发送一个值
    /// 
    /// 如果接收器已被丢弃则返回 `Err(value)`
    #[inline]
    pub fn send(self, value: T) -> Result<(), T> {
        self.inner.send(value)
    }
}

/// Receiver for one-shot value transfer
/// 
/// Implements `Future` directly, allowing direct `.await` usage on both owned values and mutable references
/// 
/// 一次性值传递的接收器
/// 
/// 直接实现了 `Future`，允许对拥有的值和可变引用都直接使用 `.await`
/// 
/// # Examples
/// 
/// ## Basic usage
/// 
/// ```
/// use lite_sync::oneshot::generic::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (sender, receiver) = Sender::<String>::new();
/// 
/// tokio::spawn(async move {
///     sender.send("Hello, World!".to_string()).unwrap();
/// });
/// 
/// // Direct await via Future impl
/// let message = receiver.await;
/// assert_eq!(message, "Hello, World!");
/// # });
/// ```
/// 
/// ## Awaiting on mutable reference
/// 
/// ```
/// use lite_sync::oneshot::generic::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (sender, mut receiver) = Sender::<i32>::new();
/// 
/// tokio::spawn(async move {
///     sender.send(42).unwrap();
/// });
/// 
/// // Can also await on &mut receiver
/// let value = (&mut receiver).await;
/// assert_eq!(value, 42);
/// # });
/// ```
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

// Receiver is Unpin because all its fields are Unpin
impl<T> Unpin for Receiver<T> {}

impl<T> Receiver<T> {
    /// Wait for value asynchronously
    /// 
    /// This is equivalent to using `.await` directly on the receiver
    /// 
    /// 异步等待值
    /// 
    /// 这等同于直接在 receiver 上使用 `.await`
    /// 
    /// # Returns
    /// Returns the received value
    /// 
    /// # 返回值
    /// 返回接收到的值
    #[inline]
    pub async fn wait(self) -> T {
        self.await
    }
    
    /// Try to receive a value without blocking
    /// 
    /// Returns `None` if no value has been sent yet
    /// 
    /// 尝试接收值而不阻塞
    /// 
    /// 如果还没有发送值则返回 `None`
    #[inline]
    pub fn try_recv(&mut self) -> Option<T> {
        self.inner.try_recv()
    }
}

/// Direct Future implementation for Receiver
/// 
/// This allows both `receiver.await` and `(&mut receiver).await` to work
/// 
/// Optimized implementation:
/// - Fast path: Immediate return if value already sent (no allocation)
/// - Slow path: Direct waker registration (no Box allocation, just copy two pointers)
/// - No intermediate future state needed
/// 
/// 为 Receiver 直接实现 Future
/// 
/// 这允许 `receiver.await` 和 `(&mut receiver).await` 都能工作
/// 
/// 优化实现：
/// - 快速路径：如值已发送则立即返回（无分配）
/// - 慢速路径：直接注册 waker（无 Box 分配，只复制两个指针）
/// - 无需中间 future 状态
impl<T> Future for Receiver<T> {
    type Output = T;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Receiver is Unpin, so we can safely get a mutable reference
        let this = self.get_mut();
        
        // Fast path: check if value already sent
        if let Some(value) = this.inner.try_recv() {
            return Poll::Ready(value);
        }
        
        // Slow path: register waker for notification
        this.inner.register_waker(cx.waker());
        
        // Check again after registering waker to avoid race condition
        // The sender might have sent between our first check and waker registration
        if let Some(value) = this.inner.try_recv() {
            return Poll::Ready(value);
        }
        
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_oneshot_string() {
        let (sender, receiver) = Sender::<String>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send("Hello".to_string()).unwrap();
        });
        
        let result = receiver.wait().await;
        assert_eq!(result, "Hello");
    }
    
    #[tokio::test]
    async fn test_oneshot_integer() {
        let (sender, receiver) = Sender::<i32>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send(42).unwrap();
        });
        
        let result = receiver.wait().await;
        assert_eq!(result, 42);
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate() {
        let (sender, receiver) = Sender::<String>::new();
        
        // Send before waiting (fast path)
        sender.send("Immediate".to_string()).unwrap();
        
        let result = receiver.wait().await;
        assert_eq!(result, "Immediate");
    }
    
    #[tokio::test]
    async fn test_oneshot_custom_struct() {
        #[derive(Debug, Clone, PartialEq)]
        struct CustomData {
            id: u64,
            name: String,
        }
        
        let (sender, receiver) = Sender::<CustomData>::new();
        
        let data = CustomData {
            id: 123,
            name: "Test".to_string(),
        };
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send(data).unwrap();
        });
        
        let result = receiver.wait().await;
        assert_eq!(result.id, 123);
        assert_eq!(result.name, "Test");
    }
    
    #[tokio::test]
    async fn test_oneshot_direct_await() {
        let (sender, receiver) = Sender::<i32>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send(99).unwrap();
        });
        
        // Direct await without .wait()
        let result = receiver.await;
        assert_eq!(result, 99);
    }
    
    #[tokio::test]
    async fn test_oneshot_await_mut_reference() {
        let (sender, mut receiver) = Sender::<String>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send("Mutable".to_string()).unwrap();
        });
        
        // Await on mutable reference
        let result = (&mut receiver).await;
        assert_eq!(result, "Mutable");
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate_await() {
        let (sender, receiver) = Sender::<Vec<u8>>::new();
        
        // Immediate send (fast path)
        sender.send(vec![1, 2, 3]).unwrap();
        
        // Direct await
        let result = receiver.await;
        assert_eq!(result, vec![1, 2, 3]);
    }
    
    #[tokio::test]
    async fn test_oneshot_try_recv() {
        let (sender, mut receiver) = Sender::<i32>::new();
        
        // Try receive before sending
        assert_eq!(receiver.try_recv(), None);
        
        // Send value
        sender.send(42).unwrap();
        
        // Try receive after sending
        assert_eq!(receiver.try_recv(), Some(42));
    }
    
    #[tokio::test]
    async fn test_oneshot_large_data() {
        let (sender, receiver) = Sender::<Vec<u8>>::new();
        
        let large_vec = vec![0u8; 1024 * 1024]; // 1MB
        
        tokio::spawn(async move {
            sender.send(large_vec).unwrap();
        });
        
        let result = receiver.await;
        assert_eq!(result.len(), 1024 * 1024);
    }
}
