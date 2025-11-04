/// Lightweight single-waiter notification primitive
/// 
/// Optimized for SPSC (Single Producer Single Consumer) pattern where
/// only one task waits at a time. Much lighter than tokio::sync::Notify.
/// 
/// 轻量级单等待者通知原语
/// 
/// 为 SPSC（单生产者单消费者）模式优化，其中每次只有一个任务等待。
/// 比 tokio::sync::Notify 更轻量。

use std::sync::atomic::{AtomicU8, Ordering};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::atomic_waker::AtomicWaker;

// States for the notification
const EMPTY: u8 = 0;      // No waiter, no notification
const WAITING: u8 = 1;    // Waiter registered
const NOTIFIED: u8 = 2;   // Notification sent

/// Lightweight single-waiter notifier optimized for SPSC pattern
/// 
/// 为 SPSC 模式优化的轻量级单等待者通知器
pub struct SingleWaiterNotify {
    state: AtomicU8,
    waker: AtomicWaker,
}

impl SingleWaiterNotify {
    /// Create a new single-waiter notifier
    /// 
    /// 创建一个新的单等待者通知器
    #[inline]
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            waker: AtomicWaker::new(),
        }
    }
    
    /// Returns a future that completes when notified
    /// 
    /// 返回一个在收到通知时完成的 future
    #[inline]
    pub fn notified(&self) -> Notified<'_> {
        Notified { 
            notify: self,
            registered: false,
        }
    }
    
    /// Wake the waiting task (if any)
    /// 
    /// If called before wait, the next wait will complete immediately.
    /// 
    /// 唤醒等待的任务（如果有）
    /// 
    /// 如果在等待之前调用，下一次等待将立即完成。
    #[inline]
    pub fn notify_one(&self) {
        // Mark as notified
        let prev_state = self.state.swap(NOTIFIED, Ordering::AcqRel);
        
        // If there was a waiter, wake it
        if prev_state == WAITING {
            self.waker.wake();
        }
    }
    
    /// Register a waker to be notified
    /// 
    /// Returns true if already notified (fast path)
    /// 
    /// 注册一个 waker 以接收通知
    /// 
    /// 如果已经被通知则返回 true（快速路径）
    #[inline]
    fn register_waker(&self, waker: &std::task::Waker) -> bool {
        // Try to transition from EMPTY to WAITING
        match self.state.compare_exchange(
            EMPTY,
            WAITING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // Successfully transitioned, store the waker
                self.waker.register(waker);
                false // Not notified yet
            }
            Err(state) => {
                // Already notified or waiting
                if state == NOTIFIED {
                    // Reset to EMPTY for next wait
                    self.state.store(EMPTY, Ordering::Release);
                    true // Already notified
                } else {
                    // State is WAITING, update the waker
                    self.waker.register(waker);
                    
                    // Check if notified while we were updating waker
                    if self.state.load(Ordering::Acquire) == NOTIFIED {
                        self.state.store(EMPTY, Ordering::Release);
                        true
                    } else {
                        false
                    }
                }
            }
        }
    }
}

// Drop is automatically handled by AtomicWaker's drop implementation
// No need for explicit drop implementation
//
// Drop 由 AtomicWaker 的 drop 实现自动处理
// 无需显式的 drop 实现

/// Future returned by `SingleWaiterNotify::notified()`
/// 
/// `SingleWaiterNotify::notified()` 返回的 Future
pub struct Notified<'a> {
    notify: &'a SingleWaiterNotify,
    registered: bool,
}

impl Future for Notified<'_> {
    type Output = ();
    
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // On first poll, register the waker
        if !self.registered {
            self.registered = true;
            if self.notify.register_waker(cx.waker()) {
                // Already notified (fast path)
                return Poll::Ready(());
            }
        } else {
            // On subsequent polls, check if notified
            if self.notify.state.load(Ordering::Acquire) == NOTIFIED {
                self.notify.state.store(EMPTY, Ordering::Release);
                return Poll::Ready(());
            }
            // Update waker in case it changed
            self.notify.register_waker(cx.waker());
        }
        
        Poll::Pending
    }
}

impl Drop for Notified<'_> {
    fn drop(&mut self) {
        if self.registered {
            // If we registered but are being dropped, try to clean up
            // PERFORMANCE: Direct compare_exchange without pre-check:
            // - Single atomic operation instead of two (load + compare_exchange)
            // - Relaxed ordering is sufficient - just cleaning up state
            // - CAS will fail harmlessly if state is not WAITING
            //
            // 如果我们注册了但正在被 drop，尝试清理
            // 性能优化：直接 compare_exchange 无需预检查：
            // - 单次原子操作而不是两次（load + compare_exchange）
            // - Relaxed ordering 就足够了 - 只是清理状态
            // - 如果状态不是 WAITING，CAS 会无害地失败
            let _ = self.notify.state.compare_exchange(
                WAITING,
                EMPTY,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_notify_before_wait() {
        let notify = Arc::new(SingleWaiterNotify::new());
        
        // Notify before waiting
        notify.notify_one();
        
        // Should complete immediately
        notify.notified().await;
    }

    #[tokio::test]
    async fn test_notify_after_wait() {
        let notify = Arc::new(SingleWaiterNotify::new());
        let notify_clone = notify.clone();
        
        // Spawn a task that notifies after a delay
        tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            notify_clone.notify_one();
        });
        
        // Wait for notification
        notify.notified().await;
    }

    #[tokio::test]
    async fn test_multiple_notify_cycles() {
        let notify = Arc::new(SingleWaiterNotify::new());
        
        for _ in 0..10 {
            let notify_clone = notify.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(5)).await;
                notify_clone.notify_one();
            });
            
            notify.notified().await;
        }
    }

    #[tokio::test]
    async fn test_concurrent_notify() {
        let notify = Arc::new(SingleWaiterNotify::new());
        let notify_clone = notify.clone();
        
        // Multiple notifiers (only one should wake the waiter)
        for _ in 0..5 {
            let n = notify_clone.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(10)).await;
                n.notify_one();
            });
        }
        
        notify.notified().await;
    }

    #[tokio::test]
    async fn test_notify_no_waiter() {
        let notify = SingleWaiterNotify::new();
        
        // Notify with no waiter should not panic
        notify.notify_one();
        notify.notify_one();
        
        // Next wait should complete immediately
        notify.notified().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_stress_test() {
        let notify = Arc::new(SingleWaiterNotify::new());
        
        for i in 0..100 {
            let notify_clone = notify.clone();
            tokio::spawn(async move {
                sleep(Duration::from_micros(i % 10)).await;
                notify_clone.notify_one();
            });
            
            notify.notified().await;
        }
    }

    #[tokio::test]
    async fn test_immediate_notification_race() {
        // Test the race between notification and registration
        for _ in 0..100 {
            let notify = Arc::new(SingleWaiterNotify::new());
            let notify_clone = notify.clone();
            
            let waiter = tokio::spawn(async move {
                notify.notified().await;
            });
            
            // Notify immediately (might happen before or after registration)
            notify_clone.notify_one();
            
            // Should complete without timeout
            tokio::time::timeout(Duration::from_millis(100), waiter)
                .await
                .expect("Should not timeout")
                .expect("Task should complete");
        }
    }
}

