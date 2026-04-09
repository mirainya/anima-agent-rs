use serde_json::{Map, Value};

use crate::bus::InboundMessage;
pub(crate) fn truncate_preview(text: &str, max_chars: usize) -> String {
    let truncated = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        format!("{}…", truncated)
    } else {
        truncated
    }
}

pub(crate) fn infer_provider(response: Option<&Value>) -> Option<String> {
    response
        .and_then(|value| value.get("provider").and_then(Value::as_str))
        .map(ToString::to_string)
        .or_else(|| Some("opencode".to_string()))
}

pub(crate) fn infer_operation(response: Option<&Value>) -> Option<String> {
    response
        .and_then(|value| value.get("operation").and_then(Value::as_str))
        .map(ToString::to_string)
        .or_else(|| {
            response
                .and_then(|value| value.get("type").and_then(Value::as_str))
                .map(ToString::to_string)
        })
        .or_else(|| Some("send_prompt".to_string()))
}

pub(crate) fn subtask_job_id(inbound_msg: &InboundMessage) -> Option<String> {
    inbound_msg
        .metadata
        .get("subtask_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

pub(crate) fn runtime_message_id(inbound_msg: &InboundMessage) -> String {
    subtask_job_id(inbound_msg).unwrap_or_else(|| inbound_msg.id.clone())
}

pub(crate) fn merge_runtime_metadata(inbound_msg: &InboundMessage, payload: Value) -> Value {
    let mut merged = match payload {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".into(), other);
            map
        }
    };

    for key in ["parent_job_id", "subtask_id", "plan_id"] {
        if let Some(value) = inbound_msg.metadata.get(key).cloned() {
            merged.entry(key.to_string()).or_insert(value);
        }
    }

    Value::Object(merged)
}

pub(crate) fn extract_response_text(response: Option<&Value>) -> String {
    let Some(response) = response else {
        return String::new();
    };

    if let Some(parts) = response.get("parts").and_then(Value::as_array) {
        let mut reasoning = Vec::new();
        let mut text = Vec::new();
        for part in parts {
            match part.get("type").and_then(Value::as_str) {
                Some("reasoning") => {
                    if let Some(content) = part.get("text").and_then(Value::as_str) {
                        reasoning.push(content.to_string());
                    }
                }
                Some("text") => {
                    if let Some(content) = part.get("text").and_then(Value::as_str) {
                        text.push(content.to_string());
                    }
                }
                _ => {}
            }
        }
        if !reasoning.is_empty() {
            return format!(
                "【Reasoning】\n{}\n【End Reasoning】\n\n{}",
                reasoning.join("\n"),
                text.join("\n")
            );
        }
        return text.join("\n");
    }

    if let Some(messages) = response
        .get("data")
        .and_then(|data| data.get("messages"))
        .and_then(Value::as_array)
    {
        let mut chunks = Vec::new();
        for message in messages {
            if let Some(parts) = message.get("parts").and_then(Value::as_array) {
                for part in parts {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("text") | Some("reasoning")
                    ) {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            chunks.push(text.to_string());
                        }
                    }
                }
            }
        }
        return chunks.join("\n");
    }

    if let Some(content) = response.get("content").and_then(Value::as_str) {
        return content.to_string();
    }

    if let Some(content) = response.as_str() {
        return content.to_string();
    }

    response.to_string()
}
