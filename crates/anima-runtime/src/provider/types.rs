use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::messages::types::ContentBlock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub system: Option<String>,
    pub tools: Option<Vec<Value>>,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub metadata: Value,
}

impl ChatRequest {
    pub fn from_text(prompt: &str) -> Self {
        Self {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: vec![ContentBlock::Text {
                    text: prompt.to_string(),
                }],
            }],
            system: None,
            tools: None,
            model: None,
            temperature: None,
            max_tokens: None,
            metadata: Value::Null,
        }
    }
}

impl Default for ChatRequest {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            system: None,
            tools: None,
            model: None,
            temperature: None,
            max_tokens: None,
            metadata: Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Option<Usage>,
    pub raw: Value,
}

impl ChatResponse {
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn has_tool_use(&self) -> bool {
        self.content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    }
}
