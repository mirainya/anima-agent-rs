use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct SendResult {
    pub success: bool,
    pub error: Option<String>,
    pub data: Option<Value>,
}

pub fn success(result: &SendResult) -> bool {
    result.success
}

pub fn error(result: &SendResult) -> bool {
    !result.success
}

pub fn ok(data: Option<Value>) -> SendResult {
    SendResult {
        success: true,
        error: None,
        data,
    }
}

pub fn err(message: impl Into<String>, data: Option<Value>) -> SendResult {
    SendResult {
        success: false,
        error: Some(message.into()),
        data,
    }
}

pub trait Channel: Send + Sync {
    fn start(&self);
    fn stop(&self);
    fn send_message(&self, target: &str, message: &str, opts: SendOptions) -> SendResult;
    fn channel_name(&self) -> &str;
    fn health_check(&self) -> bool;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendOptions {
    pub media: Vec<String>,
    pub stage: Option<String>,
}

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

    pub fn failing(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            running: AtomicBool::new(false),
            should_fail: true,
            sent_messages: Mutex::new(Vec::new()),
        }
    }

    pub fn sent_messages(&self) -> Vec<SentMessage> {
        self.sent_messages.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

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
        self.sent_messages.lock().unwrap().push(SentMessage {
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
