/// Bidirectional request-response channels
/// 
/// This module provides request-response communication patterns optimized for
/// different use cases. Supports one-to-one and many-to-one communication.
/// 
/// 双向请求-响应通道
/// 
/// 此模块提供针对不同用例优化的请求-响应通信模式。
/// 支持一对一和多对一通信。

mod common;
pub mod one_to_one;
#[cfg(feature = "crossbeam-queue")]
pub mod many_to_one;

// Re-export core types
pub use common::ChannelError;