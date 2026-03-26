use crate::support::now_ms;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMessage {
    pub id: String,
    pub session_id: String,
    pub sender: String,
    pub content: String,
    pub channel: String,
    pub timestamp: u64,
    pub reply_target: Option<String>,
    pub message_id: Option<String>,
    pub first_name: Option<String>,
    pub is_group: bool,
    pub account_id: Option<String>,
    pub metadata: Value,
}

pub fn create_message(input: CreateMessage) -> ChannelMessage {
    ChannelMessage {
        id: Uuid::new_v4().to_string(),
        session_id: input
            .session_id
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        sender: input.sender.unwrap_or_else(|| "unknown".into()),
        content: input.content,
        channel: input.channel.unwrap_or_else(|| "unknown".into()),
        timestamp: now_ms(),
        reply_target: input.reply_target,
        message_id: input.message_id,
        first_name: input.first_name,
        is_group: input.is_group.unwrap_or(false),
        account_id: input.account_id,
        metadata: input.metadata.unwrap_or_else(|| json!({})),
    }
}

#[derive(Debug, Default)]
pub struct CreateMessage {
    pub session_id: Option<String>,
    pub sender: Option<String>,
    pub content: String,
    pub channel: Option<String>,
    pub reply_target: Option<String>,
    pub message_id: Option<String>,
    pub first_name: Option<String>,
    pub is_group: Option<bool>,
    pub account_id: Option<String>,
    pub metadata: Option<Value>,
}

pub const BROADCAST_ROUTING_KEY: &str = "anima.broadcast";

pub fn extract_session_id(routing_key: &str) -> Option<String> {
    routing_key
        .strip_prefix("anima.session.")
        .map(ToString::to_string)
}

pub fn extract_user_id(routing_key: &str) -> Option<String> {
    routing_key
        .strip_prefix("anima.user.")
        .map(ToString::to_string)
}

pub fn make_session_routing_key(session_id: &str) -> String {
    format!("anima.session.{session_id}")
}

pub fn make_user_routing_key(user_id: &str) -> String {
    format!("anima.user.{user_id}")
}

pub fn make_channel_routing_key(channel_name: &str) -> String {
    format!("anima.channel.{channel_name}")
}

