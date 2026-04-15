//! 消息通道适配器模块
//!
//! 定义了消息通道的核心抽象（`Channel` trait）和消息发送协议。
//! 通道负责将消息投递到具体的目标（如 WebSocket、HTTP、消息队列等），
//! 并提供流式传输能力（`StreamingChannel`）用于打字指示器和分块发送。
//! 同时包含 `TestChannel` 用于单元测试中的消息通道模拟。

use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

/// 消息发送结果，封装成功/失败状态及可选的返回数据
#[derive(Debug, Clone, PartialEq)]
pub struct SendResult {
    pub success: bool,
    pub error: Option<String>,
    pub data: Option<Value>,
}

/// 判断发送结果是否成功
pub fn success(result: &SendResult) -> bool {
    result.success
}

/// 判断发送结果是否失败
pub fn error(result: &SendResult) -> bool {
    !result.success
}

/// 构造一个成功的发送结果
pub fn ok(data: Option<Value>) -> SendResult {
    SendResult {
        success: true,
        error: None,
        data,
    }
}

/// 构造一个失败的发送结果
pub fn err(message: impl Into<String>, data: Option<Value>) -> SendResult {
    SendResult {
        success: false,
        error: Some(message.into()),
        data,
    }
}

/// 消息通道核心 trait
///
/// 所有具体通道（Discord、Telegram、WebSocket 等）都需要实现此 trait。
/// 要求 Send + Sync 以支持跨线程共享。
pub trait Channel: Send + Sync {
    fn start(&self);
    fn stop(&self);
    fn send_message(&self, target: &str, message: &str, opts: SendOptions) -> SendResult;
    fn channel_name(&self) -> &str;
    fn health_check(&self) -> bool;
}

/// 消息发送选项，支持附带媒体文件和指定处理阶段
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendOptions {
    pub media: Vec<String>,
    pub stage: Option<String>,
}

/// 测试用模拟通道，记录所有发送的消息，可配置为始终失败模式
#[derive(Debug)]
pub struct TestChannel {
    name: String,
    running: AtomicBool,
    should_fail: bool,
    sent_messages: Mutex<Vec<SentMessage>>,
}

impl TestChannel {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            running: AtomicBool::new(false),
            should_fail: false,
            sent_messages: Mutex::new(Vec::new()),
        }
    }

    /// 创建一个始终失败的测试通道，用于测试错误处理路径
    pub fn failing(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            running: AtomicBool::new(false),
            should_fail: true,
            sent_messages: Mutex::new(Vec::new()),
        }
    }

    pub fn sent_messages(&self) -> Vec<SentMessage> {
        self.sent_messages.lock().clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// 已发送消息的记录，用于测试断言
#[derive(Debug, Clone, PartialEq)]
pub struct SentMessage {
    pub target: String,
    pub message: String,
    pub opts: SendOptions,
}

impl Channel for TestChannel {
    fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn send_message(&self, target: &str, message: &str, opts: SendOptions) -> SendResult {
        if self.should_fail {
            return err("Mock send failed", None);
        }
        self.sent_messages.lock().push(SentMessage {
            target: target.to_string(),
            message: message.to_string(),
            opts,
        });
        ok(None)
    }

    fn channel_name(&self) -> &str {
        &self.name
    }

    fn health_check(&self) -> bool {
        self.running.load(Ordering::SeqCst) && !self.should_fail
    }
}

/// 流式通道扩展 trait，支持分块发送和打字状态指示
pub trait StreamingChannel: Channel {
    fn send_chunk(&self, target: &str, chunk: &str) -> SendResult;
    fn start_typing(&self, recipient: &str) -> SendResult;
    fn stop_typing(&self, recipient: &str) -> SendResult;
}

impl StreamingChannel for TestChannel {
    fn send_chunk(&self, target: &str, chunk: &str) -> SendResult {
        self.send_message(target, chunk, SendOptions::default())
    }

    fn start_typing(&self, _recipient: &str) -> SendResult {
        ok(None)
    }

    fn stop_typing(&self, _recipient: &str) -> SendResult {
        ok(None)
    }
}
