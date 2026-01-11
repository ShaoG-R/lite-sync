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
- **no_std support**: Compatible with `no_std` environments (requires `alloc`)

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
lite-sync = "0.2"
```

### no_std

`lite-sync` supports `no_std` environments (requires `alloc`). Disable default features in `Cargo.toml`:

```toml
[dependencies]
lite-sync = { version = "0.2", default-features = false }
```

## Modules

### `oneshot`

One-shot channel for sending a single value between tasks, with **API behavior aligned with tokio::sync::oneshot**.

Provides two variants:
- **`oneshot::generic`** - For arbitrary types `T: Send`, uses `UnsafeCell<MaybeUninit<T>>` for storage
- **`oneshot::lite`** - Ultra-lightweight variant for `State`-encodable types, uses only `AtomicU8` for storage

**API (aligned with tokio oneshot)**:
- `channel<T>()` - Create a sender/receiver pair
- `Sender::send(value) -> Result<(), T>` - Send value, returns `Err(value)` if receiver is closed
- `Sender::is_closed()` - Check if receiver has been dropped or closed
- `Receiver::recv().await` / `receiver.await` - Async receive, returns `Result<T, RecvError>`
- `Receiver::try_recv()` - Non-blocking receive, returns `Result<T, TryRecvError>`
- `Receiver::close()` - Close the receiver, preventing future sends
- `Receiver::blocking_recv()` - Blocking receive for synchronous code

> **Note**: Unlike tokio's oneshot which uses CAS to guarantee `Err` when receiver is already closed, our implementation uses `Arc` refcount check for simplicity. If `send` and `Receiver` drop occur concurrently, `send` may return `Ok(())` even if the value will not be received. Use `Receiver::close()` for explicit cancellation when guaranteed detection is needed.

**Key features**:
- Zero Box allocation for waker storage
- Direct `Future` implementation for ergonomic `.await`
- Fast path for immediate completion
- Supports both sync (`blocking_recv`) and async usage

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

### Generic oneshot channel (like tokio::sync::oneshot)

```rust
use lite_sync::oneshot::generic::{channel, Sender, Receiver, RecvError, TryRecvError};

#[tokio::main]
async fn main() {
    // Create a channel for any Send type
    let (tx, rx) = channel::<String>();
    
    tokio::spawn(async move {
        // send() returns Err(value) if receiver is closed
        if tx.send("Hello".to_string()).is_err() {
            println!("Receiver dropped");
        }
    });
    
    // Direct .await or use recv()
    match rx.await {
        Ok(msg) => println!("Received: {}", msg),
        Err(RecvError) => println!("Sender dropped"),
    }
}
```

### Receiver close and try_recv

```rust
use lite_sync::oneshot::generic::channel;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = channel::<i32>();
    
    // Check if receiver is closed
    assert!(!tx.is_closed());
    
    // Close the receiver - prevents future sends
    rx.close();
    assert!(tx.is_closed());
    
    // send() fails after close
    assert!(tx.send(42).is_err());
}
```

### Lite oneshot with custom state (ultra-lightweight)

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
    fn receiver_closed_value() -> u8 { 254 }
}

#[tokio::main]
async fn main() {
    let (sender, receiver) = Sender::<TaskResult>::new();
    
    tokio::spawn(async move {
        sender.send(TaskResult::Success).unwrap();
    });
    
    match receiver.await {
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
        sender.send(()).unwrap();
    });
    
    match receiver.await {
        Ok(()) => println!("Task completed"),
        Err(_) => println!("Sender dropped"),
    }
}
```

### Blocking receive (for sync code)

```rust
use lite_sync::oneshot::generic::channel;

fn main() {
    let (tx, rx) = channel::<String>();
    
    std::thread::spawn(move || {
        tx.send("Hello from thread".to_string()).unwrap();
    });
    
    // blocking_recv() for synchronous code
    match rx.blocking_recv() {
        Ok(msg) => println!("Received: {}", msg),
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

