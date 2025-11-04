/// Atomic waker storage using state machine for safe concurrent access
/// 
/// Based on Tokio's AtomicWaker but simplified for our use cases.
/// Uses UnsafeCell<Option<Waker>> + atomic state machine to avoid Box allocation
/// while maintaining safe concurrent access.
/// 
/// 使用状态机进行安全并发访问的原子 waker 存储
/// 
/// 基于 Tokio 的 AtomicWaker 但为我们的用例简化。
/// 使用 UnsafeCell<Option<Waker>> + 原子状态机避免 Box 分配
/// 同时保持安全的并发访问。

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Waker;

// Waker registration states
const WAITING: usize = 0;
const REGISTERING: usize = 0b01;
const WAKING: usize = 0b10;

/// Atomic waker storage with state machine synchronization
/// 
/// 带有状态机同步的原子 waker 存储
pub(crate) struct AtomicWaker {
    state: AtomicUsize,
    waker: UnsafeCell<Option<Waker>>,
}

// SAFETY: AtomicWaker is Sync because access to waker is synchronized via atomic state machine
unsafe impl Sync for AtomicWaker {}
unsafe impl Send for AtomicWaker {}

impl AtomicWaker {
    /// Create a new atomic waker
    /// 
    /// 创建一个新的原子 waker
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicUsize::new(WAITING),
            waker: UnsafeCell::new(None),
        }
    }

    /// Register a waker to be notified
    /// 
    /// This will store the waker and handle concurrent access safely.
    /// If a concurrent wake happens during registration, the newly
    /// registered waker will be woken immediately.
    /// 
    /// 注册一个要通知的 waker
    /// 
    /// 这将存储 waker 并安全地处理并发访问。
    /// 如果在注册期间发生并发唤醒，新注册的 waker 将立即被唤醒。
    #[inline]
    pub(crate) fn register(&self, waker: &Waker) {
        match self.state.compare_exchange(
            WAITING,
            REGISTERING,
            Ordering::Acquire,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // SAFETY: We have exclusive access via REGISTERING lock
                unsafe {
                    // Replace the waker
                    let old_waker = (*self.waker.get()).replace(waker.clone());
                    
                    // Try to release the lock
                    match self.state.compare_exchange(
                        REGISTERING,
                        WAITING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            // Successfully released, just drop old waker
                            drop(old_waker);
                        }
                        Err(_) => {
                            // Concurrent wake happened, take waker and wake it
                            // State must be REGISTERING | WAKING
                            let waker = (*self.waker.get()).take();
                            self.state.store(WAITING, Ordering::Release);
                            
                            drop(old_waker);
                            if let Some(waker) = waker {
                                waker.wake();
                            }
                        }
                    }
                }
            }
            Err(WAKING) => {
                // Currently waking, just wake the new waker directly
                waker.wake_by_ref();
            }
            Err(_) => {
                // Concurrent register (shouldn't happen in normal usage)
                // Just drop this registration
            }
        }
    }

    /// Take the waker out for waking
    /// 
    /// Returns the waker if one was registered, None otherwise.
    /// This atomically removes the waker from storage.
    /// 
    /// 取出 waker 用于唤醒
    /// 
    /// 如果注册了 waker 则返回它，否则返回 None。
    /// 这会原子地从存储中移除 waker。
    #[inline]
    pub(crate) fn take(&self) -> Option<Waker> {
        match self.state.fetch_or(WAKING, Ordering::AcqRel) {
            WAITING => {
                // SAFETY: We have exclusive access via WAKING lock
                let waker = unsafe { (*self.waker.get()).take() };
                
                // Release the lock
                self.state.store(WAITING, Ordering::Release);
                
                waker
            }
            _ => {
                // Concurrent register or wake in progress
                None
            }
        }
    }

    /// Wake the registered waker if any
    /// 
    /// 唤醒已注册的 waker（如果有）
    #[inline]
    pub(crate) fn wake(&self) {
        if let Some(waker) = self.take() {
            waker.wake();
        }
    }
}

impl Drop for AtomicWaker {
    fn drop(&mut self) {
        // SAFETY: We have exclusive access during drop
        unsafe {
            let _ = (*self.waker.get()).take();
        }
    }
}

impl std::fmt::Debug for AtomicWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtomicWaker").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_basic_register_and_take() {
        let atomic_waker = AtomicWaker::new();
        let waker = futures::task::noop_waker();
        
        atomic_waker.register(&waker);
        let taken = atomic_waker.take();
        assert!(taken.is_some());
        
        // Second take should return None
        let taken2 = atomic_waker.take();
        assert!(taken2.is_none());
    }

    #[test]
    fn test_concurrent_access() {
        use std::thread;
        
        let atomic_waker = Arc::new(AtomicWaker::new());
        let waker = futures::task::noop_waker();
        
        let aw1 = atomic_waker.clone();
        let w1 = waker.clone();
        let h1 = thread::spawn(move || {
            for _ in 0..100 {
                aw1.register(&w1);
            }
        });
        
        let aw2 = atomic_waker.clone();
        let h2 = thread::spawn(move || {
            for _ in 0..100 {
                aw2.take();
            }
        });
        
        h1.join().unwrap();
        h2.join().unwrap();
    }
}

