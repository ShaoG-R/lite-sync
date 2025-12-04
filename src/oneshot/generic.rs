//! Generic oneshot channel for arbitrary types.
//!
//! 用于任意类型的通用一次性通道。

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU8, Ordering};

use super::common::{self, OneshotStorage, TakeResult};

// Re-export common types
pub use super::common::error;
pub use super::common::RecvError;

// States for the value cell
const EMPTY: u8 = 0;    // No value stored
const READY: u8 = 1;    // Value is ready
const CLOSED: u8 = 2;   // Sender dropped without sending

// ============================================================================
// Generic Storage
// ============================================================================

/// Storage for generic types using `UnsafeCell<MaybeUninit<T>>`
/// 
/// 使用 `UnsafeCell<MaybeUninit<T>>` 存储泛型类型
pub struct GenericStorage<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: GenericStorage<T> is Send + Sync as long as T is Send
// - UnsafeCell<MaybeUninit<T>> is protected by atomic state transitions
// - Only one thread can access the value at a time (enforced by state machine)
unsafe impl<T: Send> Send for GenericStorage<T> {}
unsafe impl<T: Send> Sync for GenericStorage<T> {}

impl<T: Send> OneshotStorage for GenericStorage<T> {
    type Value = T;
    
    #[inline]
    fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
    
    #[inline]
    fn store(&self, value: T) {
        // SAFETY: Only called once by sender (enforced by ownership)
        unsafe {
            (*self.value.get()).write(value);
        }
        self.state.store(READY, Ordering::Release);
    }
    
    #[inline]
    fn try_take(&self) -> TakeResult<T> {
        let state = self.state.swap(EMPTY, Ordering::Acquire);
        match state {
            READY => {
                // SAFETY: State was READY, value is initialized
                unsafe { TakeResult::Ready((*self.value.get()).assume_init_read()) }
            }
            CLOSED => TakeResult::Closed,
            _ => TakeResult::Pending,
        }
    }
    
    #[inline]
    fn is_sender_dropped(&self) -> bool {
        self.state.load(Ordering::Acquire) == CLOSED
    }
    
    #[inline]
    fn mark_sender_dropped(&self) {
        self.state.store(CLOSED, Ordering::Release);
    }
}

impl<T> Drop for GenericStorage<T> {
    fn drop(&mut self) {
        // Clean up the value if it was sent but not received
        if *self.state.get_mut() == READY {
            unsafe {
                (*self.value.get()).assume_init_drop();
            }
        }
    }
}

// ============================================================================
// Type Aliases
// ============================================================================

/// Sender for one-shot value transfer of generic types
/// 
/// 用于泛型类型一次性值传递的发送器
pub type Sender<T> = common::Sender<GenericStorage<T>>;

/// Receiver for one-shot value transfer of generic types
/// 
/// 用于泛型类型一次性值传递的接收器
pub type Receiver<T> = common::Receiver<GenericStorage<T>>;

/// Create a new oneshot channel for generic types
/// 
/// 创建一个用于泛型类型的新 oneshot 通道
#[inline]
pub fn channel<T: Send>() -> (Sender<T>, Receiver<T>) {
    Sender::new()
}

// ============================================================================
// Receiver Extension Methods
// ============================================================================

impl<T: Send> Receiver<T> {
    /// Try to receive a value without blocking
    /// 
    /// Returns `None` if no value has been sent yet
    /// 
    /// 尝试接收值而不阻塞
    /// 
    /// 如果还没有发送值则返回 `None`
    #[inline]
    pub fn try_recv(&mut self) -> Option<T> {
        self.inner.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_oneshot_string() {
        let (sender, receiver) = Sender::<String>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send("Hello".to_string()).unwrap();
        });
        
        let result = receiver.wait().await.unwrap();
        assert_eq!(result, "Hello");
    }
    
    #[tokio::test]
    async fn test_oneshot_integer() {
        let (sender, receiver) = Sender::<i32>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send(42).unwrap();
        });
        
        let result = receiver.wait().await.unwrap();
        assert_eq!(result, 42);
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate() {
        let (sender, receiver) = Sender::<String>::new();
        
        // Send before waiting (fast path)
        sender.send("Immediate".to_string()).unwrap();
        
        let result = receiver.wait().await.unwrap();
        assert_eq!(result, "Immediate");
    }
    
    #[tokio::test]
    async fn test_oneshot_custom_struct() {
        #[derive(Debug, Clone, PartialEq)]
        struct CustomData {
            id: u64,
            name: String,
        }
        
        let (sender, receiver) = Sender::<CustomData>::new();
        
        let data = CustomData {
            id: 123,
            name: "Test".to_string(),
        };
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send(data).unwrap();
        });
        
        let result = receiver.wait().await.unwrap();
        assert_eq!(result.id, 123);
        assert_eq!(result.name, "Test");
    }
    
    #[tokio::test]
    async fn test_oneshot_direct_await() {
        let (sender, receiver) = Sender::<i32>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send(99).unwrap();
        });
        
        // Direct await without .wait()
        let result = receiver.await.unwrap();
        assert_eq!(result, 99);
    }
    
    #[tokio::test]
    async fn test_oneshot_await_mut_reference() {
        let (sender, mut receiver) = Sender::<String>::new();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender.send("Mutable".to_string()).unwrap();
        });
        
        // Await on mutable reference
        let result = (&mut receiver).await.unwrap();
        assert_eq!(result, "Mutable");
    }
    
    #[tokio::test]
    async fn test_oneshot_immediate_await() {
        let (sender, receiver) = Sender::<Vec<u8>>::new();
        
        // Immediate send (fast path)
        sender.send(vec![1, 2, 3]).unwrap();
        
        // Direct await
        let result = receiver.await.unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }
    
    #[tokio::test]
    async fn test_oneshot_try_recv() {
        let (sender, mut receiver) = Sender::<i32>::new();
        
        // Try receive before sending
        assert_eq!(receiver.try_recv(), None);
        
        // Send value
        sender.send(42).unwrap();
        
        // Try receive after sending
        assert_eq!(receiver.try_recv(), Some(42));
    }
    
    #[tokio::test]
    async fn test_oneshot_dropped() {
        let (sender, receiver) = Sender::<i32>::new();
        drop(sender);
        assert_eq!(receiver.await, Err(RecvError));
    }

    #[tokio::test]
    async fn test_oneshot_large_data() {
        let (sender, receiver) = Sender::<Vec<u8>>::new();
        
        let large_vec = vec![0u8; 1024 * 1024]; // 1MB
        
        tokio::spawn(async move {
            sender.send(large_vec).unwrap();
        });
        
        let result = receiver.await.unwrap();
        assert_eq!(result.len(), 1024 * 1024);
    }
}
