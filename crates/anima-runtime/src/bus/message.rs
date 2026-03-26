//! 消息类型定义模块
//!
//! 定义了消息总线中流转的所有消息类型：
//! - InboundMessage / OutboundMessage：外部通信消息（入站/出站）
//! - InternalMessage：组件间内部通信消息，支持请求/响应/事件三种模式
//! - ControlMessage：系统控制信号（关闭、暂停、扩缩容等）
//!
//! 每种消息都配有对应的 Builder 结构体（MakeXxx）和工厂函数（make_xxx），
//! 自动填充 ID、时间戳等默认值。

use serde_json::json;
use uuid::Uuid;
use crate::support::now_ms;

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
    pub metadata: serde_json::Value,
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

/// 构建入站消息，自动生成 UUID 并填充默认值
pub fn make_inbound(input: MakeInbound) -> InboundMessage {
    InboundMessage {
        id: Uuid::new_v4().to_string(),
        channel: input.channel,
        sender_id: input.sender_id.unwrap_or_else(|| "unknown".into()),
        chat_id: input.chat_id,
        content: input.content,
        session_key: input.session_key,
        media: input.media.unwrap_or_default(),
        metadata: input.metadata.unwrap_or_else(|| json!({})),
    }
}

/// 构建出站消息，自动生成 UUID 并填充默认值
pub fn make_outbound(input: MakeOutbound) -> OutboundMessage {
    OutboundMessage {
        id: Uuid::new_v4().to_string(),
        channel: input.channel,
        account_id: input.account_id.unwrap_or_else(|| "default".into()),
        chat_id: input.chat_id,
        content: input.content,
        media: input.media.unwrap_or_default(),
        stage: input.stage.unwrap_or_else(|| "final".into()),
        reply_target: input.reply_target,
        sender_id: input.sender_id,
    }
}

/// 入站消息的构建参数
#[derive(Debug, Default)]
pub struct MakeInbound {
    pub channel: String,
    pub sender_id: Option<String>,
    pub chat_id: Option<String>,
    pub content: String,
    pub session_key: Option<String>,
    pub media: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

/// 出站消息的构建参数
#[derive(Debug, Default)]
pub struct MakeOutbound {
    pub channel: String,
    pub account_id: Option<String>,
    pub chat_id: Option<String>,
    pub content: String,
    pub media: Option<Vec<String>>,
    pub stage: Option<String>,
    pub reply_target: Option<String>,
    pub sender_id: Option<String>,
}

// ── 内部总线消息 ──────────────────────────────────────────────────

/// 内部消息类型：区分请求/响应/事件三种通信模式
#[derive(Debug, Clone, PartialEq)]
#[derive(Default)]
pub enum InternalMessageType {
    Request,
    Response,
    #[default]
    Event,
}


/// 内部消息：组件间通信的载体
///
/// 支持 trace_id 进行分布式追踪，priority 控制处理优先级，
/// ttl 防止过期消息堆积。
#[derive(Debug, Clone, PartialEq)]
pub struct InternalMessage {
    pub id: String,
    pub trace_id: String,
    pub source: String,
    pub target: Option<String>,
    pub msg_type: InternalMessageType,
    pub priority: u8,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    pub timestamp: u64,
    pub ttl: u64,
}

/// 内部消息的构建参数
#[derive(Debug, Default)]
pub struct MakeInternal {
    pub source: String,
    pub target: Option<String>,
    pub msg_type: Option<InternalMessageType>,
    pub priority: Option<u8>,
    pub payload: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub ttl: Option<u64>,
    pub trace_id: Option<String>,
}

/// 构建内部消息，自动生成 ID/trace_id/时间戳，默认优先级 5，默认 TTL 30 秒
pub fn make_internal(input: MakeInternal) -> InternalMessage {
    InternalMessage {
        id: Uuid::new_v4().to_string(),
        trace_id: input
            .trace_id
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        source: input.source,
        target: input.target,
        msg_type: input.msg_type.unwrap_or_default(),
        priority: input.priority.unwrap_or(5),
        payload: input.payload,
        metadata: input.metadata.unwrap_or_else(|| json!({})),
        timestamp: now_ms(),
        ttl: input.ttl.unwrap_or(30_000),
    }
}

// ── 控制总线消息 ──────────────────────────────────────────────────

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
    pub payload: serde_json::Value,
    pub timestamp: u64,
}

/// 控制消息的构建参数
#[derive(Debug)]
pub struct MakeControl {
    pub signal: ControlSignal,
    pub source: String,
    pub payload: Option<serde_json::Value>,
}

/// 构建控制消息
pub fn make_control(input: MakeControl) -> ControlMessage {
    ControlMessage {
        id: Uuid::new_v4().to_string(),
        signal: input.signal,
        source: input.source,
        payload: input.payload.unwrap_or_else(|| json!({})),
        timestamp: now_ms(),
    }
}
