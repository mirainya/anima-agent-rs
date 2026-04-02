//! 消息三层类型定义
//!
//! - `InternalMsg`: 运行时内部消息（完整上下文）
//! - `ApiMsg`: 发送给 LLM API 的消息格式
//! - `SdkMsg`: SDK 层面的消息封装

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 消息角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// API 层消息：发送给 LLM 的消息格式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiMsg {
    pub role: MessageRole,
    pub content: Value,
}

/// SDK 层消息：SDK 封装的消息格式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdkMsg {
    pub role: MessageRole,
    pub content: Value,
    /// 原始 tool_use 的 ID（如果这是 tool_result）
    pub tool_use_id: Option<String>,
}

/// 内部消息标注
#[derive(Debug, Clone, PartialEq)]
pub struct InternalMsg {
    pub role: MessageRole,
    pub content: Value,
    /// 消息唯一 ID
    pub message_id: String,
    /// 关联的 tool_use ID
    pub tool_use_id: Option<String>,
    /// 是否已被过滤
    pub filtered: bool,
    /// 附加元数据
    pub metadata: Value,
}
