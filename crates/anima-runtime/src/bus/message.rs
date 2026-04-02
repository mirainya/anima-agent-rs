//! 消息类型定义模块
//!
//! 消息数据结构已下沉到 `anima-types::message`，此处 re-export 并提供 Builder 函数。

use serde_json::json;
use uuid::Uuid;
use crate::support::now_ms;

// Re-export 所有消息数据结构
pub use anima_types::message::*;

/// 构建入站消息的参数
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

/// 构建出站消息的参数
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

/// 控制消息的构建参数
#[derive(Debug)]
pub struct MakeControl {
    pub signal: ControlSignal,
    pub source: String,
    pub payload: Option<serde_json::Value>,
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
