use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Duration;
use tokio::sync::oneshot;
use lite_sync::oneshot::lite::channel;

/// Benchmark: Oneshot creation comparison (custom Notify+AtomicU8 vs tokio oneshot)
/// 基准测试：Oneshot 创建对比（自定义 Notify+AtomicU8 vs tokio oneshot）
fn bench_oneshot_creation_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_lite_creation");
    
    // Custom oneshot - OPTIMIZED (1 Arc allocation)
    // 自定义 oneshot - 优化版（1个 Arc 分配）
    group.bench_function("lite_sync_lite", |b| {
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
    let mut group = c.benchmark_group("oneshot_lite_send_recv");
    
    // Custom oneshot - OPTIMIZED (1 Arc allocation)
    // 自定义 oneshot - 优化版（1个 Arc 分配）
    group.bench_function("lite_sync_lite", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (notifier, receiver) = channel();
                
                let start = std::time::Instant::now();
                
                // Send and receive
                notifier.send(());
                let _value = receiver.recv().await;
                
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
    let mut group = c.benchmark_group("oneshot_lite_batch");
    
    for size in [10, 100, 1000].iter() {
        // Custom oneshot - OPTIMIZED
        group.bench_with_input(
            BenchmarkId::new("lite_sync_lite", size),
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
                            ch.0.send(());
                        }
                        
                        // Receive all
                        for ch in channels {
                            let _value = ch.1.recv().await;
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
    let mut group = c.benchmark_group("oneshot_lite_cross_task");
    
    // Custom oneshot - OPTIMIZED
    group.bench_function("lite_sync_lite", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (notifier, receiver) = channel();
                
                let start = std::time::Instant::now();
                
                // Spawn receiver task
                let receiver_handle = tokio::spawn(async move {
                    receiver.recv().await
                });
                
                // Send from main task
                notifier.send(());
                
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
    let mut group = c.benchmark_group("oneshot_lite_immediate");
    
    // Custom oneshot - OPTIMIZED
    group.bench_function("lite_sync_lite", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let (notifier, receiver) = channel();
                
                let start = std::time::Instant::now();
                
                // Send BEFORE receiving (should use fast path)
                notifier.send(());
                
                // Receive (should complete immediately via fast path)
                let _value = receiver.recv().await;
                
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

/// Benchmark: Oneshot drop performance comparison (pure drop without creation overhead)
/// 基准测试：Oneshot drop 性能对比（纯 drop 性能，不包含创建开销）
fn bench_oneshot_drop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("oneshot_lite_drop");
    
    // Custom oneshot - drop without receiving (no waker registered)
    group.bench_function("lite_sync_lite_drop_no_recv", |b| {
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
    group.bench_function("lite_sync_lite_drop_after_notify", |b| {
        b.iter_batched(
            || {
                // Setup: create and notify (not measured)
                let (notifier, receiver) = channel::<()>();
                notifier.send(());
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
    group.bench_function("lite_sync_lite_drop_with_waker", |b| {
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
    group.bench_function("lite_sync_lite_batch_drop", |b| {
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
    bench_oneshot_creation_comparison,
    bench_oneshot_send_recv_comparison,
    bench_oneshot_batch_comparison,
    bench_oneshot_cross_task_comparison,
    bench_oneshot_immediate_notification,
    bench_oneshot_drop_comparison,
);

criterion_main!(benches);
