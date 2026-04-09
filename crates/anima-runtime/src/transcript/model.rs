use crate::messages::types::{InternalMsg, MessageRole};
use crate::streaming::types::{ContentBlock as StreamContentBlock, ContentDelta};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    Json {
        value: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageRecord {
    pub message_id: String,
    pub run_id: String,
    pub turn_id: Option<String>,
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    pub tool_use_id: Option<String>,
    pub metadata: Value,
    pub filtered: bool,
    pub appended_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptInvariantViolation {
    MissingToolResult,
    DuplicateToolResult,
}

impl MessageRecord {
    pub fn from_internal(
        run_id: String,
        turn_id: Option<String>,
        appended_at_ms: u64,
        msg: &InternalMsg,
    ) -> Self {
        Self {
            message_id: msg.message_id.clone(),
            run_id,
            turn_id,
            role: msg.role.clone(),
            blocks: blocks_from_value(&msg.content, msg.tool_use_id.as_deref()),
            tool_use_id: msg.tool_use_id.clone(),
            metadata: msg.metadata.clone(),
            filtered: msg.filtered,
            appended_at_ms,
        }
    }

    pub fn to_internal(&self) -> InternalMsg {
        InternalMsg {
            role: self.role.clone(),
            content: value_from_blocks(&self.blocks),
            message_id: self.message_id.clone(),
            tool_use_id: self.tool_use_id.clone(),
            filtered: self.filtered,
            metadata: self.metadata.clone(),
        }
    }
}

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

pub fn value_from_blocks(blocks: &[ContentBlock]) -> Value {
    Value::Array(
        blocks
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => serde_json::json!({"type": "text", "text": text}),
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
                ContentBlock::Json { value } => value.clone(),
            })
            .collect(),
    )
}

pub fn stream_block_to_transcript(block: &StreamContentBlock) -> ContentBlock {
    match block {
        StreamContentBlock::Text { text } => ContentBlock::Text { text: text.clone() },
        StreamContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
    }
}

pub fn apply_delta(block: &mut ContentBlock, delta: &ContentDelta) {
    match (block, delta) {
        (ContentBlock::Text { text }, ContentDelta::TextDelta { text: chunk }) => {
            text.push_str(chunk)
        }
        (ContentBlock::ToolUse { input, .. }, ContentDelta::InputJsonDelta { partial_json }) => {
            *input = Value::String(match input {
                Value::String(current) => format!("{current}{partial_json}"),
                _ => partial_json.clone(),
            });
        }
        _ => {}
    }
}
