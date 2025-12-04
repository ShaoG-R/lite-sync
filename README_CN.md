# lite-sync

[![Crates.io](https://img.shields.io/crates/v/lite-sync.svg)](https://crates.io/crates/lite-sync)
[![Documentation](https://docs.rs/lite-sync/badge.svg)](https://docs.rs/lite-sync)
[![License](https://img.shields.io/crates/l/lite-sync.svg)](https://github.com/ShaoG-R/lite-sync#license)

快速、轻量级的异步原语：SPSC 通道、oneshot、通知器和原子唤醒器。

[📖 English](README.md) | [📖 中文文档](README_CN.md)

## 概述

`lite-sync` 提供了一系列优化的同步原语，专为低延迟和最小分配而设计。这些原语从头开始构建，以性能为核心，为更重的标准库实现提供替代方案。

## 特性

- **零或最小分配**：大多数原语完全避免堆分配
- **无锁算法**：使用原子操作实现最大并发性
- **单等待者优化**：专为常见的 SPSC（单生产者单消费者）模式优化
- **内联存储**：支持栈分配缓冲区以避免堆分配
- **类型安全**：利用 Rust 的类型系统在编译时强制正确性

## 安装

将以下内容添加到您的 `Cargo.toml`：

```toml
[dependencies]
lite-sync = "0.2"
```

## 模块

### `oneshot`

用于在任务之间发送单个值的一次性通道，**API 行为与 tokio::sync::oneshot 对齐**。

提供两种变体：
- **`oneshot::generic`** - 适用于任意类型 `T: Send`，使用 `UnsafeCell<MaybeUninit<T>>` 存储
- **`oneshot::lite`** - 超轻量变体，适用于 `State` 可编码类型，仅使用 `AtomicU8` 存储

**API（与 tokio oneshot 对齐）**：
- `channel<T>()` - 创建发送器/接收器对
- `Sender::send(value) -> Result<(), T>` - 发送值，如果接收器已关闭则返回 `Err(value)`
- `Sender::is_closed()` - 检查接收器是否已被丢弃或关闭
- `Receiver::recv().await` / `receiver.await` - 异步接收，返回 `Result<T, RecvError>`
- `Receiver::try_recv()` - 非阻塞接收，返回 `Result<T, TryRecvError>`
- `Receiver::close()` - 关闭接收器，阻止后续发送
- `Receiver::blocking_recv()` - 阻塞接收，用于同步代码

> **注意**：与 tokio 的 oneshot 使用 CAS 保证接收器已关闭时返回 `Err` 不同，我们的实现为了简单使用 `Arc` 引用计数检查。如果 `send` 和 `Receiver` 的 drop 同时发生，`send` 可能返回 `Ok(())` 即使值不会被接收。如需保证检测到取消，请使用 `Receiver::close()` 显式取消。

**关键特性**：
- Waker 存储零 Box 分配
- 直接实现 `Future` 以支持便捷的 `.await`
- 立即完成的快速路径
- 支持同步（`blocking_recv`）和异步使用

### `spsc`

高性能异步 SPSC（单生产者单消费者）通道。

基于 `smallring` 构建，支持内联存储的高效环形缓冲区操作。类型安全地强制单生产者/消费者语义，消除同步开销。

**关键优化**：
- 使用 `UnsafeCell` 实现零成本内部可变性
- 小容量通道的内联缓冲区支持
- 批量发送/接收操作
- 单等待者通知

### `notify`

轻量级单等待者通知原语。

当您每次只需唤醒一个任务时，比 `tokio::sync::Notify` 更轻量。非常适合在其他原语中进行内部同步。

### `atomic_waker`

带有状态机同步的原子 waker 存储。

基于 Tokio 的 `AtomicWaker` 但为特定用例简化。提供对 waker 的安全并发访问，无需 Box 分配。

## 示例

### 通用 oneshot 通道（类似 tokio::sync::oneshot）

```rust
use lite_sync::oneshot::generic::{channel, Sender, Receiver, RecvError, TryRecvError};

#[tokio::main]
async fn main() {
    // 为任意 Send 类型创建通道
    let (tx, rx) = channel::<String>();
    
    tokio::spawn(async move {
        // send() 在接收器关闭时返回 Err(value)
        if tx.send("Hello".to_string()).is_err() {
            println!("接收器已丢弃");
        }
    });
    
    // 直接 .await 或使用 recv()
    match rx.await {
        Ok(msg) => println!("收到: {}", msg),
        Err(RecvError) => println!("发送器已丢弃"),
    }
}
```

### 接收器关闭和 try_recv

```rust
use lite_sync::oneshot::generic::channel;

#[tokio::main]
async fn main() {
    let (tx, mut rx) = channel::<i32>();
    
    // 检查接收器是否已关闭
    assert!(!tx.is_closed());
    
    // 关闭接收器 - 阻止后续发送
    rx.close();
    assert!(tx.is_closed());
    
    // close 后 send() 失败
    assert!(tx.send(42).is_err());
}
```

### Lite oneshot 自定义状态（超轻量）

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
        Ok(TaskResult::Success) => println!("任务成功"),
        Ok(TaskResult::Error) => println!("任务失败"),
        Err(_) => println!("发送器已丢弃"),
    }
}
```

### 带有内联存储的 SPSC 通道

```rust
use lite_sync::spsc::channel;
use std::num::NonZeroUsize;

#[tokio::main]
async fn main() {
    // 创建容量为 32、内联缓冲区大小为 8 的通道
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

### 单等待者通知

```rust
use lite_sync::notify::SingleWaiterNotify;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let notify = Arc::new(SingleWaiterNotify::new());
    let notify_clone = notify.clone();
    
    tokio::spawn(async move {
        // 执行一些工作...
        notify_clone.notify_one();
    });
    
    notify.notified().await;
}
```

### 简单的完成通知（单元类型）

```rust
use lite_sync::oneshot::lite::Sender;

#[tokio::main]
async fn main() {
    let (sender, receiver) = Sender::<()>::new();
    
    tokio::spawn(async move {
        sender.send(()).unwrap();
    });
    
    match receiver.await {
        Ok(()) => println!("任务完成"),
        Err(_) => println!("发送器已丢弃"),
    }
}
```

### 阻塞接收（用于同步代码）

```rust
use lite_sync::oneshot::generic::channel;

fn main() {
    let (tx, rx) = channel::<String>();
    
    std::thread::spawn(move || {
        tx.send("来自线程的问候".to_string()).unwrap();
    });
    
    // blocking_recv() 用于同步代码
    match rx.blocking_recv() {
        Ok(msg) => println!("收到: {}", msg),
        Err(_) => println!("发送器已丢弃"),
    }
}
```

## 基准测试

性能基准测试位于 `benches/` 目录中。运行方式：

```bash
cargo bench
```

主要特性：
- **Oneshot**：立即完成极快，优化的异步等待路径
- **SPSC**：每条消息低延迟开销，高效的批量操作
- **Notify**：最小的通知往返时间

## 安全性

所有原语在内部使用 `unsafe` 以提高性能，但暴露安全的 API。安全性通过以下方式保证：

- **类型系统强制**单一所有权（SPSC 端点不实现 `Clone`）
- 用于同步的**原子状态机**
- 原子操作的**仔细排序**
- **全面的测试覆盖**，包括并发场景

## 最低支持的 Rust 版本 (MSRV)

Rust 2024 版本（Rust 1.85.0 或更高版本）

## 贡献

欢迎贡献！请随时提交 Pull Request。

## 许可证

根据以下任一许可证授权：

- Apache 许可证，版本 2.0 ([LICENSE-APACHE](LICENSE-APACHE) 或 http://www.apache.org/licenses/LICENSE-2.0)
- MIT 许可证 ([LICENSE-MIT](LICENSE-MIT) 或 http://opensource.org/licenses/MIT)

由您选择。

### 贡献协议

除非您明确声明，否则您有意提交给本作品的任何贡献（如 Apache-2.0 许可证中所定义），均应按上述方式双重许可，不附加任何额外条款或条件。

