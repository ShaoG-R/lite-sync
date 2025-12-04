# lite-sync

[![Crates.io](https://img.shields.io/crates/v/lite-sync.svg)](https://crates.io/crates/lite-sync)
[![Documentation](https://docs.rs/lite-sync/badge.svg)](https://docs.rs/lite-sync)
[![License](https://img.shields.io/crates/l/lite-sync.svg)](https://github.com/ShaoG-R/lite-sync#license)

Fast, lightweight async primitives: SPSC channel, oneshot, notify, and atomic waker.

[📖 English](README.md) | [📖 中文文档](README_CN.md)

## Overview

`lite-sync` provides a collection of optimized synchronization primitives designed for low latency and minimal allocations. These primitives are built from the ground up with performance in mind, offering alternatives to heavier standard library implementations.

## Features

- **Zero or minimal allocations**: Most primitives avoid heap allocations entirely
- **Lock-free algorithms**: Using atomic operations for maximum concurrency
- **Single-waiter optimization**: Specialized for common SPSC (Single Producer Single Consumer) patterns
- **Inline storage**: Support for stack-allocated buffers to avoid heap allocations
- **Type-safe**: Leverages Rust's type system to enforce correctness at compile time

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
lite-sync = "0.1"
```

## Modules

### `oneshot`

One-shot completion notification with customizable state.

Perfect for signaling task completion with minimal overhead. Supports custom state types through the `State` trait, allowing you to communicate not just "done" but also "how it finished" (success, failure, timeout, etc.).

**Key features**:
- Returns `Result<T, RecvError>` - detects when sender is dropped
- Provides `recv()` async method and `try_recv()` non-blocking method
- Zero Box allocation for waker storage
- Direct `Future` implementation for ergonomic `.await`
- Fast path for immediate completion

### `spsc`

High-performance async SPSC (Single Producer Single Consumer) channel.

Built on `smallring` for efficient ring buffer operations with inline storage support. Type-safe enforcement of single producer/consumer semantics eliminates synchronization overhead.

**Key optimizations**:
- Zero-cost interior mutability using `UnsafeCell`
- Inline buffer support for small channels
- Batch send/receive operations
- Single-waiter notification

### `notify`

Lightweight single-waiter notification primitive.

Much lighter than `tokio::sync::Notify` when you only need to wake one task at a time. Ideal for internal synchronization in other primitives.

### `atomic_waker`

Atomic waker storage with state machine synchronization.

Based on Tokio's `AtomicWaker` but simplified for specific use cases. Provides safe concurrent access to a waker without Box allocation.

## Examples

### One-shot completion with custom state

```rust
use lite_sync::oneshot::lite::{State, Sender};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskResult {
    Success,
    Error,
}

impl State for TaskResult {
    fn to_u8(&self) -> u8 {
        match self {
            TaskResult::Success => 1,
            TaskResult::Error => 2,
        }
    }
    
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(TaskResult::Success),
            2 => Some(TaskResult::Error),
            _ => None,
        }
    }
    
    fn pending_value() -> u8 { 0 }
    fn closed_value() -> u8 { 255 }
}

#[tokio::main]
async fn main() {
    let (sender, receiver) = Sender::<TaskResult>::new();
    
    tokio::spawn(async move {
        // Do some work...
        sender.notify(TaskResult::Success);
    });
    
    // Use recv() to receive asynchronously, or direct .await
    match receiver.recv().await {
        Ok(TaskResult::Success) => println!("Task succeeded"),
        Ok(TaskResult::Error) => println!("Task failed"),
        Err(_) => println!("Sender dropped"),
    }
}
```

### SPSC channel with inline storage

```rust
use lite_sync::spsc::channel;
use std::num::NonZeroUsize;

#[tokio::main]
async fn main() {
    // Channel with capacity 32, inline buffer size 8
    let (tx, rx) = channel::<i32, 8>(NonZeroUsize::new(32).unwrap());
    
    tokio::spawn(async move {
        for i in 0..10 {
            tx.send(i).await.unwrap();
        }
    });
    
    let mut sum = 0;
    while let Some(value) = rx.recv().await {
        sum += value;
    }
    assert_eq!(sum, 45); // 0+1+2+...+9
}
```

### Single-waiter notification

```rust
use lite_sync::notify::SingleWaiterNotify;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let notify = Arc::new(SingleWaiterNotify::new());
    let notify_clone = notify.clone();
    
    tokio::spawn(async move {
        // Do some work...
        notify_clone.notify_one();
    });
    
    notify.notified().await;
}
```

### Simple completion notification (unit type)

```rust
use lite_sync::oneshot::lite::Sender;

#[tokio::main]
async fn main() {
    let (sender, receiver) = Sender::<()>::new();
    
    tokio::spawn(async move {
        // Task completes
        sender.notify(());
    });
    
    // Use recv() or direct .await, returns Result
    match receiver.recv().await {
        Ok(()) => println!("Task completed"),
        Err(_) => println!("Sender dropped"),
    }
}
```

## Benchmarks

Performance benchmarks are available in the `benches/` directory. Run them with:

```bash
cargo bench
```

Key characteristics:
- **Oneshot**: Extremely fast for immediate completion, optimized async wait path
- **SPSC**: Low latency per-message overhead with efficient batch operations
- **Notify**: Minimal notification roundtrip time

## Safety

All primitives use `unsafe` internally for performance but expose safe APIs. Safety is guaranteed through:

- **Type system enforcement** of single ownership (no `Clone` on SPSC endpoints)
- **Atomic state machines** for synchronization
- **Careful ordering** of atomic operations
- **Comprehensive test coverage** including concurrent scenarios

## Minimum Supported Rust Version (MSRV)

Rust 2024 edition (Rust 1.85.0 or later)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

