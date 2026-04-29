//! 消息三层类型定义
//!
//! - `InternalMsg`: 运行时内部消息（完整上下文）
//! - `ApiMsg`: 发送给 LLM API 的消息格式（wire format 中间层）
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

/// 内部消息内容块 — runtime 自有的类型化表示
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Value,
        is_error: bool,
    },
    Thinking {
        thinking: String,
    },
    Json {
        value: Value,
    },
}

/// API 层消息：发送给 LLM 的消息格式（保持 Value 用于 wire format 序列化）
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

/// 内部消息
#[derive(Debug, Clone, PartialEq)]
pub struct InternalMsg {
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    /// 消息唯一 ID
    pub message_id: String,
    /// 关联的 tool_use ID
    pub tool_use_id: Option<String>,
    /// 是否已被过滤
    pub filtered: bool,
    /// 附加元数据
    pub metadata: Value,
}

/// 将 blocks 序列化为 Anthropic wire format Value
pub fn value_from_blocks(blocks: &[ContentBlock]) -> Value {
    if blocks.len() == 1 {
        if let ContentBlock::Text { text } = &blocks[0] {
            return Value::String(text.clone());
        }
    }
    Value::Array(
        blocks
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => {
                    serde_json::json!({"type": "text", "text": text})
                }
                ContentBlock::ToolUse { id, name, input } => {
                    serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input})
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content,
                    "is_error": is_error,
                }),
                ContentBlock::Thinking { thinking } => {
                    serde_json::json!({"type": "thinking", "thinking": thinking})
                }
                ContentBlock::Json { value } => value.clone(),
            })
            .collect(),
    )
}

/// 从 Anthropic wire format Value 解析为 blocks
pub fn blocks_from_value(content: &Value, fallback_tool_use_id: Option<&str>) -> Vec<ContentBlock> {
    match content {
        Value::Array(items) => items
            .iter()
            .map(|item| match item.get("type").and_then(Value::as_str) {
                Some("text") => ContentBlock::Text {
                    text: item
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                Some("tool_use") => ContentBlock::ToolUse {
                    id: item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input: item.get("input").cloned().unwrap_or(Value::Null),
                },
                Some("tool_result") => ContentBlock::ToolResult {
                    tool_use_id: item
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .or(fallback_tool_use_id)
                        .unwrap_or_default()
                        .to_string(),
                    content: item.get("content").cloned().unwrap_or(Value::Null),
                    is_error: item
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
                Some("thinking") => ContentBlock::Thinking {
                    thinking: item
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                _ => ContentBlock::Json {
                    value: item.clone(),
                },
            })
            .collect(),
        Value::String(text) => vec![ContentBlock::Text { text: text.clone() }],
        other => vec![ContentBlock::Json {
            value: other.clone(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thinking_block_round_trip() {
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "let me think...".into(),
            },
            ContentBlock::Text {
                text: "answer".into(),
            },
        ];
        let val = value_from_blocks(&blocks);
        let restored = blocks_from_value(&val, None);
        assert_eq!(restored.len(), 2);
        assert!(
            matches!(&restored[0], ContentBlock::Thinking { thinking } if thinking == "let me think...")
        );
        assert!(matches!(&restored[1], ContentBlock::Text { text } if text == "answer"));
    }

    #[test]
    fn thinking_block_serializes_to_wire_format() {
        let blocks = vec![ContentBlock::Thinking {
            thinking: "hmm".into(),
        }];
        let val = value_from_blocks(&blocks);
        let arr = val.as_array().unwrap();
        assert_eq!(arr[0], json!({"type": "thinking", "thinking": "hmm"}));
    }
}
