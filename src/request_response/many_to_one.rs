/// Many-to-one bidirectional request-response channel
/// 
/// Optimized for multiple request senders (side A) communicating with a single
/// response handler (side B). Uses lock-free queue for concurrent request submission.
/// 
/// 多对一双向请求-响应通道
/// 
/// 为多个请求发送方（A方）与单个响应处理方（B方）通信而优化。
/// 使用无锁队列实现并发请求提交。
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use crossbeam_queue::SegQueue;

use crate::oneshot::generic::Sender as OneshotSender;
use super::common::ChannelError;

/// Internal request wrapper containing request data and response channel
/// 
/// 内部请求包装器，包含请求数据和响应通道
struct RequestWrapper<Req, Resp: Send> {
    /// The actual request data
    /// 
    /// 实际的请求数据
    request: Req,
    
    /// Oneshot sender to return the response
    /// 
    /// 用于返回响应的 oneshot sender
    response_tx: OneshotSender<Resp>,
}

/// Shared internal state for many-to-one channel
/// 
/// 多对一通道的共享内部状态
struct Inner<Req, Resp: Send> {
    /// Lock-free queue for pending requests
    /// 
    /// 待处理请求的无锁队列
    queue: SegQueue<RequestWrapper<Req, Resp>>,
    
    /// Whether side B (receiver) is closed
    /// 
    /// B 方（接收方）是否已关闭
    b_closed: AtomicBool,
    
    /// Number of active SideA instances
    /// 
    /// 活跃的 SideA 实例数量
    sender_count: AtomicUsize,
    
    /// Waker for side B waiting for requests
    /// 
    /// B 方等待请求的 waker
    b_waker: crate::atomic_waker::AtomicWaker,
}

impl<Req, Resp: Send> std::fmt::Debug for Inner<Req, Resp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sender_count = self.sender_count.load(Ordering::Acquire);
        f.debug_struct("Inner")
            .field("b_closed", &self.is_b_closed())
            .field("sender_count", &sender_count)
            .finish()
    }
}

impl<Req, Resp: Send> Inner<Req, Resp> {
    /// Create new shared state
    /// 
    /// 创建新的共享状态
    #[inline]
    fn new() -> Self {
        Self {
            queue: SegQueue::new(),
            b_closed: AtomicBool::new(false),
            sender_count: AtomicUsize::new(1), // Start with 1 sender
            b_waker: crate::atomic_waker::AtomicWaker::new(),
        }
    }
    
    /// Check if side B is closed
    /// 
    /// 检查 B 方是否已关闭
    #[inline]
    fn is_b_closed(&self) -> bool {
        self.b_closed.load(Ordering::Acquire)
    }
}

/// Side A endpoint (request sender, response receiver) - can be cloned
/// 
/// A 方的 channel 端点（请求发送方，响应接收方）- 可以克隆
pub struct SideA<Req, Resp: Send> {
    inner: Arc<Inner<Req, Resp>>,
}

impl<Req, Resp: Send> std::fmt::Debug for SideA<Req, Resp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideA")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<Req, Resp: Send> Clone for SideA<Req, Resp> {
    fn clone(&self) -> Self {
        // Increment sender count with Relaxed (reads will use Acquire)
        self.inner.sender_count.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: self.inner.clone(),
        }
    }
}

// Drop implementation for SideA to decrement sender count
impl<Req, Resp: Send> Drop for SideA<Req, Resp> {
    fn drop(&mut self) {
        // Decrement sender count with Release ordering to ensure visibility
        if self.inner.sender_count.fetch_sub(1, Ordering::Release) == 1 {
            // This was the last sender, wake up side B
            self.inner.b_waker.wake();
        }
    }
}

/// Side B endpoint (request receiver, response sender) - single instance
/// 
/// B 方的 channel 端点（请求接收方，响应发送方）- 单实例
pub struct SideB<Req, Resp: Send> {
    inner: Arc<Inner<Req, Resp>>,
}

impl<Req, Resp: Send> std::fmt::Debug for SideB<Req, Resp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideB")
            .field("inner", &self.inner)
            .finish()
    }
}

/// Create a new many-to-one request-response channel
/// 
/// Returns (SideA, SideB) tuple. SideA can be cloned to create multiple senders.
/// 
/// 创建一个新的多对一请求-响应 channel
/// 
/// 返回 (SideA, SideB) 元组。SideA 可以克隆以创建多个发送方。
/// 
/// # Example
/// 
/// ```
/// use lite_sync::request_response::many_to_one::channel;
/// 
/// # tokio_test::block_on(async {
/// let (side_a, side_b) = channel::<String, i32>();
/// 
/// // Clone side_a for multiple senders
/// let side_a2 = side_a.clone();
/// 
/// // Side B handles requests
/// tokio::spawn(async move {
///     while let Ok(guard) = side_b.recv_request().await {
///         let response = guard.request().len() as i32;
///         guard.reply(response);
///     }
/// });
/// 
/// // Multiple senders can send concurrently
/// let response1 = side_a.request("Hello".to_string()).await;
/// let response2 = side_a2.request("World".to_string()).await;
/// 
/// assert_eq!(response1, Ok(5));
/// assert_eq!(response2, Ok(5));
/// # });
/// ```
#[inline]
pub fn channel<Req, Resp: Send>() -> (SideA<Req, Resp>, SideB<Req, Resp>) {
    let inner = Arc::new(Inner::new());
    
    let side_a = SideA {
        inner: inner.clone(),
    };
    
    let side_b = SideB {
        inner,
    };
    
    (side_a, side_b)
}

impl<Req, Resp: Send> SideA<Req, Resp> {
    /// Send a request and wait for response
    /// 
    /// This method will:
    /// 1. Push request to the queue
    /// 2. Wait for side B to process and respond
    /// 3. Return the response
    /// 
    /// 发送请求并等待响应
    /// 
    /// 这个方法会：
    /// 1. 将请求推入队列
    /// 2. 等待 B 方处理并响应
    /// 3. 返回响应
    /// 
    /// # Returns
    /// 
    /// - `Ok(response)`: Received response from side B
    /// - `Err(ChannelError::Closed)`: Side B has been closed
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::many_to_one::channel;
    /// # tokio_test::block_on(async {
    /// let (side_a, side_b) = channel::<String, i32>();
    /// 
    /// tokio::spawn(async move {
    ///     while let Ok(guard) = side_b.recv_request().await {
    ///         let len = guard.request().len() as i32;
    ///         guard.reply(len);
    ///     }
    /// });
    /// 
    /// let response = side_a.request("Hello".to_string()).await;
    /// assert_eq!(response, Ok(5));
    /// # });
    /// ```
    pub async fn request(&self, req: Req) -> Result<Resp, ChannelError> {
        // Check if B is closed first
        if self.inner.is_b_closed() {
            return Err(ChannelError::Closed);
        }
        
        // Create oneshot channel for response
        let (response_tx, response_rx) = OneshotSender::<Resp>::new();
        
        // Push request to queue
        self.inner.queue.push(RequestWrapper {
            request: req,
            response_tx,
        });
        
        // Wake up side B
        self.inner.b_waker.wake();
        
        // Wait for response
        response_rx.await.map_err(|_| ChannelError::Closed)
    }
    
    /// Try to send a request without waiting for response
    /// 
    /// Returns a future that will resolve to the response.
    /// 
    /// 尝试发送请求但不等待响应
    /// 
    /// 返回一个 future，将解析为响应。
    pub fn try_request(&self, req: Req) -> Result<impl Future<Output = Result<Resp, ChannelError>>, ChannelError> {
        // Check if B is closed first
        if self.inner.is_b_closed() {
            return Err(ChannelError::Closed);
        }
        
        // Create oneshot channel for response
        let (response_tx, response_rx) = OneshotSender::<Resp>::new();
        
        // Push request to queue
        self.inner.queue.push(RequestWrapper {
            request: req,
            response_tx,
        });
        
        // Wake up side B
        self.inner.b_waker.wake();
        
        // Return future that waits for response
        Ok(async move {
            response_rx.await.map_err(|_| ChannelError::Closed)
        })
    }
}

/// Request guard that enforces B must reply
/// 
/// This guard ensures that B must call `reply()` before dropping the guard.
/// If the guard is dropped without replying, it will panic to prevent A from deadlocking.
/// 
/// 强制 B 必须回复的 Guard
/// 
/// 这个 guard 确保 B 必须在丢弃 guard 之前调用 `reply()`。
/// 如果 guard 在没有回复的情况下被丢弃，会 panic 以防止 A 死锁。
pub struct RequestGuard<Req, Resp>
where
    Req: Send, Resp: Send,
{
    req: Option<Req>,
    response_tx: Option<OneshotSender<Resp>>,
}

impl<Req, Resp> std::fmt::Debug for RequestGuard<Req, Resp>
where
    Req: Send + std::fmt::Debug,
    Resp: Send,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestGuard")
            .field("req", &self.req)
            .finish_non_exhaustive()
    }
}

// PartialEq for testing purposes
impl<Req, Resp> PartialEq for RequestGuard<Req, Resp>
where
    Req: Send + PartialEq,
    Resp: Send,
{
    fn eq(&self, other: &Self) -> bool {
        self.req == other.req
    }
}

impl<Req, Resp> RequestGuard<Req, Resp>
where
    Req: Send, Resp: Send,
{
    /// Get a reference to the request
    /// 
    /// 获取请求内容的引用
    #[inline]
    pub fn request(&self) -> &Req {
        self.req.as_ref().expect("RequestGuard logic error: request already consumed")
    }
    
    /// Consume the guard and send reply
    /// 
    /// This method will send the response back to the requester.
    /// 
    /// 消耗 Guard 并发送回复
    /// 
    /// 这个方法会将响应发送回请求方。
    #[inline]
    pub fn reply(mut self, resp: Resp) {
        if let Some(response_tx) = self.response_tx.take() {
            let _ = response_tx.send(resp);
        }
        // Mark as replied by taking the request
        self.req = None;
    }
}

/// Drop guard: If B drops the guard without calling `reply`, we panic.
/// This enforces the "must reply" protocol.
/// 
/// Drop 守卫：如果 B 不调用 `reply` 就丢弃了 Guard，我们会 panic。
/// 这强制执行了 "必须回复" 的协议。
impl<Req, Resp> Drop for RequestGuard<Req, Resp>
where
    Req: Send, Resp: Send,
{
    fn drop(&mut self) {
        if self.req.is_some() {
            // B dropped the guard without replying
            // This is a protocol error that would cause A to deadlock
            // We must panic to prevent this
            panic!("RequestGuard dropped without replying! This would cause the requester to deadlock. You must call reply() before dropping the guard.");
        }
    }
}

impl<Req, Resp: Send> SideB<Req, Resp> {
    /// Wait for and receive next request, returning a guard that must be replied to
    /// 
    /// The returned `RequestGuard` enforces that you must call `reply()` on it.
    /// If you drop the guard without calling `reply()`, it will panic.
    /// 
    /// 等待并接收下一个请求，返回一个必须回复的 guard
    /// 
    /// 返回的 `RequestGuard` 强制你必须调用 `reply()`。
    /// 如果你在没有调用 `reply()` 的情况下丢弃 guard，会 panic。
    /// 
    /// # Returns
    /// 
    /// - `Ok(RequestGuard)`: Received request from a side A
    /// - `Err(ChannelError::Closed)`: All side A instances have been closed
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::many_to_one::channel;
    /// # tokio_test::block_on(async {
    /// let (side_a, side_b) = channel::<String, i32>();
    /// 
    /// tokio::spawn(async move {
    ///     while let Ok(guard) = side_b.recv_request().await {
    ///         let len = guard.request().len() as i32;
    ///         guard.reply(len);
    ///     }
    /// });
    /// 
    /// let response = side_a.request("Hello".to_string()).await;
    /// assert_eq!(response, Ok(5));
    /// # });
    /// ```
    pub async fn recv_request(&self) -> Result<RequestGuard<Req, Resp>, ChannelError>
    where
        Req: Send,
        Resp: Send,
    {
        RecvRequest {
            inner: &self.inner,
            registered: false,
        }.await
    }
    
    /// Convenient method to handle request and send response
    /// 
    /// This method will:
    /// 1. Wait for and receive request
    /// 2. Call the handler function
    /// 3. Send the response via the guard
    /// 
    /// 处理请求并发送响应的便捷方法
    /// 
    /// 这个方法会：
    /// 1. 等待并接收请求
    /// 2. 调用处理函数
    /// 3. 通过 guard 发送响应
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::many_to_one::channel;
    /// # tokio_test::block_on(async {
    /// let (side_a, side_b) = channel::<String, i32>();
    /// 
    /// tokio::spawn(async move {
    ///     while side_b.handle_request(|req| req.len() as i32).await.is_ok() {
    ///         // Continue handling
    ///     }
    /// });
    /// 
    /// let response = side_a.request("Hello".to_string()).await;
    /// assert_eq!(response, Ok(5));
    /// # });
    /// ```
    pub async fn handle_request<F>(&self, handler: F) -> Result<(), ChannelError>
    where
        Req: Send,
        Resp: Send,
        F: FnOnce(&Req) -> Resp,
    {
        let guard = self.recv_request().await?;
        let resp = handler(guard.request());
        guard.reply(resp);
        Ok(())
    }
    
    /// Convenient async method to handle request and send response
    /// 
    /// Similar to `handle_request`, but supports async handler functions.
    /// Note: The handler takes ownership of the request to avoid lifetime issues.
    /// 
    /// 处理请求并发送响应的异步便捷方法
    /// 
    /// 与 `handle_request` 类似，但支持异步处理函数。
    /// 注意：处理函数会获取请求的所有权以避免生命周期问题。
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::many_to_one::channel;
    /// # tokio_test::block_on(async {
    /// let (side_a, side_b) = channel::<String, String>();
    /// 
    /// tokio::spawn(async move {
    ///     while side_b.handle_request_async(|req| async move {
    ///         // Async processing - req is owned
    ///         req.to_uppercase()
    ///     }).await.is_ok() {
    ///         // Continue handling
    ///     }
    /// });
    /// 
    /// let response = side_a.request("hello".to_string()).await;
    /// assert_eq!(response, Ok("HELLO".to_string()));
    /// # });
    /// ```
    pub async fn handle_request_async<F, Fut>(&self, handler: F) -> Result<(), ChannelError>
    where
        Req: Send,
        Resp: Send,
        F: FnOnce(Req) -> Fut,
        Fut: Future<Output = Resp>,
    {
        let mut guard = self.recv_request().await?;
        let req = guard.req.take().expect("RequestGuard logic error: request already consumed");
        let resp = handler(req).await;
        
        // Manually send the reply since we've consumed the request
        if let Some(response_tx) = guard.response_tx.take() {
            let _ = response_tx.send(resp);
        }
        // Mark as replied
        guard.req = None;
        
        Ok(())
    }
}

// Drop implementation to clean up
impl<Req, Resp: Send> Drop for SideB<Req, Resp> {
    fn drop(&mut self) {
        // Side B closed, notify any waiting senders
        self.inner.b_closed.store(true, Ordering::Release);
        
        // Drain queue and drop all pending response channels
        // The generic oneshot will handle cleanup automatically
        while let Some(_wrapper) = self.inner.queue.pop() {
            // Just drop the wrapper, oneshot cleanup is automatic
        }
    }
}

/// Future: Side B receives request
struct RecvRequest<'a, Req, Resp: Send> {
    inner: &'a Inner<Req, Resp>,
    registered: bool,
}

// RecvRequest is Unpin because it only holds references and a bool
impl<Req, Resp: Send> Unpin for RecvRequest<'_, Req, Resp> {}

impl<Req, Resp> Future for RecvRequest<'_, Req, Resp>
where
    Req: Send,
    Resp: Send,
{
    type Output = Result<RequestGuard<Req, Resp>, ChannelError>;
    
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Try to pop from queue
        if let Some(wrapper) = self.inner.queue.pop() {
            return Poll::Ready(Ok(RequestGuard {
                req: Some(wrapper.request),
                response_tx: Some(wrapper.response_tx),
            }));
        }
        
        // Check if there are any senders left
        if self.inner.sender_count.load(Ordering::Acquire) == 0 {
            return Poll::Ready(Err(ChannelError::Closed));
        }
        
        // Register waker if not already registered
        if !self.registered {
            self.inner.b_waker.register(cx.waker());
            self.registered = true;
        }
        
        // Always check queue and sender_count again before returning Pending
        // This is critical to avoid deadlock when senders drop after waker is registered
        if let Some(wrapper) = self.inner.queue.pop() {
            return Poll::Ready(Ok(RequestGuard {
                req: Some(wrapper.request),
                response_tx: Some(wrapper.response_tx),
            }));
        }
        
        // Final check if there are any senders
        if self.inner.sender_count.load(Ordering::Acquire) == 0 {
            return Poll::Ready(Err(ChannelError::Closed));
        }
        
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_basic_many_to_one() {
        let (side_a, side_b) = channel::<String, i32>();
        
        tokio::spawn(async move {
            while let Ok(guard) = side_b.recv_request().await {
                let response = guard.request().len() as i32;
                guard.reply(response);
            }
        });
        
        let response = side_a.request("Hello".to_string()).await;
        assert_eq!(response, Ok(5));
    }

    #[tokio::test]
    async fn test_multiple_senders() {
        let (side_a, side_b) = channel::<i32, i32>();
        let side_a2 = side_a.clone();
        let side_a3 = side_a.clone();
        
        tokio::spawn(async move {
            while let Ok(guard) = side_b.recv_request().await {
                let result = *guard.request() * 2;
                guard.reply(result);
            }
        });
        
        let handle1 = tokio::spawn(async move {
            let mut sum = 0;
            for i in 0..10 {
                let resp = side_a.request(i).await.unwrap();
                sum += resp;
            }
            sum
        });
        
        let handle2 = tokio::spawn(async move {
            let mut sum = 0;
            for i in 10..20 {
                let resp = side_a2.request(i).await.unwrap();
                sum += resp;
            }
            sum
        });
        
        let handle3 = tokio::spawn(async move {
            let mut sum = 0;
            for i in 20..30 {
                let resp = side_a3.request(i).await.unwrap();
                sum += resp;
            }
            sum
        });
        
        let sum1 = handle1.await.unwrap();
        let sum2 = handle2.await.unwrap();
        let sum3 = handle3.await.unwrap();
        
        // Each range should give: sum(i*2) = 2 * sum(i)
        assert_eq!(sum1, 2 * (0..10).sum::<i32>());
        assert_eq!(sum2, 2 * (10..20).sum::<i32>());
        assert_eq!(sum3, 2 * (20..30).sum::<i32>());
    }

    #[tokio::test]
    async fn test_side_b_closes() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        // Side A closes immediately
        drop(side_a);
        
        // Side B should receive Err
        let request = side_b.recv_request().await;
        assert!(request.is_err());
    }

    #[tokio::test]
    async fn test_all_side_a_close() {
        let (side_a, side_b) = channel::<i32, i32>();
        let side_a2 = side_a.clone();
        
        // All side A instances close
        drop(side_a);
        drop(side_a2);
        
        // Side B should receive Err
        let request = side_b.recv_request().await;
        assert!(request.is_err());
    }

    #[tokio::test]
    async fn test_handle_request() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        tokio::spawn(async move {
            while side_b.handle_request(|req| req * 3).await.is_ok() {
                // Continue handling
            }
        });
        
        for i in 0..5 {
            let response = side_a.request(i).await.unwrap();
            assert_eq!(response, i * 3);
        }
    }

    #[tokio::test]
    async fn test_handle_request_async() {
        let (side_a, side_b) = channel::<String, usize>();
        
        tokio::spawn(async move {
            while side_b.handle_request_async(|req| async move {
                sleep(Duration::from_millis(10)).await;
                req.len()
            }).await.is_ok() {
                // Continue handling
            }
        });
        
        let test_strings = vec!["Hello", "World", "Rust"];
        for s in test_strings {
            let response = side_a.request(s.to_string()).await.unwrap();
            assert_eq!(response, s.len());
        }
    }

    #[tokio::test]
    async fn test_concurrent_requests() {
        let (side_a, side_b) = channel::<String, String>();
        
        tokio::spawn(async move {
            while side_b.handle_request_async(|req| async move {
                sleep(Duration::from_millis(5)).await;
                req.to_uppercase()
            }).await.is_ok() {
                // Continue
            }
        });
        
        // Send multiple requests concurrently
        let mut handles = vec![];
        for i in 0..10 {
            let side_a_clone = side_a.clone();
            let handle = tokio::spawn(async move {
                let msg = format!("message{}", i);
                let resp = side_a_clone.request(msg.clone()).await.unwrap();
                assert_eq!(resp, msg.to_uppercase());
            });
            handles.push(handle);
        }
        
        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_request_guard_must_reply() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        let handle = tokio::spawn(async move {
            let _guard = side_b.recv_request().await.unwrap();
            // Intentionally not calling reply() - this should panic
        });
        
        // Send a request
        tokio::spawn(async move {
            let _ = side_a.request(42).await;
        });
        
        // Wait for the spawned task and verify it panicked
        let result = handle.await;
        assert!(result.is_err(), "Task should have panicked");
        
        // Verify the panic message contains our expected text
        if let Err(e) = result {
            if let Ok(panic_payload) = e.try_into_panic() {
                if let Some(s) = panic_payload.downcast_ref::<String>() {
                    assert!(s.contains("RequestGuard dropped without replying"), 
                        "Panic message should mention RequestGuard: {}", s);
                } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    assert!(s.contains("RequestGuard dropped without replying"), 
                        "Panic message should mention RequestGuard: {}", s);
                } else {
                    panic!("Unexpected panic type");
                }
            }
        }
    }
}
