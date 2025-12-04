use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::num::NonZeroUsize;
use std::time::Duration;
use tokio::sync::mpsc;
use lite_sync::spsc;

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

criterion_group!(
    benches,
    bench_bounded_creation_comparison,
    bench_bounded_spsc_comparison,
    bench_bounded_batch_comparison,
    bench_bounded_cross_task_comparison,
    bench_bounded_throughput_comparison,
);

criterion_main!(benches);
