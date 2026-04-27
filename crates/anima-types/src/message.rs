//! 消息类型定义
//!
//! 定义入站/出站消息的纯数据结构。
//! Builder 函数（make_inbound, make_outbound 等）保留在 anima-runtime 的 bus 模块中。

use serde_json::Value;

/// 入站消息：从外部渠道进入系统的用户消息
#[derive(Debug, Clone, PartialEq)]
pub struct InboundMessage {
    pub id: String,
    pub channel: String,
    pub sender_id: String,
    pub chat_id: Option<String>,
    pub content: String,
    pub session_key: Option<String>,
    pub media: Vec<String>,
    pub metadata: Value,
}

/// 出站消息：系统向外部渠道发送的响应消息
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundMessage {
    pub id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: Option<String>,
    pub content: String,
    pub media: Vec<String>,
    pub stage: String,
    pub reply_target: Option<String>,
    pub sender_id: Option<String>,
}

/// 内部消息类型：区分请求/响应/事件三种通信模式
#[derive(Debug, Clone, PartialEq, Default)]
pub enum InternalMessageType {
    Request,
    Response,
    #[default]
    Event,
}

/// 内部消息的类型化载荷
#[derive(Debug, Clone, PartialEq)]
pub enum InternalPayload {
    RuntimeEvent {
        event: String,
        message_id: String,
        channel: String,
        chat_id: Option<String>,
        sender_id: String,
        payload: Value,
    },
    WorkerStatus {
        event: String,
        worker_id: String,
        status: String,
        task_type: String,
        channel: String,
        message_id: String,
        chat_id: Option<String>,
    },
    Raw(Value),
}

impl Default for InternalPayload {
    fn default() -> Self {
        Self::Raw(Value::Object(Default::default()))
    }
}

/// 内部消息：组件间通信的载体
#[derive(Debug, Clone, PartialEq)]
pub struct InternalMessage {
    pub id: String,
    pub trace_id: String,
    pub source: String,
    pub target: Option<String>,
    pub msg_type: InternalMessageType,
    pub priority: u8,
    pub payload: InternalPayload,
    pub metadata: Value,
    pub timestamp: u64,
    pub ttl: u64,
}

/// 控制信号类型：用于系统生命周期管理
#[derive(Debug, Clone, PartialEq)]
pub enum ControlSignal {
    Shutdown,
    Pause,
    Resume,
    ScaleUp,
    ScaleDown,
    HealthCheck,
    ConfigReload,
}

/// 控制消息：携带控制信号和可选的附加数据
#[derive(Debug, Clone, PartialEq)]
pub struct ControlMessage {
    pub id: String,
    pub signal: ControlSignal,
    pub source: String,
    pub payload: Value,
    pub timestamp: u64,
}
