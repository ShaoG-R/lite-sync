//! # lite-sync
//! 
//! Lightweight, high-performance async synchronization primitives for Rust.
//! 
//! 轻量级、高性能的 Rust 异步同步原语库。
//! 
//! ## Overview / 概述
//! 
//! `lite-sync` provides a collection of optimized synchronization primitives designed for
//! low latency and minimal allocations. These primitives are built from the ground up with
//! performance in mind, offering alternatives to heavier standard library implementations.
//! 
//! `lite-sync` 提供了一系列优化的同步原语，专为低延迟和最小分配而设计。
//! 这些原语从头开始构建，以性能为核心，为更重的标准库实现提供替代方案。
//! 
//! ## Key Features / 主要特性
//! 
//! - **Zero or minimal allocations**: Most primitives avoid heap allocations entirely
//! - **Lock-free algorithms**: Using atomic operations for maximum concurrency
//! - **Single-waiter optimization**: Specialized for common SPSC (Single Producer Single Consumer) patterns
//! - **Inline storage**: Support for stack-allocated buffers to avoid heap allocations
//! 
//! - **零或最小分配**：大多数原语完全避免堆分配
//! - **无锁算法**：使用原子操作实现最大并发性
//! - **单等待者优化**：专为常见的 SPSC（单生产者单消费者）模式优化
//! - **内联存储**：支持栈分配缓冲区以避免堆分配
//! 
//! ## Modules / 模块
//! 
//! ### [`oneshot`]
//! 
//! One-shot completion notification with customizable state.
//! 
//! 带有可自定义状态的一次性完成通知。
//! 
//! Perfect for signaling task completion with minimal overhead. Supports custom state types
//! through the [`oneshot::State`] trait, allowing you to communicate not just "done" but
//! also "how it finished" (success, failure, timeout, etc.).
//! 
//! 非常适合以最小开销发出任务完成信号。通过 [`oneshot::State`] trait 支持自定义状态类型，
//! 允许您不仅传达"完成"，还能传达"如何完成"（成功、失败、超时等）。
//! 
//! **Key optimizations / 关键优化**:
//! - Zero Box allocation for waker storage / Waker 存储零 Box 分配
//! - Direct `Future` implementation for ergonomic `.await` / 直接实现 `Future` 以支持便捷的 `.await`
//! - Fast path for immediate completion / 立即完成的快速路径
//! 
//! ### [`spsc`]
//! 
//! High-performance async SPSC (Single Producer Single Consumer) channel.
//! 
//! 高性能异步 SPSC（单生产者单消费者）通道。
//! 
//! Built on `smallring` for efficient ring buffer operations with inline storage support.
//! Type-safe enforcement of single producer/consumer semantics eliminates synchronization overhead.
//! 
//! 基于 `smallring` 构建，支持内联存储的高效环形缓冲区操作。
//! 类型安全地强制单生产者/消费者语义，消除同步开销。
//! 
//! **Key optimizations / 关键优化**:
//! - Zero-cost interior mutability using `UnsafeCell` / 使用 `UnsafeCell` 实现零成本内部可变性
//! - Inline buffer support for small channels / 小容量通道的内联缓冲区支持
//! - Batch send/receive operations / 批量发送/接收操作
//! - Single-waiter notification / 单等待者通知
//! 
//! ### [`notify`]
//! 
//! Lightweight single-waiter notification primitive.
//! 
//! 轻量级单等待者通知原语。
//! 
//! Much lighter than `tokio::sync::Notify` when you only need to wake one task at a time.
//! Ideal for internal synchronization in other primitives.
//! 
//! 当您每次只需唤醒一个任务时，比 `tokio::sync::Notify` 更轻量。
//! 非常适合在其他原语中进行内部同步。
//! 
//! ### [`atomic_waker`]
//! 
//! Atomic waker storage with state machine synchronization.
//! 
//! 带有状态机同步的原子 waker 存储。
//! 
//! Based on Tokio's `AtomicWaker` but simplified for specific use cases.
//! Provides safe concurrent access to a waker without Box allocation.
//! 
//! 基于 Tokio 的 `AtomicWaker` 但为特定用例简化。
//! 提供对 waker 的安全并发访问，无需 Box 分配。
//! 
//! ## Examples / 示例
//! 
//! ### One-shot completion with custom state
//! 
//! ```
//! use lite_sync::oneshot::lite::{State, Sender};
//! 
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! enum TaskResult {
//!     Success,
//!     Error,
//! }
//! 
//! impl State for TaskResult {
//!     fn to_u8(&self) -> u8 {
//!         match self {
//!             TaskResult::Success => 1,
//!             TaskResult::Error => 2,
//!         }
//!     }
//!     
//!     fn from_u8(value: u8) -> Option<Self> {
//!         match value {
//!             1 => Some(TaskResult::Success),
//!             2 => Some(TaskResult::Error),
//!             _ => None,
//!         }
//!     }
//!     
//!     fn pending_value() -> u8 { 0 }
//!     fn closed_value() -> u8 { 255 }
//!     fn receiver_closed_value() -> u8 { 254 }
//! }
//! 
//! # tokio_test::block_on(async {
//! let (sender, receiver) = Sender::<TaskResult>::new();
//! 
//! tokio::spawn(async move {
//!     // Do some work...
//!     sender.send(TaskResult::Success);
//! });
//! 
//! let result = receiver.await;
//! assert_eq!(result, Ok(TaskResult::Success));
//! # });
//! ```
//! 
//! ### SPSC channel with inline storage
//! 
//! ```
//! use lite_sync::spsc::channel;
//! use std::num::NonZeroUsize;
//! 
//! # tokio_test::block_on(async {
//! // Channel with capacity 32, inline buffer size 8
//! let (tx, rx) = channel::<i32, 8>(NonZeroUsize::new(32).unwrap());
//! 
//! tokio::spawn(async move {
//!     for i in 0..10 {
//!         tx.send(i).await.unwrap();
//!     }
//! });
//! 
//! let mut sum = 0;
//! while let Some(value) = rx.recv().await {
//!     sum += value;
//! }
//! assert_eq!(sum, 45); // 0+1+2+...+9
//! # });
//! ```
//! 
//! ### Single-waiter notification
//! 
//! ```
//! use lite_sync::notify::SingleWaiterNotify;
//! use std::sync::Arc;
//! 
//! # tokio_test::block_on(async {
//! let notify = Arc::new(SingleWaiterNotify::new());
//! let notify_clone = notify.clone();
//! 
//! tokio::spawn(async move {
//!     // Do some work...
//!     notify_clone.notify_one();
//! });
//! 
//! notify.notified().await;
//! # });
//! ```
//! 
//! ## Safety / 安全性
//! 
//! All primitives use `unsafe` internally for performance but expose safe APIs.
//! Safety is guaranteed through:
//! 
//! 所有原语在内部使用 `unsafe` 以提高性能，但暴露安全的 API。
//! 安全性通过以下方式保证：
//! 
//! - Type system enforcement of single ownership (no `Clone` on SPSC endpoints)
//! - Atomic state machines for synchronization
//! - Careful ordering of atomic operations
//! - Comprehensive test coverage including concurrent scenarios
//! 
//! - 类型系统强制单一所有权（SPSC 端点不实现 `Clone`）
//! - 用于同步的原子状态机
//! - 原子操作的仔细排序
//! - 全面的测试覆盖，包括并发场景

pub mod notify;
pub mod atomic_waker;
pub mod oneshot;
pub mod spsc;