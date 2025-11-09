/// One-shot channel implementations for single-use value transfer
/// 
/// This module provides two variants of one-shot channels optimized for different use cases:
/// 
/// 一次性通道实现，用于单次值传递
/// 
/// 此模块提供两种针对不同使用场景优化的一次性通道变体：
/// 
/// # Variants | 变体
/// 
/// ## `lite` - Lightweight state-based oneshot
/// 
/// Optimized for small copyable states that can be encoded as u8.
/// Zero heap allocation for the value itself (stored in AtomicU8).
/// Perfect for completion notifications, simple enums, and boolean states.
/// 
/// 为可编码为 u8 的小型可复制状态优化。
/// 值本身零堆分配（存储在 AtomicU8 中）。
/// 非常适合完成通知、简单枚举和布尔状态。
/// 
/// ### Use cases | 使用场景:
/// - Task completion notification (unit type `()`)
/// - Small state machines (custom enums with `State` trait)
/// - Boolean flags and simple status codes
/// 
/// ### Example:
/// 
/// ```
/// use lite_sync::oneshot::lite::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (sender, receiver) = Sender::<()>::new();
/// 
/// tokio::spawn(async move {
///     // ... do work ...
///     sender.notify(());
/// });
/// 
/// receiver.await; // Wait for completion
/// # });
/// ```
/// 
/// ## `generic` - Generic value transfer oneshot
/// 
/// Supports arbitrary types with zero extra heap allocation for the value.
/// Uses `UnsafeCell<MaybeUninit<T>>` + `AtomicU8` for lock-free value transfer.
/// Value is stored directly in the structure, avoiding Box allocation.
/// Ideal for transferring ownership of any data between tasks.
/// 
/// 支持任意类型，值无需额外堆分配。
/// 使用 `UnsafeCell<MaybeUninit<T>>` + `AtomicU8` 实现无锁值传递。
/// 值直接存储在结构体中，避免 Box 分配。
/// 非常适合在任务间传递任意数据的所有权。
/// 
/// ### Use cases | 使用场景:
/// - Request-response patterns with complex types
/// - Transferring ownership of heap-allocated data
/// - Sending results of arbitrary computations
/// 
/// ### Example:
/// 
/// ```
/// use lite_sync::oneshot::generic::Sender;
/// 
/// # tokio_test::block_on(async {
/// let (sender, receiver) = Sender::<String>::new();
/// 
/// tokio::spawn(async move {
///     let result = "Hello, World!".to_string();
///     sender.send(result).unwrap();
/// });
/// 
/// let message = receiver.await;
/// assert_eq!(message, "Hello, World!");
/// # });
/// ```
/// 
/// # Performance Characteristics | 性能特点
/// 
/// ## `lite`
/// - **Allocation**: Zero heap allocation for values
/// - **Size**: Single `AtomicU8` + `AtomicWaker`
/// - **Latency**: Minimal - direct atomic operations
/// - **Best for**: High-frequency notifications
/// 
/// ## `generic`
/// - **Allocation**: Zero heap allocation (value stored inline)
/// - **Size**: `UnsafeCell<MaybeUninit<T>>` + `AtomicU8` + `AtomicWaker`
/// - **Latency**: Low - single atomic state transition
/// - **Best for**: Arbitrary data transfer
/// 
/// # Thread Safety | 线程安全
/// 
/// Both implementations are fully thread-safe and lock-free:
/// - Sender can be called from any thread
/// - Receiver can be awaited from any task
/// - No spinlocks or blocking operations
/// 
/// 两种实现都是完全线程安全且无锁的：
/// - Sender 可以从任何线程调用
/// - Receiver 可以从任何任务 await
/// - 无自旋锁或阻塞操作

pub mod lite;
pub mod generic;