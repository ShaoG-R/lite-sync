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
lite-sync = "0.1"
```

## 模块

### `oneshot`

带有可自定义状态的一次性完成通知。

非常适合以最小开销发出任务完成信号。通过 `State` trait 支持自定义状态类型，允许您不仅传达"完成"，还能传达"如何完成"（成功、失败、超时等）。

**关键特性**：
- 返回 `Result<T, SendError>`，当发送器被丢弃时返回错误
- 提供 `recv()` 异步方法和 `try_recv()` 非阻塞方法
- Waker 存储零 Box 分配
- 直接实现 `Future` 以支持便捷的 `.await`
- 立即完成的快速路径

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

### 带有自定义状态的一次性完成通知

```rust
use lite_sync::oneshot::{State, Sender};

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
        // 执行一些工作...
        sender.notify(TaskResult::Success);
    });
    
    // 使用 recv() 异步接收，或直接 .await
    match receiver.recv().await {
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
use lite_sync::oneshot::Sender;

#[tokio::main]
async fn main() {
    let (sender, receiver) = Sender::<()>::new();
    
    tokio::spawn(async move {
        // 任务完成
        sender.notify(());
    });
    
    // 使用 recv() 或直接 .await，返回 Result
    match receiver.recv().await {
        Ok(()) => println!("任务完成"),
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

