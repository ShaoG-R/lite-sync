use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::num::NonZeroUsize;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::sync::mpsc;
use lite_sync::oneshot::channel;
use lite_sync::spsc;

/// Benchmark: Oneshot creation comparison (custom Notify+AtomicU8 vs tokio oneshot)
/// 基准测试：Oneshot 创建对比（自定义 Notify+AtomicU8 vs tokio oneshot）
fn bench_oneshot_creation_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_creation_comparison");
    
    // Custom oneshot - OPTIMIZED (1 Arc allocation)
    // 自定义 oneshot - 优化版（1个 Arc 分配）
    group.bench_function("custom_1arc_optimized", |b| {
        b.iter(|| {
            let (_notifier, _receiver) = channel::<()>();
        });
    });
    
    // Tokio oneshot
    group.bench_function("tokio_oneshot", |b| {
        b.iter(|| {
            let (_tx, _rx) = oneshot::channel::<u32>();
        });
    });
    
    group.finish();
}

/// Benchmark: Oneshot send/recv comparison (custom Notify+AtomicU8 vs tokio oneshot)
/// 基准测试：Oneshot 发送接收对比（自定义 Notify+AtomicU8 vs tokio oneshot）
fn bench_oneshot_send_recv_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_send_recv_comparison");
    
    // Custom oneshot - OPTIMIZED (1 Arc allocation)
    // 自定义 oneshot - 优化版（1个 Arc 分配）
    group.bench_function("custom_1arc_optimized", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (notifier, receiver) = channel();
                
                let start = std::time::Instant::now();
                
                // Send and receive
                notifier.notify(());
                let _value = receiver.wait().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Tokio oneshot
    group.bench_function("tokio_oneshot", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, rx) = oneshot::channel::<u32>();
                
                let start = std::time::Instant::now();
                
                tx.send(42).unwrap();
                let _ = rx.await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// Benchmark: Oneshot batch operations (multiple oneshot channels)
/// 基准测试：Oneshot 批量操作（多个 oneshot 通道）
fn bench_oneshot_batch_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_batch_comparison");
    
    for size in [10, 100, 1000].iter() {
        // Custom oneshot - OPTIMIZED
        group.bench_with_input(
            BenchmarkId::new("custom_1arc_optimized", size),
            size,
            |b, &size| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        // Create channels
                        let mut channels = Vec::new();
                        for _ in 0..size {
                            channels.push(channel());
                        }
                        
                        let start = std::time::Instant::now();
                        
                        // Send all
                        for ch in &channels {
                            ch.0.notify(());
                        }
                        
                        // Receive all
                        for ch in channels {
                            let _value = ch.1.wait().await;
                        }
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
        
        // Tokio oneshot
        group.bench_with_input(
            BenchmarkId::new("tokio_oneshot", size),
            size,
            |b, &size| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        // Create channels
                        let mut channels = Vec::new();
                        for _ in 0..size {
                            channels.push(oneshot::channel::<u32>());
                        }
                        
                        let start = std::time::Instant::now();
                        
                        // Send all
                        let mut receivers = Vec::new();
                        for (tx, rx) in channels {
                            tx.send(42).unwrap();
                            receivers.push(rx);
                        }
                        
                        // Receive all
                        for rx in receivers {
                            let _ = rx.await.unwrap();
                        }
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark: Oneshot cross-task communication
/// 基准测试：Oneshot 跨任务通信
fn bench_oneshot_cross_task_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_cross_task_comparison");
    
    // Custom oneshot - OPTIMIZED
    group.bench_function("custom_1arc_optimized", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (notifier, receiver) = channel();
                
                let start = std::time::Instant::now();
                
                // Spawn receiver task
                let receiver_handle = tokio::spawn(async move {
                    receiver.wait().await
                });
                
                // Send from main task
                notifier.notify(());
                
                // Wait for receiver
                let _value = receiver_handle.await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Tokio oneshot
    group.bench_function("tokio_oneshot", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, rx) = oneshot::channel::<u32>();
                
                let start = std::time::Instant::now();
                
                // Spawn receiver task
                let receiver_handle = tokio::spawn(async move {
                    rx.await.unwrap()
                });
                
                // Send from main task
                tx.send(42).unwrap();
                
                // Wait for receiver
                let _value = receiver_handle.await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// Benchmark: Oneshot with immediate notification (already notified before await)
/// 基准测试：立即通知的 Oneshot（await 之前已经通知）
fn bench_oneshot_immediate_notification(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_immediate_notification");
    
    // Custom oneshot - OPTIMIZED
    group.bench_function("custom_1arc_optimized", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (notifier, receiver) = channel();
                
                let start = std::time::Instant::now();
                
                // Send BEFORE receiving (should use fast path)
                notifier.notify(());
                
                // Receive (should complete immediately via fast path)
                let _value = receiver.wait().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Tokio oneshot
    group.bench_function("tokio_oneshot", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, rx) = oneshot::channel::<u32>();
                
                let start = std::time::Instant::now();
                
                // Send immediately
                tx.send(42).unwrap();
                
                // Receive (should complete immediately)
                let _ = rx.await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}


/// Benchmark: Bounded channel creation comparison (tokio mpsc vs custom rtrb spsc)
/// 基准测试：有界通道创建对比（tokio mpsc vs 自定义 rtrb spsc）
fn bench_bounded_creation_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_creation_comparison");
    
    const CAPACITY: usize = 100;
    
    // Tokio mpsc bounded
    group.bench_function("tokio_mpsc_bounded", |b| {
        b.iter(|| {
            let (_tx, _rx) = mpsc::channel::<u32>(CAPACITY);
        });
    });
    
    // Custom rtrb-based SPSC
    group.bench_function("custom_rtrb_spsc", |b| {
        b.iter(|| {
            let (_tx, _rx) = spsc::channel::<u32, CAPACITY>(NonZeroUsize::new(CAPACITY).unwrap());
        });
    });
    
    group.finish();
}

/// Benchmark: Single producer single consumer send/recv (tokio mpsc vs custom rtrb spsc)
/// 基准测试：单生产者单消费者发送接收（tokio mpsc vs 自定义 rtrb spsc）
fn bench_bounded_spsc_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_spsc_comparison");
    
    const CAPACITY: usize = 100;
    
    // Tokio mpsc bounded
    group.bench_function("tokio_mpsc_bounded", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, mut rx) = mpsc::channel::<u32>(CAPACITY);
                
                let start = std::time::Instant::now();
                
                tx.send(42).await.unwrap();
                let _ = rx.recv().await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Custom rtrb-based SPSC
    group.bench_function("custom_rtrb_spsc", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, rx) = spsc::channel::<u32, CAPACITY>(NonZeroUsize::new(CAPACITY).unwrap());
                
                let start = std::time::Instant::now();
                
                tx.send(42).await.unwrap();
                let _ = rx.recv().await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// Benchmark: Batch operations (tokio mpsc vs custom rtrb spsc)
/// 基准测试：批量操作（tokio mpsc vs 自定义 rtrb spsc）
fn bench_bounded_batch_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_batch_comparison");
    
    for size in [10, 100, 1000].iter() {
        let capacity = size + 10; // 容量稍大于消息数量
        
        // Tokio mpsc bounded
        group.bench_with_input(
            BenchmarkId::new("tokio_mpsc_bounded", size),
            size,
            |b, &size| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        let (tx, mut rx) = mpsc::channel::<u32>(capacity);
                        
                        let start = std::time::Instant::now();
                        
                        // Send all
                        for i in 0..size {
                            tx.send(i as u32).await.unwrap();
                        }
                        
                        // Receive all
                        for _ in 0..size {
                            let _ = rx.recv().await.unwrap();
                        }
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
        
        // Custom rtrb-based SPSC
        group.bench_with_input(
            BenchmarkId::new("custom_rtrb_spsc", size),
            size,
            |b, &size| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        let (tx, rx) = spsc::channel::<u32, 32>(NonZeroUsize::new(capacity).unwrap());
                        
                        let start = std::time::Instant::now();
                        
                        // Send all
                        for i in 0..size {
                            tx.send(i as u32).await.unwrap();
                        }
                        
                        // Receive all
                        for _ in 0..size {
                            let _ = rx.recv().await.unwrap();
                        }
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark: Cross-task communication (tokio mpsc vs custom rtrb spsc)
/// 基准测试：跨任务通信（tokio mpsc vs 自定义 rtrb spsc）
fn bench_bounded_cross_task_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_cross_task_comparison");
    
    const CAPACITY: usize = 100;
    
    // Tokio mpsc bounded
    group.bench_function("tokio_mpsc_bounded", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, mut rx) = mpsc::channel::<u32>(CAPACITY);
                
                let start = std::time::Instant::now();
                
                // Spawn receiver task
                let receiver_handle = tokio::spawn(async move {
                    rx.recv().await.unwrap()
                });
                
                // Send from main task
                tx.send(42).await.unwrap();
                
                // Wait for receiver
                let _ = receiver_handle.await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Custom rtrb-based SPSC
    group.bench_function("custom_rtrb_spsc", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (tx, rx) = spsc::channel::<u32, CAPACITY>(NonZeroUsize::new(CAPACITY).unwrap());
                
                let start = std::time::Instant::now();
                
                // Spawn receiver task
                let receiver_handle = tokio::spawn(async move {
                    rx.recv().await.unwrap()
                });
                
                // Send from main task
                tx.send(42).await.unwrap();
                
                // Wait for receiver
                let _ = receiver_handle.await.unwrap();
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// Benchmark: High throughput (tokio mpsc vs custom rtrb spsc)
/// 基准测试：高吞吐量（tokio mpsc vs 自定义 rtrb spsc）
fn bench_bounded_throughput_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("bounded_throughput_comparison");
    
    for size in [1000, 10000].iter() {
        let capacity = 1024; // 固定的缓冲区大小，测试背压场景
        
        // Tokio mpsc bounded
        group.bench_with_input(
            BenchmarkId::new("tokio_mpsc_bounded", size),
            size,
            |b, &size| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        let (tx, mut rx) = mpsc::channel::<u32>(capacity);
                        
                        let start = std::time::Instant::now();
                        
                        // Spawn sender task
                        let sender_handle = tokio::spawn(async move {
                            for i in 0..size {
                                tx.send(i).await.unwrap();
                            }
                        });
                        
                        // Receive all in main task
                        for _ in 0..size {
                            let _ = rx.recv().await.unwrap();
                        }
                        
                        sender_handle.await.unwrap();
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
        
        // Custom rtrb-based SPSC
        group.bench_with_input(
            BenchmarkId::new("custom_rtrb_spsc", size),
            size,
            |b, &size| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        let (tx, rx) = spsc::channel::<u32, 32>(NonZeroUsize::new(capacity).unwrap());
                        
                        let start = std::time::Instant::now();
                        
                        // Spawn sender task
                        let sender_handle = tokio::spawn(async move {
                            for i in 0..size {
                                tx.send(i).await.unwrap();
                            }
                        });
                        
                        // Receive all in main task
                        for _ in 0..size {
                            let _ = rx.recv().await.unwrap();
                        }
                        
                        sender_handle.await.unwrap();
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark: Oneshot drop performance comparison (pure drop without creation overhead)
/// 基准测试：Oneshot drop 性能对比（纯 drop 性能，不包含创建开销）
fn bench_oneshot_drop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_drop_comparison");
    
    // Custom oneshot - drop without receiving (no waker registered)
    group.bench_function("custom_1arc_optimized_drop_no_recv", |b| {
        b.iter_batched(
            || {
                // Setup: create receiver (not measured)
                let (_notifier, receiver) = channel::<()>();
                receiver
            },
            |receiver| {
                // Only measure drop
                drop(receiver);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio oneshot - drop without receiving
    group.bench_function("tokio_oneshot_drop_no_recv", |b| {
        b.iter_batched(
            || {
                // Setup: create receiver (not measured)
                let (_tx, rx) = oneshot::channel::<u32>();
                rx
            },
            |rx| {
                // Only measure drop
                drop(rx);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Custom oneshot - drop after notification (waker already consumed, common case)
    group.bench_function("custom_1arc_optimized_drop_after_notify", |b| {
        b.iter_batched(
            || {
                // Setup: create and notify (not measured)
                let (notifier, receiver) = channel::<()>();
                notifier.notify(());
                receiver
            },
            |receiver| {
                // Only measure drop
                drop(receiver);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio oneshot - drop after send (value already stored)
    group.bench_function("tokio_oneshot_drop_after_send", |b| {
        b.iter_batched(
            || {
                // Setup: create and send (not measured)
                let (tx, rx) = oneshot::channel::<u32>();
                let _ = tx.send(42);
                rx
            },
            |rx| {
                // Only measure drop
                drop(rx);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Custom oneshot - drop with registered waker (uncommon slow path)
    group.bench_function("custom_1arc_optimized_drop_with_waker", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_batched(
            || {
                // Setup: create and register waker (not measured)
                let (_notifier, mut receiver) = channel::<()>();
                
                // Poll once to register waker
                let mut receiver_pin = std::pin::Pin::new(&mut receiver);
                let waker = futures::task::noop_waker();
                let mut cx = std::task::Context::from_waker(&waker);
                let _ = std::future::Future::poll(receiver_pin.as_mut(), &mut cx);
                
                receiver
            },
            |receiver| async move {
                // Only measure drop
                drop(receiver);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio oneshot - drop with registered waker (uncommon slow path)
    group.bench_function("tokio_oneshot_drop_with_waker", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_batched(
            || {
                // Setup: create and register waker (not measured)
                let (_tx, mut rx) = oneshot::channel::<u32>();
                
                // Poll once to register waker
                let mut rx_pin = std::pin::Pin::new(&mut rx);
                let waker = futures::task::noop_waker();
                let mut cx = std::task::Context::from_waker(&waker);
                let _ = std::future::Future::poll(rx_pin.as_mut(), &mut cx);
                
                rx
            },
            |rx| async move {
                // Only measure drop
                drop(rx);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Custom oneshot - batch drop performance (100 receivers)
    group.bench_function("custom_1arc_optimized_batch_drop", |b| {
        b.iter_batched(
            || {
                // Setup: create 100 receivers (not measured)
                let mut receivers = Vec::new();
                for _ in 0..100 {
                    let (_notifier, receiver) = channel::<()>();
                    receivers.push(receiver);
                }
                receivers
            },
            |receivers| {
                // Only measure batch drop
                drop(receivers);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio oneshot - batch drop performance (100 receivers)
    group.bench_function("tokio_oneshot_batch_drop", |b| {
        b.iter_batched(
            || {
                // Setup: create 100 receivers (not measured)
                let mut receivers = Vec::new();
                for _ in 0..100 {
                    let (_tx, rx) = oneshot::channel::<u32>();
                    receivers.push(rx);
                }
                receivers
            },
            |receivers| {
                // Only measure batch drop
                drop(receivers);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    group.finish();
}

criterion_group!(
    benches,
    // Oneshot 优化版对比测试
    bench_oneshot_creation_comparison,
    bench_oneshot_send_recv_comparison,
    bench_oneshot_batch_comparison,
    bench_oneshot_cross_task_comparison,
    bench_oneshot_immediate_notification,
    bench_oneshot_drop_comparison,
    // Bounded channel 对比测试 (tokio mpsc vs custom rtrb spsc)
    bench_bounded_creation_comparison,
    bench_bounded_spsc_comparison,
    bench_bounded_batch_comparison,
    bench_bounded_cross_task_comparison,
    bench_bounded_throughput_comparison,
);

criterion_main!(benches);
