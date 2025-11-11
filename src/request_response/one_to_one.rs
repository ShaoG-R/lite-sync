/// Lightweight bidirectional request-response channel for one-to-one communication
/// 
/// Optimized for strict request-response pattern where side A sends requests
/// and side B must respond before A can send the next request. No buffer needed.
/// 
/// 轻量级一对一双向请求-响应通道
/// 
/// 为严格的请求-响应模式优化，A方发送请求，B方必须响应后A方才能发送下一个请求。
/// 无需缓冲区。
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU8, AtomicBool, Ordering};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::sync::Arc;

use crate::atomic_waker::AtomicWaker;
use super::common::{ChannelError, state};

// Channel states - re-export from core
const IDLE: u8 = state::IDLE;
const WAITING_RESPONSE: u8 = state::WAITING_RESPONSE;
const RESPONSE_READY: u8 = state::RESPONSE_READY;

/// Internal state for bidirectional request-response channel
/// 
/// Uses atomic operations and UnsafeCell for lock-free bidirectional communication.
/// State machine ensures strict request-response ordering.
/// 
/// 双向请求-响应 channel 的内部状态
/// 
/// 使用原子操作和 UnsafeCell 实现无锁的双向通信。
/// 通过状态机确保严格的请求-响应顺序。
struct Inner<Req, Resp> {
    /// Channel state
    state: AtomicU8,
    
    /// Waker for A waiting for response
    a_waker: AtomicWaker,
    
    /// Waker for B waiting for request
    b_waker: AtomicWaker,
    
    /// Whether side A is closed
    a_closed: AtomicBool,
    
    /// Whether side B is closed
    b_closed: AtomicBool,
    
    /// Storage for request data
    request: UnsafeCell<MaybeUninit<Req>>,
    
    /// Storage for response data
    response: UnsafeCell<MaybeUninit<Resp>>,
}

// SAFETY: Access to UnsafeCell is synchronized via atomic state machine
unsafe impl<Req: Send, Resp: Send> Send for Inner<Req, Resp> {}
unsafe impl<Req: Send, Resp: Send> Sync for Inner<Req, Resp> {}

impl<Req, Resp> std::fmt::Debug for Inner<Req, Resp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.current_state();
        let state_str = match state {
            IDLE => "Idle",
            WAITING_RESPONSE => "WaitingResponse",
            RESPONSE_READY => "ResponseReady",
            _ => "Unknown",
        };
        f.debug_struct("Inner")
            .field("state", &state_str)
            .field("a_closed", &self.is_a_closed())
            .field("b_closed", &self.is_b_closed())
            .finish()
    }
}

impl<Req, Resp> Inner<Req, Resp> {
    /// Create new channel internal state
    /// 
    /// 创建新的 channel 内部状态
    #[inline]
    fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            a_waker: AtomicWaker::new(),
            b_waker: AtomicWaker::new(),
            a_closed: AtomicBool::new(false),
            b_closed: AtomicBool::new(false),
            request: UnsafeCell::new(MaybeUninit::uninit()),
            response: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
    
    /// Try to send request (state transition: IDLE -> WAITING_RESPONSE)
    /// 
    /// 尝试发送请求（状态转换：IDLE -> WAITING_RESPONSE）
    #[inline]
    fn try_send_request(&self) -> bool {
        self.state.compare_exchange(
            IDLE,
            WAITING_RESPONSE,
            Ordering::AcqRel,
            Ordering::Acquire,
        ).is_ok()
    }
    
    /// Mark response as ready (state transition: WAITING_RESPONSE -> RESPONSE_READY)
    /// 
    /// 标记响应已就绪（状态转换：WAITING_RESPONSE -> RESPONSE_READY）
    #[inline]
    fn mark_response_ready(&self) {
        self.state.store(RESPONSE_READY, Ordering::Release);
    }
    
    /// Complete response reception (state transition: RESPONSE_READY -> IDLE)
    /// 
    /// 完成响应接收（状态转换：RESPONSE_READY -> IDLE）
    #[inline]
    fn complete_response(&self) {
        self.state.store(IDLE, Ordering::Release);
    }
    
    /// Check if side A is closed
    /// 
    /// 检查 A 方是否已关闭
    #[inline]
    fn is_a_closed(&self) -> bool {
        self.a_closed.load(Ordering::Acquire)
    }
    
    /// Check if side B is closed
    /// 
    /// 检查 B 方是否已关闭
    #[inline]
    fn is_b_closed(&self) -> bool {
        self.b_closed.load(Ordering::Acquire)
    }
    
    /// Get current state
    /// 
    /// 获取当前状态
    #[inline]
    fn current_state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }
}

/// Side A endpoint (request sender, response receiver)
/// 
/// A 方的 channel 端点（请求发送方，响应接收方）
pub struct SideA<Req, Resp> {
    inner: Arc<Inner<Req, Resp>>,
}

impl<Req, Resp> std::fmt::Debug for SideA<Req, Resp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideA")
            .field("inner", &self.inner)
            .finish()
    }
}

/// Side B endpoint (request receiver, response sender)
/// 
/// B 方的 channel 端点（请求接收方，响应发送方）
pub struct SideB<Req, Resp> {
    inner: Arc<Inner<Req, Resp>>,
}

impl<Req, Resp> std::fmt::Debug for SideB<Req, Resp> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SideB")
            .field("inner", &self.inner)
            .finish()
    }
}

/// Create a new request-response channel
/// 
/// Returns (SideA, SideB) tuple representing both ends of the channel.
/// 
/// 创建一个新的请求-响应 channel
/// 
/// 返回 (SideA, SideB) 元组，分别代表 channel 的两端。
/// 
/// # Example
/// 
/// ```
/// use lite_sync::request_response::one_to_one::channel;
/// 
/// # tokio_test::block_on(async {
/// let (side_a, side_b) = channel::<String, i32>();
/// 
/// // Side B uses convenient handle_request method
/// tokio::spawn(async move {
///     while side_b.handle_request(|request| request.len() as i32).await.is_ok() {
///         // Continue handling requests
///     }
/// });
/// 
/// let response = side_a.request("Hello".to_string()).await;
/// assert_eq!(response, Ok(5));
/// # });
/// ```
/// 
/// # Advanced Example: Async Processing
/// 
/// ```
/// use lite_sync::request_response::one_to_one::channel;
/// 
/// # tokio_test::block_on(async {
/// let (side_a, side_b) = channel::<String, String>();
/// 
/// tokio::spawn(async move {
///     while side_b.handle_request_async(|req| async move {
///         // Async processing logic
///         req.to_uppercase()
///     }).await.is_ok() {
///         // Continue handling
///     }
/// });
/// 
/// let result = side_a.request("hello".to_string()).await;
/// assert_eq!(result, Ok("HELLO".to_string()));
/// # });
/// ```
#[inline]
pub fn channel<Req, Resp>() -> (SideA<Req, Resp>, SideB<Req, Resp>) {
    let inner = Arc::new(Inner::new());
    
    let side_a = SideA {
        inner: inner.clone(),
    };
    
    let side_b = SideB {
        inner,
    };
    
    (side_a, side_b)
}

impl<Req, Resp> SideA<Req, Resp> {
    /// Send a request and wait for response
    /// 
    /// This method will:
    /// 1. Wait for channel to be idle (if previous request is still being processed)
    /// 2. Send request to side B
    /// 3. Wait for side B's response
    /// 4. Return the response
    /// 
    /// 发送请求并等待响应
    /// 
    /// 这个方法会：
    /// 1. 等待 channel 进入空闲状态（如果之前的请求还在处理中）
    /// 2. 发送请求到 B 方
    /// 3. 等待 B 方的响应
    /// 4. 返回响应
    /// 
    /// # Returns
    /// 
    /// - `Ok(response)`: Received response from side B
    /// - `Err(ChannelError::Closed)`: Side B has been closed
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::one_to_one::channel;
    /// # tokio_test::block_on(async {
    /// let (side_a, side_b) = channel::<String, i32>();
    /// 
    /// tokio::spawn(async move {
    ///     while let Ok(guard) = side_b.recv_request().await {
    ///         let response = guard.request().len() as i32;
    ///         guard.reply(response);
    ///     }
    /// });
    /// 
    /// let response = side_a.request("Hello".to_string()).await;
    /// assert_eq!(response, Ok(5));
    /// # });
    /// ```
    pub async fn request(&self, req: Req) -> Result<Resp, ChannelError> {
        // Send request
        self.send_request(req).await?;
        
        // Wait for response
        self.recv_response().await
    }
    
    /// Send request (without waiting for response)
    /// 
    /// Will wait if channel is not in idle state.
    /// 
    /// 发送请求（不等待响应）
    /// 
    /// 如果 channel 不在空闲状态，会等待直到可以发送。
    async fn send_request(&self, req: Req) -> Result<(), ChannelError> {
        SendRequest {
            inner: &self.inner,
            request: Some(req),
            registered: false,
        }.await
    }
    
    /// Wait for and receive response
    /// 
    /// 等待并接收响应
    /// 
    /// # Returns
    /// 
    /// - `Ok(response)`: Received response
    /// - `Err(ChannelError::Closed)`: Side B closed
    async fn recv_response(&self) -> Result<Resp, ChannelError> {
        RecvResponse {
            inner: &self.inner,
            registered: false,
        }.await
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
pub struct RequestGuard<'a, Req, Resp>
where
    Req: Send, Resp: Send,
{
    inner: &'a Inner<Req, Resp>,
    req: Option<Req>,
    replied: bool,
}

impl<'a, Req, Resp> std::fmt::Debug for RequestGuard<'a, Req, Resp>
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

// PartialEq for testing purposes - comparing RequestGuards doesn't make sense
// but we need it for Result<RequestGuard, _> comparisons in tests
impl<'a, Req, Resp> PartialEq for RequestGuard<'a, Req, Resp>
where
    Req: Send + PartialEq,
    Resp: Send,
{
    fn eq(&self, other: &Self) -> bool {
        // Two guards are equal if they hold the same request value
        // This is mainly for testing purposes
        self.req == other.req
    }
}

impl<'a, Req, Resp> RequestGuard<'a, Req, Resp>
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
    /// This method will:
    /// 1. Store the response
    /// 2. Update state to RESPONSE_READY
    /// 3. Wake up side A
    /// 
    /// 消耗 Guard 并发送回复
    /// 
    /// 这个方法会：
    /// 1. 存储响应
    /// 2. 更新状态为 RESPONSE_READY
    /// 3. 唤醒 A 方
    #[inline]
    pub fn reply(mut self, resp: Resp) {
        // Store response
        // SAFETY: Side B has exclusive access to response storage
        unsafe {
            (*self.inner.response.get()).write(resp);
        }
        
        // Mark state as RESPONSE_READY
        self.inner.mark_response_ready();
        
        // Wake up side A
        self.inner.a_waker.wake();
        
        // Mark as replied (prevent Drop panic)
        self.replied = true;
    }
    
}

/// Drop guard: If B drops the guard without calling `reply`, we panic.
/// This enforces the "must reply" protocol.
/// 
/// Drop 守卫：如果 B 不调用 `reply` 就丢弃了 Guard，我们会 panic。
/// 这强制执行了 "必须回复" 的协议。
impl<'a, Req, Resp> Drop for RequestGuard<'a, Req, Resp>
where
    Req: Send, Resp: Send,
{
    fn drop(&mut self) {
        if !self.replied {
            // B dropped the guard without replying
            // This is a protocol error that would cause A to deadlock
            // We must panic to prevent this
            panic!("RequestGuard dropped without replying! This would cause the requester to deadlock. You must call reply() before dropping the guard.");
        }
    }
}

impl<Req, Resp> SideB<Req, Resp> {
    /// Wait for and receive request, returning a guard that must be replied to
    /// 
    /// The returned `RequestGuard` enforces that you must call `reply()` on it.
    /// If you drop the guard without calling `reply()`, it will panic.
    /// 
    /// 等待并接收请求，返回一个必须回复的 guard
    /// 
    /// 返回的 `RequestGuard` 强制你必须调用 `reply()`。
    /// 如果你在没有调用 `reply()` 的情况下丢弃 guard，会 panic。
    /// 
    /// # Returns
    /// 
    /// - `Ok(RequestGuard)`: Received request from side A
    /// - `Err(ChannelError::Closed)`: Side A has been closed
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::one_to_one::channel;
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
    pub async fn recv_request(&self) -> Result<RequestGuard<'_, Req, Resp>, ChannelError>
    where
        Req: Send,
        Resp: Send,
    {
        let req = RecvRequest {
            inner: &self.inner,
            registered: false,
        }.await?;
        
        Ok(RequestGuard {
            inner: &self.inner,
            req: Some(req),
            replied: false,
        })
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
    /// # Returns
    /// 
    /// - `Ok(())`: Successfully handled request and sent response
    /// - `Err(ChannelError::Closed)`: Side A closed
    /// 
    /// # Example
    /// 
    /// ```
    /// # use lite_sync::request_response::one_to_one::channel;
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
    /// # use lite_sync::request_response::one_to_one::channel;
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
        unsafe {
            (*guard.inner.response.get()).write(resp);
        }
        guard.inner.mark_response_ready();
        guard.inner.a_waker.wake();
        guard.replied = true;
        
        Ok(())
    }
}

// Drop implementations to clean up wakers
impl<Req, Resp> Drop for SideA<Req, Resp> {
    fn drop(&mut self) {
        // Side A closed, wake up side B that might be waiting
        self.inner.a_closed.store(true, Ordering::Release);
        self.inner.b_waker.wake();
    }
}

impl<Req, Resp> Drop for SideB<Req, Resp> {
    fn drop(&mut self) {
        // Side B closed, wake up side A that might be waiting
        self.inner.b_closed.store(true, Ordering::Release);
        self.inner.a_waker.wake();
    }
}

/// Future: Side A sends request
struct SendRequest<'a, Req, Resp> {
    inner: &'a Inner<Req, Resp>,
    request: Option<Req>,
    registered: bool,
}

// SendRequest is Unpin because we only need to move data, not pin it
impl<Req, Resp> Unpin for SendRequest<'_, Req, Resp> {}

impl<Req, Resp> Future for SendRequest<'_, Req, Resp> {
    type Output = Result<(), ChannelError>;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        
        // Check if side B is closed
        if this.inner.is_b_closed() {
            return Poll::Ready(Err(ChannelError::Closed));
        }
        
        // Try to send request
        if this.inner.try_send_request() {
            // Successfully sent request
            // SAFETY: We have exclusive access (guaranteed by state machine)
            unsafe {
                (*this.inner.request.get()).write(this.request.take().unwrap());
            }
            
            // Wake up side B
            this.inner.b_waker.wake();
            
            return Poll::Ready(Ok(()));
        }
        
        // Channel busy, register waker and wait
        if !this.registered {
            this.inner.a_waker.register(cx.waker());
            this.registered = true;
            
            // Check again (avoid race condition)
            if this.inner.is_b_closed() {
                return Poll::Ready(Err(ChannelError::Closed));
            }
            
            if this.inner.current_state() == IDLE {
                cx.waker().wake_by_ref();
            }
        }
        
        Poll::Pending
    }
}

/// Future: Side A receives response
struct RecvResponse<'a, Req, Resp> {
    inner: &'a Inner<Req, Resp>,
    registered: bool,
}

// RecvResponse is Unpin because it only holds references and a bool
impl<Req, Resp> Unpin for RecvResponse<'_, Req, Resp> {}

impl<Req, Resp> Future for RecvResponse<'_, Req, Resp> {
    type Output = Result<Resp, ChannelError>;
    
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check if state is RESPONSE_READY first
        // Try to receive sent response even if B closed
        if self.inner.current_state() == RESPONSE_READY {
            // Take the response
            // SAFETY: Response must exist when state is RESPONSE_READY
            let response = unsafe {
                (*self.inner.response.get()).assume_init_read()
            };
            
            // Reset to IDLE for next round
            self.inner.complete_response();
            
            return Poll::Ready(Ok(response));
        }
        
        // Check if B closed (after confirming no response)
        if self.inner.is_b_closed() {
            return Poll::Ready(Err(ChannelError::Closed));
        }
        
        // Register waker
        if !self.registered {
            self.inner.a_waker.register(cx.waker());
            self.registered = true;
            
            // Check state again (avoid race condition)
            if self.inner.current_state() == RESPONSE_READY {
                let response = unsafe {
                    (*self.inner.response.get()).assume_init_read()
                };
                self.inner.complete_response();
                return Poll::Ready(Ok(response));
            }
            
            // Check again if B closed
            if self.inner.is_b_closed() {
                return Poll::Ready(Err(ChannelError::Closed));
            }
        }
        
        Poll::Pending
    }
}

/// Future: Side B receives request
struct RecvRequest<'a, Req, Resp> {
    inner: &'a Inner<Req, Resp>,
    registered: bool,
}

// RecvRequest is Unpin because it only holds references and a bool
impl<Req, Resp> Unpin for RecvRequest<'_, Req, Resp> {}

impl<Req, Resp> Future for RecvRequest<'_, Req, Resp> {
    type Output = Result<Req, ChannelError>;
    
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check state first, then access data (fixes race condition)
        // Try to receive sent request even if A closed
        if self.inner.current_state() == WAITING_RESPONSE {
            // Take the request
            // SAFETY: Request must exist when state is WAITING_RESPONSE
            let request = unsafe {
                (*self.inner.request.get()).assume_init_read()
            };
            
            return Poll::Ready(Ok(request));
        }
        
        // Check if A closed (after confirming no request)
        if self.inner.is_a_closed() {
            return Poll::Ready(Err(ChannelError::Closed));
        }
        
        // Register waker
        if !self.registered {
            self.inner.b_waker.register(cx.waker());
            self.registered = true;
            
            // Check again (avoid race condition)
            if self.inner.current_state() == WAITING_RESPONSE {
                let request = unsafe {
                    (*self.inner.request.get()).assume_init_read()
                };
                return Poll::Ready(Ok(request));
            }
            
            // Check again if A closed
            if self.inner.is_a_closed() {
                return Poll::Ready(Err(ChannelError::Closed));
            }
        }
        
        Poll::Pending
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_basic_request_response() {
        let (side_a, side_b) = channel::<String, i32>();
        
        tokio::spawn(async move {
            let guard = side_b.recv_request().await.unwrap();
            assert_eq!(guard.request(), "Hello");
            guard.reply(42);
        });
        
        let response = side_a.request("Hello".to_string()).await;
        assert_eq!(response, Ok(42));
    }

    #[tokio::test]
    async fn test_multiple_rounds() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        tokio::spawn(async move {
            for i in 0..5 {
                let guard = side_b.recv_request().await.unwrap();
                assert_eq!(*guard.request(), i);
                guard.reply(i * 2);
            }
        });
        
        for i in 0..5 {
            let response = side_a.request(i).await;
            assert_eq!(response, Ok(i * 2));
        }
    }

    #[tokio::test]
    async fn test_delayed_response() {
        let (side_a, side_b) = channel::<String, String>();
        
        tokio::spawn(async move {
            let guard = side_b.recv_request().await.unwrap();
            sleep(Duration::from_millis(50)).await;
            let response = guard.request().to_uppercase();
            guard.reply(response);
        });
        
        let response = side_a.request("hello".to_string()).await;
        assert_eq!(response, Ok("HELLO".to_string()));
    }

    #[tokio::test]
    async fn test_side_b_closes() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        // Side B closes immediately
        drop(side_b);
        
        // Side A should receive Err
        let response = side_a.request(42).await;
        assert_eq!(response, Err(ChannelError::Closed));
    }

    #[tokio::test]
    async fn test_side_a_closes() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        // Side A closes immediately
        drop(side_a);
        
        // Side B should receive Err
        let request = side_b.recv_request().await;
        assert_eq!(request, Err(ChannelError::Closed));
    }

    #[tokio::test]
    async fn test_concurrent_requests() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        let handle_b = tokio::spawn(async move {
            let mut count = 0;
            loop {
                if let Ok(guard) = side_b.recv_request().await {
                    count += 1;
                    let response = *guard.request() * 2;
                    guard.reply(response);
                } else {
                    break;
                }
            }
            count
        });
        
        let handle_a = tokio::spawn(async move {
            for i in 0..10 {
                let response = side_a.request(i).await.unwrap();
                assert_eq!(response, i * 2);
            }
            drop(side_a);
        });
        
        handle_a.await.unwrap();
        let count = handle_b.await.unwrap();
        assert_eq!(count, 10);
    }

    #[tokio::test]
    async fn test_string_messages() {
        let (side_a, side_b) = channel::<String, String>();
        
        tokio::spawn(async move {
            loop {
                if let Ok(guard) = side_b.recv_request().await {
                    let response = format!("Echo: {}", guard.request());
                    guard.reply(response);
                } else {
                    break;
                }
            }
        });
        
        let messages = vec!["Hello", "World", "Rust"];
        for msg in messages {
            let response = side_a.request(msg.to_string()).await.unwrap();
            assert_eq!(response, format!("Echo: {}", msg));
        }
    }

    #[tokio::test]
    async fn test_handle_request() {
        let (side_a, side_b) = channel::<i32, i32>();
        
        tokio::spawn(async move {
            // Using handle_request convenience method
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
            // Using handle_request_async async convenience method
            while side_b.handle_request_async(|req| async move {
                sleep(Duration::from_millis(10)).await;
                req.len()
            }).await.is_ok() {
                // Continue handling
            }
        });
        
        let test_strings = vec!["Hello", "World", "Rust", "Async"];
        for s in test_strings {
            let response = side_a.request(s.to_string()).await.unwrap();
            assert_eq!(response, s.len());
        }
    }

    #[tokio::test]
    async fn test_error_display() {
        // Test Display implementation for error types
        assert_eq!(format!("{}", ChannelError::Closed), "channel closed");
    }

    #[tokio::test]
    async fn test_multiple_handle_request_rounds() {
        let (side_a, side_b) = channel::<String, String>();
        
        let handle = tokio::spawn(async move {
            let mut count = 0;
            // Manually handle to maintain state
            while let Ok(guard) = side_b.recv_request().await {
                count += 1;
                let resp = format!("{}:{}", count, guard.request().to_uppercase());
                guard.reply(resp);
            }
            count
        });
        
        let response1 = side_a.request("hello".to_string()).await.unwrap();
        assert_eq!(response1, "1:HELLO");
        
        let response2 = side_a.request("world".to_string()).await.unwrap();
        assert_eq!(response2, "2:WORLD");
        
        let response3 = side_a.request("rust".to_string()).await.unwrap();
        assert_eq!(response3, "3:RUST");
        
        drop(side_a);
        let count = handle.await.unwrap();
        assert_eq!(count, 3);
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
