use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Duration;
use std::sync::Arc;
use lite_sync::notify::SingleWaiterNotify;
use tokio::sync::Notify;

/// Benchmark: Notify creation comparison (custom vs tokio)
/// 基准测试：Notify 创建对比（自定义 vs tokio）
fn bench_notify_creation_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_creation_comparison");
    
    // Custom SingleWaiterNotify
    group.bench_function("custom_single_waiter_notify", |b| {
        b.iter(|| {
            let _notify = SingleWaiterNotify::new();
        });
    });
    
    // Tokio Notify
    group.bench_function("tokio_notify", |b| {
        b.iter(|| {
            let _notify = Notify::new();
        });
    });
    
    group.finish();
}

/// Benchmark: Notify before wait (fast path)
/// 基准测试：等待前通知（快速路径）
fn bench_notify_before_wait_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_before_wait_comparison");
    
    // Custom SingleWaiterNotify
    group.bench_function("custom_single_waiter_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let notify = SingleWaiterNotify::new();
                
                let start = std::time::Instant::now();
                
                // Notify before waiting (fast path)
                notify.notify_one();
                
                // Wait (should complete immediately)
                notify.notified().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Tokio Notify
    group.bench_function("tokio_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let notify = Notify::new();
                
                let start = std::time::Instant::now();
                
                // Notify before waiting (fast path)
                notify.notify_one();
                
                // Wait (should complete immediately)
                notify.notified().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// Benchmark: Notify after wait registration (slow path)
/// 基准测试：等待注册后通知（慢速路径）
fn bench_notify_after_wait_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_after_wait_comparison");
    
    // Custom SingleWaiterNotify
    group.bench_function("custom_single_waiter_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let notify = Arc::new(SingleWaiterNotify::new());
                let notify_clone = notify.clone();
                
                let start = std::time::Instant::now();
                
                // Spawn notifier task
                tokio::spawn(async move {
                    notify_clone.notify_one();
                });
                
                // Wait for notification
                notify.notified().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Tokio Notify
    group.bench_function("tokio_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let notify = Arc::new(Notify::new());
                let notify_clone = notify.clone();
                
                let start = std::time::Instant::now();
                
                // Spawn notifier task
                tokio::spawn(async move {
                    notify_clone.notify_one();
                });
                
                // Wait for notification
                notify.notified().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// Benchmark: Multiple notification cycles
/// 基准测试：多次通知循环
fn bench_notify_multiple_cycles_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_multiple_cycles_comparison");
    
    for cycles in [10, 100].iter() {
        // Custom SingleWaiterNotify
        group.bench_with_input(
            BenchmarkId::new("custom_single_waiter_notify", cycles),
            cycles,
            |b, &cycles| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        let notify = Arc::new(SingleWaiterNotify::new());
                        
                        let start = std::time::Instant::now();
                        
                        for _ in 0..cycles {
                            let notify_clone = notify.clone();
                            tokio::spawn(async move {
                                notify_clone.notify_one();
                            });
                            
                            notify.notified().await;
                        }
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
        
        // Tokio Notify
        group.bench_with_input(
            BenchmarkId::new("tokio_notify", cycles),
            cycles,
            |b, &cycles| {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                
                b.to_async(&runtime).iter_custom(|iters| async move {
                    let mut total_duration = Duration::from_secs(0);
                    
                    for _ in 0..iters {
                        let notify = Arc::new(Notify::new());
                        
                        let start = std::time::Instant::now();
                        
                        for _ in 0..cycles {
                            let notify_clone = notify.clone();
                            tokio::spawn(async move {
                                notify_clone.notify_one();
                            });
                            
                            notify.notified().await;
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

/// Benchmark: Notify drop performance (pure drop without creation overhead)
/// 基准测试：Notify drop 性能（纯 drop 性能，不包含创建开销）
fn bench_notify_drop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_drop_comparison");
    
    // Custom SingleWaiterNotify - drop without waiting (no waker registered)
    group.bench_function("custom_single_waiter_notify_drop_no_wait", |b| {
        b.iter_batched(
            || {
                // Setup: create notify (not measured)
                SingleWaiterNotify::new()
            },
            |notify| {
                // Only measure drop
                drop(notify);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio Notify - drop without waiting
    group.bench_function("tokio_notify_drop_no_wait", |b| {
        b.iter_batched(
            || {
                // Setup: create notify (not measured)
                Notify::new()
            },
            |notify| {
                // Only measure drop
                drop(notify);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Custom SingleWaiterNotify - drop with registered waker
    group.bench_function("custom_single_waiter_notify_drop_with_waker", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_batched(
            || {
                // Setup: create notify and register waker (not measured)
                let notify = Arc::new(SingleWaiterNotify::new());
                let notify_clone = notify.clone();
                
                // Start waiting and register waker
                let wait_handle = tokio::spawn(async move {
                    let mut notified = Box::pin(notify_clone.notified());
                    // Poll once to register waker
                    tokio::select! {
                        _ = &mut notified => {},
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                });
                
                // Give it time to register (using blocking sleep in setup)
                std::thread::sleep(Duration::from_micros(100));
                
                (notify, wait_handle)
            },
            |(notify, wait_handle)| async move {
                // Only measure drop
                drop(notify);
                
                // Clean up task
                wait_handle.abort();
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio Notify - drop with registered waker
    group.bench_function("tokio_notify_drop_with_waker", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_batched(
            || {
                // Setup: create notify and register waker (not measured)
                let notify = Arc::new(Notify::new());
                let notify_clone = notify.clone();
                
                // Start waiting and register waker
                let wait_handle = tokio::spawn(async move {
                    let mut notified = Box::pin(notify_clone.notified());
                    // Poll once to register waker
                    tokio::select! {
                        _ = &mut notified => {},
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                });
                
                // Give it time to register (using blocking sleep in setup)
                std::thread::sleep(Duration::from_micros(100));
                
                (notify, wait_handle)
            },
            |(notify, wait_handle)| async move {
                // Only measure drop
                drop(notify);
                
                // Clean up task
                wait_handle.abort();
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Custom SingleWaiterNotify - batch drop performance
    group.bench_function("custom_single_waiter_notify_batch_drop", |b| {
        b.iter_batched(
            || {
                // Setup: create 100 notifiers (not measured)
                let mut notifiers = Vec::new();
                for _ in 0..100 {
                    notifiers.push(SingleWaiterNotify::new());
                }
                notifiers
            },
            |notifiers| {
                // Only measure batch drop
                drop(notifiers);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    // Tokio Notify - batch drop performance
    group.bench_function("tokio_notify_batch_drop", |b| {
        b.iter_batched(
            || {
                // Setup: create 100 notifiers (not measured)
                let mut notifiers = Vec::new();
                for _ in 0..100 {
                    notifiers.push(Notify::new());
                }
                notifiers
            },
            |notifiers| {
                // Only measure batch drop
                drop(notifiers);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    
    group.finish();
}

/// Benchmark: Concurrent notify operations
/// 基准测试：并发通知操作
fn bench_notify_concurrent_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("notify_concurrent_comparison");
    
    // Custom SingleWaiterNotify
    group.bench_function("custom_single_waiter_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let notify = Arc::new(SingleWaiterNotify::new());
                
                let start = std::time::Instant::now();
                
                // Multiple notifiers (only one should wake the waiter)
                for _ in 0..5 {
                    let n = notify.clone();
                    tokio::spawn(async move {
                        n.notify_one();
                    });
                }
                
                notify.notified().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    // Tokio Notify
    group.bench_function("tokio_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                let notify = Arc::new(Notify::new());
                
                let start = std::time::Instant::now();
                
                // Multiple notifiers (only one should wake the waiter)
                for _ in 0..5 {
                    let n = notify.clone();
                    tokio::spawn(async move {
                        n.notify_one();
                    });
                }
                
                notify.notified().await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_notify_creation_comparison,
    bench_notify_before_wait_comparison,
    bench_notify_after_wait_comparison,
    bench_notify_multiple_cycles_comparison,
    bench_notify_drop_comparison,
    bench_notify_concurrent_comparison,
);

criterion_main!(benches);

