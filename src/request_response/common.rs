/// Core types and utilities for request-response channels
/// 
/// 请求-响应通道的核心类型和工具
use std::fmt;

/// Error type for channel operations
/// 
/// Channel 操作的错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    /// The remote side has been closed
    /// 
    /// 对端已关闭
    Closed,
    
    /// The channel is full (for bounded channels)
    /// 
    /// 通道已满（用于有界通道）
    Full,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelError::Closed => write!(f, "channel closed"),
            ChannelError::Full => write!(f, "channel full"),
        }
    }
}

impl std::error::Error for ChannelError {}

/// State constants for request-response state machine
/// 
/// 请求-响应状态机的状态常量
pub mod state {
    /// Channel is idle, ready to accept new request
    /// 
    /// 通道空闲，准备接受新请求
    pub const IDLE: u8 = 0;
    
    /// Request sent, waiting for response
    /// 
    /// 请求已发送，等待响应
    pub const WAITING_RESPONSE: u8 = 1;
    
    /// Response is ready to be received
    /// 
    /// 响应已就绪，可以接收
    pub const RESPONSE_READY: u8 = 2;
}
