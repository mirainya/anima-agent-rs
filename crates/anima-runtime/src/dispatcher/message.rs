use crate::bus::OutboundMessage;
use crate::support::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchMessage {
    pub id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: Option<String>,
    pub content: String,
    pub media: Vec<String>,
    pub stage: String,
    pub reply_target: Option<String>,
    pub sender_id: Option<String>,
    pub priority: u8,
    pub routing_key: Option<String>,
    pub session_key: Option<String>,
    pub metadata: Value,
    pub created_at: u64,
}

impl DispatchMessage {
    pub fn from_outbound(msg: &OutboundMessage) -> Self {
        let metadata = json!({});
        let session_key = msg
            .chat_id
            .as_ref()
            .map(|chat_id| format!("anima.session.{chat_id}"));
        Self {
            id: msg.id.clone(),
            channel: msg.channel.clone(),
            account_id: msg.account_id.clone(),
            chat_id: msg.chat_id.clone(),
            content: msg.content.clone(),
            media: msg.media.clone(),
            stage: msg.stage.clone(),
            reply_target: msg.reply_target.clone(),
            sender_id: msg.sender_id.clone(),
            priority: 5,
            routing_key: session_key.clone(),
            session_key,
            metadata,
            created_at: now_ms(),
        }
    }

    pub fn target(&self) -> String {
        self.reply_target
            .clone()
            .or_else(|| self.sender_id.clone())
            .unwrap_or_default()
    }

    pub fn selection_key(&self) -> Option<&str> {
        self.session_key
            .as_deref()
            .or(self.chat_id.as_deref())
            .or(self.sender_id.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatcherConfig {
    pub dispatch_timeout_ms: u64,
    pub receive_timeout_ms: u64,
    pub default_priority: u8,
    pub metadata: Value,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            dispatch_timeout_ms: 5_000,
            receive_timeout_ms: 25,
            default_priority: 5,
            metadata: json!({}),
        }
    }
}
