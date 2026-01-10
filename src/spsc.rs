//! High-performance async SPSC (Single Producer Single Consumer) channel
//!
//! Built on top of `smallring` - a high-performance ring buffer with inline storage support.
//! Optimized for low latency and fast creation, designed to replace tokio mpsc in timer implementation.
//! The `N` const generic parameter allows specifying inline buffer size for zero-allocation small channels.
//!
//! 高性能异步 SPSC（单生产者单消费者）通道
//!
//! 基于 `smallring` 构建 - 一个支持内联存储的高性能环形缓冲区。
//! 针对低延迟和快速创建进行优化，用于替代定时器实现中的 tokio mpsc。
//! `N` 常量泛型参数允许指定内联缓冲区大小，实现小容量通道的零分配。
//!
//! # 安全性说明 (Safety Notes)
//!
//! 本实现使用 `UnsafeCell` 来提供零成本内部可变性，而不是 `Mutex`。
//! 这是安全的，基于以下保证：
//!
//! 1. **单一所有权**：`Sender` 和 `Receiver` 都不实现 `Clone`，确保每个通道只有一个发送者和一个接收者
//! 2. **访问隔离**：`Producer` 只被唯一的 `Sender` 访问，`Consumer` 只被唯一的 `Receiver` 访问
//! 3. **无数据竞争**：由于单一所有权，不会有多个线程同时访问同一个 `Producer` 或 `Consumer`
//! 4. **原子通信**：`Producer` 和 `Consumer` 内部使用原子操作进行跨线程通信
//! 5. **类型系统保证**：通过类型系统强制 SPSC 语义，防止误用为 MPMC
//!
//! 这种设计实现了零同步开销，完全消除了 `Mutex` 的性能损失。
//!
//! # Safety Guarantees
//!
//! This implementation uses `UnsafeCell` for zero-cost interior mutability instead of `Mutex`.
//! This is safe based on the following guarantees:
//!
//! 1. **Single Ownership**: Neither `Sender` nor `Receiver` implements `Clone`, ensuring only one sender and one receiver per channel
//! 2. **Access Isolation**: `Producer` is only accessed by the unique `Sender`, `Consumer` only by the unique `Receiver`
//! 3. **No Data Races**: Due to single ownership, there's no concurrent access to the same `Producer` or `Consumer`
//! 4. **Atomic Communication**: `Producer` and `Consumer` use atomic operations internally for cross-thread communication
//! 5. **Type System Enforcement**: SPSC semantics are enforced by the type system, preventing misuse as MPMC
//!
//! This design achieves zero synchronization overhead, completely eliminating `Mutex` performance costs.
use super::notify::SingleWaiterNotify;
use crate::shim::atomic::{AtomicBool, Ordering};
use crate::shim::cell::UnsafeCell;
use crate::shim::sync::Arc;
use smallring::spsc::{Consumer, PopError, Producer, PushError, new};
use std::num::NonZeroUsize;

/// SPSC channel creation function
///
/// Creates a bounded SPSC channel with the specified capacity.
///
/// # Type Parameters
/// - `T`: The type of messages to be sent through the channel
/// - `N`: The size of the inline buffer (number of elements stored inline before heap allocation)
///
/// # Parameters
/// - `capacity`: Channel capacity (total number of elements the channel can hold)
///
/// # Returns
/// A tuple of (Sender, Receiver)
///
/// # Examples
///
/// ```
/// use lite_sync::spsc::channel;
/// use std::num::NonZeroUsize;
///
/// #[tokio::main]
/// async fn main() {
///     #[cfg(not(feature = "loom"))]
///     {
///         // Create a channel with capacity 32 and inline buffer size 8
///         let (tx, rx) = channel::<i32, 8>(NonZeroUsize::new(32).unwrap());
///     
///         tokio::spawn(async move {
///             tx.send(42).await.unwrap();
///         });
///     
///         let value = rx.recv().await.unwrap();
///         assert_eq!(value, 42);
///     }
/// }
/// ```
///
/// SPSC 通道创建函数
///
/// 创建指定容量的有界 SPSC 通道。
///
/// # 类型参数
/// - `T`: 通过通道发送的消息类型
/// - `N`: 内联缓冲区大小（在堆分配之前内联存储的元素数量）
///
/// # 参数
/// - `capacity`: 通道容量（通道可以容纳的元素总数）
///
/// # 返回值
/// 返回 (Sender, Receiver) 元组
pub fn channel<T, const N: usize>(capacity: NonZeroUsize) -> (Sender<T, N>, Receiver<T, N>) {
    let (producer, consumer) = new::<T, N>(capacity);

    let inner = Arc::new(Inner::<T, N> {
        producer: UnsafeCell::new(producer),
        consumer: UnsafeCell::new(consumer),
        closed: AtomicBool::new(false),
        recv_notify: SingleWaiterNotify::new(),
        send_notify: SingleWaiterNotify::new(),
    });

    let sender = Sender {
        inner: inner.clone(),
    };

    let receiver = Receiver { inner };

    (sender, receiver)
}

/// Shared internal state for SPSC channel
///
/// Contains both shared state and the ring buffer halves.
/// Uses UnsafeCell for zero-cost interior mutability of Producer/Consumer.
///
/// SPSC 通道的共享内部状态
///
/// 包含共享状态和环形缓冲区的两端。
/// 使用 UnsafeCell 实现 Producer/Consumer 的零成本内部可变性。
struct Inner<T, const N: usize = 32> {
    /// Producer (wrapped in UnsafeCell for zero-cost interior mutability)
    ///
    /// 生产者（用 UnsafeCell 包装以实现零成本内部可变性）
    producer: UnsafeCell<Producer<T, N>>,

    /// Consumer (wrapped in UnsafeCell for zero-cost interior mutability)
    ///
    /// 消费者（用 UnsafeCell 包装以实现零成本内部可变性）
    consumer: UnsafeCell<Consumer<T, N>>,

    /// Channel closed flag
    ///
    /// 通道关闭标志
    closed: AtomicBool,

    /// Notifier for receiver waiting (lightweight single-waiter)
    ///
    /// 接收者等待通知器（轻量级单等待者）
    recv_notify: SingleWaiterNotify,

    /// Notifier for sender waiting when buffer is full (lightweight single-waiter)
    ///
    /// 发送者等待通知器，当缓冲区满时使用（轻量级单等待者）
    send_notify: SingleWaiterNotify,
}

// SAFETY: Inner<T> 可以在线程间安全共享的原因：
// 1. Sender 和 Receiver 都不实现 Clone，确保单一所有权
// 2. producer 只被唯一的 Sender 访问，不会有多线程竞争
// 3. consumer 只被唯一的 Receiver 访问，不会有多线程竞争
// 4. closed、recv_notify、send_notify 都已经是线程安全的
// 5. Producer 和 Consumer 内部使用原子操作进行跨线程通信
unsafe impl<T: Send, const N: usize> Sync for Inner<T, N> {}

impl<T, const N: usize> std::fmt::Debug for Inner<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish()
    }
}

/// SPSC channel sender
///
/// SPSC 通道发送器
pub struct Sender<T, const N: usize> {
    inner: Arc<Inner<T, N>>,
}

impl<T, const N: usize> std::fmt::Debug for Sender<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender")
            .field("closed", &self.is_closed())
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

/// SPSC channel receiver
///
/// SPSC 通道接收器
pub struct Receiver<T, const N: usize> {
    inner: Arc<Inner<T, N>>,
}

impl<T, const N: usize> std::fmt::Debug for Receiver<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Receiver")
            .field("is_empty", &self.is_empty())
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

/// Draining iterator for the SPSC channel
///
/// SPSC 通道的消费迭代器
///
/// This iterator removes and returns messages from the channel until it's empty.
///
/// 此迭代器从通道中移除并返回消息，直到通道为空。
///
/// # Type Parameters
/// - `T`: Message type
/// - `N`: Inline buffer size
///
/// # 类型参数
/// - `T`: 消息类型
/// - `N`: 内联缓冲区大小
pub struct Drain<'a, T, const N: usize> {
    receiver: &'a mut Receiver<T, N>,
}

impl<'a, T, const N: usize> std::fmt::Debug for Drain<'a, T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drain")
            .field("len", &self.receiver.len())
            .field("is_empty", &self.receiver.is_empty())
            .finish()
    }
}

impl<'a, T, const N: usize> Iterator for Drain<'a, T, N> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.try_recv().ok()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.receiver.len();
        (len, Some(len))
    }
}

/// Send error type
///
/// 发送错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError<T> {
    /// Channel is closed
    ///
    /// 通道已关闭
    Closed(T),
}

/// Try-receive error type
///
/// 尝试接收错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// Channel is empty
    ///
    /// 通道为空
    Empty,

    /// Channel is closed
    ///
    /// 通道已关闭
    Closed,
}

/// Try-send error type
///
/// 尝试发送错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySendError<T> {
    /// Buffer is full
    ///
    /// 缓冲区已满
    Full(T),

    /// Channel is closed
    ///
    /// 通道已关闭
    Closed(T),
}

impl<T, const N: usize> Sender<T, N> {
    /// Send a message to the channel (async, waits if buffer is full)
    ///
    /// # Errors
    /// Returns `SendError::Closed` if the receiver has been dropped
    ///
    /// 向通道发送消息（异步，如果缓冲区满则等待）
    ///
    /// # 错误
    /// 如果接收器已被丢弃，返回 `SendError::Closed`
    pub async fn send(&self, mut value: T) -> Result<(), SendError<T>> {
        loop {
            match self.try_send(value) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Closed(v)) => return Err(SendError::Closed(v)),
                Err(TrySendError::Full(v)) => {
                    // Store the value to retry
                    // 存储值以便重试
                    value = v;

                    // Wait for space to become available
                    // 等待空间可用
                    self.inner.send_notify.notified().await;

                    // Check if channel was closed while waiting
                    // 检查等待时通道是否已关闭
                    if self.inner.closed.load(Ordering::Acquire) {
                        return Err(SendError::Closed(value));
                    }

                    // Retry with the value in next loop iteration
                    // 在下一次循环迭代中使用该值重试
                }
            }
        }
    }

    /// Try to send a message without blocking
    ///
    /// # Errors
    /// - Returns `TrySendError::Full` if the buffer is full
    /// - Returns `TrySendError::Closed` if the receiver has been dropped
    ///
    /// 尝试非阻塞地发送消息
    ///
    /// # 错误
    /// - 如果缓冲区满，返回 `TrySendError::Full`
    /// - 如果接收器已被丢弃，返回 `TrySendError::Closed`
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        // Check if channel is closed first
        // 首先检查通道是否已关闭
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(TrySendError::Closed(value));
        }

        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // 不会有多个线程同时访问 producer
        self.inner.producer.with_mut(|producer| unsafe {
            match (*producer).push(value) {
                Ok(()) => {
                    // Successfully sent, notify receiver
                    // 成功发送，通知接收者
                    self.inner.recv_notify.notify_one();
                    Ok(())
                }
                Err(PushError::Full(v)) => Err(TrySendError::Full(v)),
            }
        })
    }

    /// Check if the channel is closed
    ///
    /// 检查通道是否已关闭
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Get the capacity of the channel
    ///
    /// 获取通道的容量
    #[inline]
    pub fn capacity(&self) -> usize {
        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // capacity 只读取数据，不需要可变访问
        self.inner
            .producer
            .with(|producer| unsafe { (*producer).capacity() })
    }

    /// Get the number of messages currently in the channel
    ///
    /// 获取通道中当前的消息数量
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // slots 只读取数据，不需要可变访问
        self.inner
            .producer
            .with(|producer| unsafe { (*producer).slots() })
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // is_empty 只读取数据，不需要可变访问
        self.inner
            .producer
            .with(|producer| unsafe { (*producer).is_empty() })
    }

    /// Get the number of free slots in the channel
    ///
    /// 获取通道中的空闲空间数量
    #[inline]
    pub fn free_slots(&self) -> usize {
        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // free_slots 只读取数据，不需要可变访问
        self.inner
            .producer
            .with(|producer| unsafe { (*producer).free_slots() })
    }

    /// Check if the channel is full
    ///
    /// 检查通道是否已满
    #[inline]
    pub fn is_full(&self) -> bool {
        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // is_full 只读取数据，不需要可变访问
        self.inner
            .producer
            .with(|producer| unsafe { (*producer).is_full() })
    }
}

impl<T: Copy, const N: usize> Sender<T, N> {
    /// Try to send multiple values from a slice without blocking
    ///
    /// 尝试非阻塞地从切片发送多个值
    ///
    /// This method attempts to send as many elements as possible from the slice.
    /// It returns the number of elements successfully sent.
    ///
    /// 此方法尝试从切片中发送尽可能多的元素。
    /// 返回成功发送的元素数量。
    ///
    /// # Parameters
    /// - `values`: Slice of values to send
    ///
    /// # Returns
    /// Number of elements successfully sent (0 to values.len())
    ///
    /// # 参数
    /// - `values`: 要发送的值的切片
    ///
    /// # 返回值
    /// 成功发送的元素数量（0 到 values.len()）
    pub fn try_send_slice(&self, values: &[T]) -> usize {
        // Check if channel is closed first
        // 首先检查通道是否已关闭
        if self.inner.closed.load(Ordering::Acquire) {
            return 0;
        }

        // SAFETY: Sender 不实现 Clone，因此只有一个 Sender 实例
        // 不会有多个线程同时访问 producer
        self.inner.producer.with_mut(|producer| unsafe {
            let sent = (*producer).push_slice(values);

            if sent > 0 {
                // Successfully sent some messages, notify receiver
                // 成功发送一些消息，通知接收者
                self.inner.recv_notify.notify_one();
            }

            sent
        })
    }

    /// Send multiple values from a slice (async, waits if buffer is full)
    ///
    /// 从切片发送多个值（异步，如果缓冲区满则等待）
    ///
    /// This method will send all elements from the slice, waiting if necessary
    /// when the buffer becomes full. Returns the number of elements sent, or
    /// an error if the channel is closed.
    ///
    /// 此方法将发送切片中的所有元素，必要时在缓冲区满时等待。
    /// 返回发送的元素数量，如果通道关闭则返回错误。
    ///
    /// # Parameters
    /// - `values`: Slice of values to send
    ///
    /// # Returns
    /// - `Ok(usize)`: Number of elements successfully sent
    /// - `Err(SendError)`: Channel is closed
    ///
    /// # 参数
    /// - `values`: 要发送的值的切片
    ///
    /// # 返回值
    /// - `Ok(usize)`: 成功发送的元素数量
    /// - `Err(SendError)`: 通道已关闭
    pub async fn send_slice(&self, values: &[T]) -> Result<usize, SendError<()>> {
        let mut total_sent = 0;

        while total_sent < values.len() {
            // Check if channel is closed
            // 检查通道是否已关闭
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(SendError::Closed(()));
            }

            let sent = self.try_send_slice(&values[total_sent..]);
            total_sent += sent;

            if total_sent < values.len() {
                // Need to wait for space
                // 需要等待空间
                self.inner.send_notify.notified().await;

                // Check if channel was closed while waiting
                // 检查等待时通道是否已关闭
                if self.inner.closed.load(Ordering::Acquire) {
                    return Err(SendError::Closed(()));
                }
            }
        }

        Ok(total_sent)
    }
}

impl<T, const N: usize> Receiver<T, N> {
    /// Receive a message from the channel (async, waits if buffer is empty)
    ///
    /// Returns `None` if the channel is closed and empty
    ///
    /// 从通道接收消息（异步，如果缓冲区空则等待）
    ///
    /// 如果通道已关闭且为空，返回 `None`
    pub async fn recv(&self) -> Option<T> {
        loop {
            match self.try_recv() {
                Ok(value) => return Some(value),
                Err(TryRecvError::Closed) => return None,
                Err(TryRecvError::Empty) => {
                    // Check if channel is closed before waiting
                    // 等待前检查通道是否已关闭
                    if self.inner.closed.load(Ordering::Acquire) {
                        // Double check if there are any remaining items
                        // 再次检查是否有剩余项
                        if let Ok(value) = self.try_recv() {
                            return Some(value);
                        }
                        return None;
                    }

                    // Wait for data to become available
                    // 等待数据可用
                    self.inner.recv_notify.notified().await;
                }
            }
        }
    }

    /// Try to receive a message without blocking
    ///
    /// # Errors
    /// - Returns `TryRecvError::Empty` if the buffer is empty
    /// - Returns `TryRecvError::Closed` if the sender has been dropped and buffer is empty
    ///
    /// 尝试非阻塞地接收消息
    ///
    /// # 错误
    /// - 如果缓冲区空，返回 `TryRecvError::Empty`
    /// - 如果发送器已被丢弃且缓冲区空，返回 `TryRecvError::Closed`
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // 不会有多个线程同时访问 consumer
        self.inner.consumer.with_mut(|consumer| unsafe {
            match (*consumer).pop() {
                Ok(value) => {
                    // Successfully received, notify sender
                    // 成功接收，通知发送者
                    self.inner.send_notify.notify_one();
                    Ok(value)
                }
                Err(PopError::Empty) => {
                    // Check if channel is closed
                    // 检查通道是否已关闭
                    if self.inner.closed.load(Ordering::Acquire) {
                        // Double-check for remaining items to avoid race condition
                        // where sender drops after push but before we check closed flag
                        // 再次检查是否有剩余项，以避免发送方在 push 后但在我们检查 closed 标志前 drop 的竞态条件
                        match (*consumer).pop() {
                            Ok(value) => {
                                self.inner.send_notify.notify_one();
                                Ok(value)
                            }
                            Err(PopError::Empty) => Err(TryRecvError::Closed),
                        }
                    } else {
                        Err(TryRecvError::Empty)
                    }
                }
            }
        })
    }

    /// Check if the channel is empty
    ///
    /// 检查通道是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // is_empty 只读取数据，不需要可变访问，但我们通过 UnsafeCell 访问以保持一致性
        self.inner
            .consumer
            .with(|consumer| unsafe { (*consumer).is_empty() })
    }

    /// Get the number of messages currently in the channel
    ///
    /// 获取通道中当前的消息数量
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // slots 只读取数据，不需要可变访问，但我们通过 UnsafeCell 访问以保持一致性
        self.inner
            .consumer
            .with(|consumer| unsafe { (*consumer).slots() })
    }

    /// Get the capacity of the channel
    ///
    /// 获取通道的容量
    #[inline]
    pub fn capacity(&self) -> usize {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // capacity 只读取数据，不需要可变访问，但我们通过 UnsafeCell 访问以保持一致性
        self.inner
            .consumer
            .with(|consumer| unsafe { (*consumer).buffer().capacity() })
    }

    /// Peek at the first message without removing it
    ///
    /// 查看第一个消息但不移除它
    ///
    /// # Returns
    /// `Some(&T)` if there is a message, `None` if the channel is empty
    ///
    /// # 返回值
    /// 如果有消息则返回 `Some(&T)`，如果通道为空则返回 `None`
    ///
    /// # Safety
    /// The returned reference is valid only as long as no other operations
    /// are performed on the Receiver that might modify the buffer.
    ///
    /// # 安全性
    /// 返回的引用仅在未对 Receiver 执行可能修改缓冲区的其他操作时有效。
    #[inline]
    pub fn peek(&self) -> Option<&T> {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // peek 只读取数据，不需要可变访问
        self.inner
            .consumer
            .with(|consumer| unsafe { std::mem::transmute((*consumer).peek()) })
    }

    /// Clear all messages from the channel
    ///
    /// 清空通道中的所有消息
    ///
    /// This method pops and drops all messages currently in the channel.
    ///
    /// 此方法弹出并 drop 通道中当前的所有消息。
    pub fn clear(&mut self) {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // 我们有可变引用，因此可以安全地访问 consumer
        self.inner
            .consumer
            .with_mut(|consumer| unsafe { (*consumer).clear() });

        // Notify sender that space is available
        // 通知发送者空间可用
        self.inner.send_notify.notify_one();
    }

    /// Create a draining iterator
    ///
    /// 创建一个消费迭代器
    ///
    /// Returns an iterator that removes and returns messages from the channel.
    /// The iterator will continue until the channel is empty.
    ///
    /// 返回一个从通道中移除并返回消息的迭代器。
    /// 迭代器将持续运行直到通道为空。
    ///
    /// # Examples
    ///
    /// ```
    /// use lite_sync::spsc::channel;
    /// use std::num::NonZeroUsize;
    ///
    ///     # #[tokio::main]
    ///     # async fn main() {
    ///     #[cfg(not(feature = "loom"))]
    ///     {
    ///         let (tx, mut rx) = channel::<i32, 8>(NonZeroUsize::new(32).unwrap());
    ///         tx.try_send(1).unwrap();
    ///         tx.try_send(2).unwrap();
    ///         tx.try_send(3).unwrap();
    ///
    ///         let items: Vec<i32> = rx.drain().collect();
    ///         assert_eq!(items, vec![1, 2, 3]);
    ///         assert!(rx.is_empty());
    ///     }
    ///     # }
    /// ```
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T, N> {
        Drain { receiver: self }
    }
}

impl<T: Copy, const N: usize> Receiver<T, N> {
    /// Try to receive multiple values into a slice without blocking
    ///
    /// 尝试非阻塞地将多个值接收到切片
    ///
    /// This method attempts to receive as many messages as possible into the provided slice.
    /// It returns the number of messages successfully received.
    ///
    /// 此方法尝试将尽可能多的消息接收到提供的切片中。
    /// 返回成功接收的消息数量。
    ///
    /// # Parameters
    /// - `dest`: Destination slice to receive values into
    ///
    /// # Returns
    /// Number of messages successfully received (0 to dest.len())
    ///
    /// # 参数
    /// - `dest`: 用于接收值的目标切片
    ///
    /// # 返回值
    /// 成功接收的消息数量（0 到 dest.len()）
    pub fn try_recv_slice(&mut self, dest: &mut [T]) -> usize {
        // SAFETY: Receiver 不实现 Clone，因此只有一个 Receiver 实例
        // 我们有可变引用，因此可以安全地访问 consumer
        self.inner.consumer.with_mut(|consumer| unsafe {
            let received = (*consumer).pop_slice(dest);

            if received > 0 {
                // Successfully received some messages, notify sender
                // 成功接收一些消息，通知发送者
                self.inner.send_notify.notify_one();
            }

            received
        })
    }

    /// Receive multiple values into a slice (async, waits if buffer is empty)
    ///
    /// 将多个值接收到切片（异步，如果缓冲区空则等待）
    ///
    /// This method will fill the destination slice as much as possible, waiting if necessary
    /// when the buffer becomes empty. Returns the number of messages received.
    ///
    /// 此方法将尽可能填充目标切片，必要时在缓冲区空时等待。
    /// 返回接收的消息数量。
    ///
    /// # Parameters
    /// - `dest`: Destination slice to receive values into
    ///
    /// # Returns
    /// Number of messages successfully received (0 to dest.len())
    /// Returns 0 if the channel is closed and empty
    ///
    /// # 参数
    /// - `dest`: 用于接收值的目标切片
    ///
    /// # 返回值
    /// 成功接收的消息数量（0 到 dest.len()）
    /// 如果通道已关闭且为空，返回 0
    pub async fn recv_slice(&mut self, dest: &mut [T]) -> usize {
        let mut total_received = 0;

        while total_received < dest.len() {
            let received = self.try_recv_slice(&mut dest[total_received..]);
            total_received += received;

            if total_received < dest.len() {
                // Check if channel is closed
                // 检查通道是否已关闭
                if self.inner.closed.load(Ordering::Acquire) {
                    // Channel closed, return what we have
                    // 通道已关闭，返回我们已有的内容
                    return total_received;
                }

                // Need to wait for data
                // 需要等待数据
                self.inner.recv_notify.notified().await;

                // Check again after waking up
                // 唤醒后再次检查
                if self.inner.closed.load(Ordering::Acquire) {
                    // Try one more time to get remaining messages
                    // 再尝试一次获取剩余消息
                    let final_received = self.try_recv_slice(&mut dest[total_received..]);
                    total_received += final_received;
                    return total_received;
                }
            }
        }

        total_received
    }
}

impl<T, const N: usize> Drop for Receiver<T, N> {
    fn drop(&mut self) {
        // Mark channel as closed when receiver is dropped
        // 当接收器被丢弃时标记通道为已关闭
        self.inner.closed.store(true, Ordering::Release);

        // Notify sender in case it's waiting
        // 通知发送者以防它正在等待
        self.inner.send_notify.notify_one();
    }
}

impl<T, const N: usize> Drop for Sender<T, N> {
    fn drop(&mut self) {
        // Mark channel as closed when sender is dropped
        // 当发送器被丢弃时标记通道为已关闭
        self.inner.closed.store(true, Ordering::Release);

        // Notify receiver in case it's waiting
        // 通知接收器以防它正在等待
        self.inner.recv_notify.notify_one();
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_send_recv() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(4).unwrap());

        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();
        tx.send(3).await.unwrap();

        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, Some(2));
        assert_eq!(rx.recv().await, Some(3));
    }

    #[tokio::test]
    async fn test_try_send_recv() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(4).unwrap());

        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();

        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv().unwrap(), 2);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn test_channel_closed_on_sender_drop() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(4).unwrap());

        tx.send(1).await.unwrap();
        drop(tx);

        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test]
    async fn test_channel_closed_on_receiver_drop() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(4).unwrap());

        drop(rx);

        assert!(matches!(tx.send(1).await, Err(SendError::Closed(1))));
    }

    #[tokio::test]
    async fn test_cross_task_communication() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(4).unwrap());

        let sender_handle = tokio::spawn(async move {
            for i in 0..10 {
                tx.send(i).await.unwrap();
            }
        });

        let receiver_handle = tokio::spawn(async move {
            let mut sum = 0;
            while let Some(value) = rx.recv().await {
                sum += value;
            }
            sum
        });

        sender_handle.await.unwrap();
        let sum = receiver_handle.await.unwrap();
        assert_eq!(sum, 45); // 0+1+2+...+9 = 45
    }

    #[tokio::test]
    async fn test_backpressure() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(4).unwrap());

        // Fill the buffer
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        tx.try_send(3).unwrap();
        tx.try_send(4).unwrap();

        // Buffer should be full now
        assert!(matches!(tx.try_send(5), Err(TrySendError::Full(5))));

        // This should block and then succeed when we consume
        let send_handle = tokio::spawn(async move {
            tx.send(5).await.unwrap();
            tx.send(6).await.unwrap();
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, Some(2));
        assert_eq!(rx.recv().await, Some(3));
        assert_eq!(rx.recv().await, Some(4));
        assert_eq!(rx.recv().await, Some(5));
        assert_eq!(rx.recv().await, Some(6));

        send_handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_capacity_and_len() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(8).unwrap());

        assert_eq!(rx.capacity(), 8);
        assert_eq!(rx.len(), 0);
        assert!(rx.is_empty());

        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        assert_eq!(rx.len(), 2);
        assert!(!rx.is_empty());
    }

    // ==================== New API Tests ====================

    #[tokio::test]
    async fn test_sender_capacity_queries() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(8).unwrap());

        assert_eq!(tx.capacity(), 8);
        assert_eq!(tx.len(), 0);
        assert_eq!(tx.free_slots(), 8);
        assert!(!tx.is_full());

        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        tx.try_send(3).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        assert_eq!(tx.len(), 3);
        assert_eq!(tx.free_slots(), 5);
        assert!(!tx.is_full());

        // Fill the buffer
        tx.try_send(4).unwrap();
        tx.try_send(5).unwrap();
        tx.try_send(6).unwrap();
        tx.try_send(7).unwrap();
        tx.try_send(8).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        assert_eq!(tx.len(), 8);
        assert_eq!(tx.free_slots(), 0);
        assert!(tx.is_full());

        // Pop one and check again
        rx.recv().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        assert_eq!(tx.len(), 7);
        assert_eq!(tx.free_slots(), 1);
        assert!(!tx.is_full());
    }

    #[tokio::test]
    async fn test_try_send_slice() {
        let (tx, rx) = channel::<u32, 32>(NonZeroUsize::new(16).unwrap());

        let data = [1, 2, 3, 4, 5];
        let sent = tx.try_send_slice(&data);

        assert_eq!(sent, 5);
        assert_eq!(rx.len(), 5);

        for i in 0..5 {
            assert_eq!(rx.recv().await.unwrap(), data[i]);
        }
    }

    #[tokio::test]
    async fn test_try_send_slice_partial() {
        let (tx, rx) = channel::<u32, 32>(NonZeroUsize::new(8).unwrap());

        // Fill with 5 elements, leaving room for 3
        let initial = [1, 2, 3, 4, 5];
        tx.try_send_slice(&initial);

        // Try to send 10 more, should only send 3
        let more = [6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let sent = tx.try_send_slice(&more);

        assert_eq!(sent, 3);
        assert_eq!(rx.len(), 8);
        assert!(tx.is_full());

        // Verify values
        for i in 1..=8 {
            assert_eq!(rx.recv().await.unwrap(), i);
        }
    }

    #[tokio::test]
    async fn test_send_slice() {
        let (tx, rx) = channel::<u32, 32>(NonZeroUsize::new(16).unwrap());

        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let result = tx.send_slice(&data).await;

        assert_eq!(result.unwrap(), 10);
        assert_eq!(rx.len(), 10);

        for i in 0..10 {
            assert_eq!(rx.recv().await.unwrap(), data[i]);
        }
    }

    #[tokio::test]
    async fn test_send_slice_with_backpressure() {
        let (tx, rx) = channel::<u32, 32>(NonZeroUsize::new(4).unwrap());

        let data = [1, 2, 3, 4, 5, 6, 7, 8];

        let send_handle = tokio::spawn(async move { tx.send_slice(&data).await.unwrap() });

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Consume some messages to make room
        for i in 1..=4 {
            assert_eq!(rx.recv().await.unwrap(), i);
        }

        let sent = send_handle.await.unwrap();
        assert_eq!(sent, 8);

        // Verify remaining messages
        for i in 5..=8 {
            assert_eq!(rx.recv().await.unwrap(), i);
        }
    }

    #[tokio::test]
    async fn test_peek() {
        let (tx, rx) = channel::<i32, 32>(NonZeroUsize::new(8).unwrap());

        // Peek empty buffer
        assert!(rx.peek().is_none());

        tx.try_send(42).unwrap();
        tx.try_send(100).unwrap();
        tx.try_send(200).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        // Peek should return first element without removing it
        assert_eq!(rx.peek(), Some(&42));
        assert_eq!(rx.peek(), Some(&42)); // Peek again, should be same
        assert_eq!(rx.len(), 3); // Length unchanged
    }

    #[tokio::test]
    async fn test_peek_after_recv() {
        let (tx, rx) = channel::<String, 32>(NonZeroUsize::new(8).unwrap());

        tx.try_send("first".to_string()).unwrap();
        tx.try_send("second".to_string()).unwrap();
        tx.try_send("third".to_string()).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        assert_eq!(rx.peek(), Some(&"first".to_string()));
        rx.recv().await.unwrap();

        assert_eq!(rx.peek(), Some(&"second".to_string()));
        rx.recv().await.unwrap();

        assert_eq!(rx.peek(), Some(&"third".to_string()));
        rx.recv().await.unwrap();

        assert!(rx.peek().is_none());
    }

    #[tokio::test]
    async fn test_clear() {
        let (tx, mut rx) = channel::<i32, 32>(NonZeroUsize::new(16).unwrap());

        for i in 0..10 {
            tx.try_send(i).unwrap();
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        assert_eq!(rx.len(), 10);

        rx.clear();

        assert_eq!(rx.len(), 0);
        assert!(rx.is_empty());
    }

    #[tokio::test]
    async fn test_clear_with_drop() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        #[derive(Debug)]
        struct DropCounter {
            counter: Arc<AtomicUsize>,
        }

        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.counter.fetch_add(1, AtomicOrdering::SeqCst);
            }
        }

        let counter = Arc::new(AtomicUsize::new(0));

        {
            let (tx, mut rx) = channel::<DropCounter, 32>(NonZeroUsize::new(16).unwrap());

            for _ in 0..8 {
                tx.try_send(DropCounter {
                    counter: counter.clone(),
                })
                .unwrap();
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            assert_eq!(counter.load(AtomicOrdering::SeqCst), 0);

            rx.clear();

            assert_eq!(counter.load(AtomicOrdering::SeqCst), 8);
        }
    }

    #[tokio::test]
    async fn test_drain() {
        let (tx, mut rx) = channel::<i32, 32>(NonZeroUsize::new(16).unwrap());

        for i in 0..10 {
            tx.try_send(i).unwrap();
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        let collected: Vec<i32> = rx.drain().collect();

        assert_eq!(collected, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(rx.is_empty());
    }

    #[tokio::test]
    async fn test_drain_empty() {
        let (_tx, mut rx) = channel::<i32, 32>(NonZeroUsize::new(8).unwrap());

        let collected: Vec<i32> = rx.drain().collect();

        assert!(collected.is_empty());
    }

    #[tokio::test]
    async fn test_drain_size_hint() {
        let (tx, mut rx) = channel::<i32, 32>(NonZeroUsize::new(16).unwrap());

        for i in 0..5 {
            tx.try_send(i).unwrap();
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        let mut drain = rx.drain();

        assert_eq!(drain.size_hint(), (5, Some(5)));

        drain.next();
        assert_eq!(drain.size_hint(), (4, Some(4)));

        drain.next();
        assert_eq!(drain.size_hint(), (3, Some(3)));
    }

    #[tokio::test]
    async fn test_try_recv_slice() {
        let (tx, mut rx) = channel::<u32, 32>(NonZeroUsize::new(16).unwrap());

        // Send some data
        for i in 0..10 {
            tx.try_send(i).unwrap();
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        let mut dest = [0u32; 5];
        let received = rx.try_recv_slice(&mut dest);

        assert_eq!(received, 5);
        assert_eq!(dest, [0, 1, 2, 3, 4]);
        assert_eq!(rx.len(), 5);
    }

    #[tokio::test]
    async fn test_try_recv_slice_partial() {
        let (tx, mut rx) = channel::<u32, 32>(NonZeroUsize::new(16).unwrap());

        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        tx.try_send(3).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        let mut dest = [0u32; 10];
        let received = rx.try_recv_slice(&mut dest);

        assert_eq!(received, 3);
        assert_eq!(&dest[0..3], &[1, 2, 3]);
        assert!(rx.is_empty());
    }

    #[tokio::test]
    async fn test_recv_slice() {
        let (tx, mut rx) = channel::<u32, 32>(NonZeroUsize::new(16).unwrap());

        for i in 1..=10 {
            tx.try_send(i).unwrap();
        }

        let mut dest = [0u32; 10];
        let received = rx.recv_slice(&mut dest).await;

        assert_eq!(received, 10);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert!(rx.is_empty());
    }

    #[tokio::test]
    async fn test_recv_slice_with_wait() {
        let (tx, mut rx) = channel::<u32, 32>(NonZeroUsize::new(4).unwrap());

        let recv_handle = tokio::spawn(async move {
            let mut dest = [0u32; 8];
            let received = rx.recv_slice(&mut dest).await;
            (received, dest)
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Send data gradually
        for i in 1..=8 {
            tx.send(i).await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }

        let (received, dest) = recv_handle.await.unwrap();
        assert_eq!(received, 8);
        assert_eq!(dest, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[tokio::test]
    async fn test_recv_slice_channel_closed() {
        let (tx, mut rx) = channel::<u32, 32>(NonZeroUsize::new(8).unwrap());

        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        tx.try_send(3).unwrap();

        drop(tx); // Close the channel

        let mut dest = [0u32; 10];
        let received = rx.recv_slice(&mut dest).await;

        // Should receive the 3 available messages, then stop
        assert_eq!(received, 3);
        assert_eq!(&dest[0..3], &[1, 2, 3]);
    }

    #[tokio::test]
    async fn test_combined_new_apis() {
        let (tx, mut rx) = channel::<u32, 32>(NonZeroUsize::new(16).unwrap());

        // Batch send
        let data = [1, 2, 3, 4, 5];
        tx.try_send_slice(&data);

        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        assert_eq!(tx.len(), 5);
        assert_eq!(rx.len(), 5);
        assert_eq!(rx.capacity(), 16);

        // Peek
        assert_eq!(rx.peek(), Some(&1));

        // Batch receive
        let mut dest = [0u32; 3];
        rx.try_recv_slice(&mut dest);
        assert_eq!(dest, [1, 2, 3]);

        assert_eq!(rx.len(), 2);
        assert_eq!(tx.free_slots(), 14);

        // Drain remaining
        let remaining: Vec<u32> = rx.drain().collect();
        assert_eq!(remaining, vec![4, 5]);

        assert!(rx.is_empty());
        assert!(!tx.is_full());
    }
}
