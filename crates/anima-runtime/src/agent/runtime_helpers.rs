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

    if let Some(text) = response.get("text").and_then(Value::as_str) {
        return text.to_string();
    }

    if let Some(content) = response.get("content").and_then(Value::as_str) {
        return content.to_string();
    }

    if let Some(content) = response.as_str() {
        return content.to_string();
    }

    response.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{make_inbound, MakeInbound};
    use serde_json::json;

    #[test]
    fn truncate_ascii_within_limit() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn truncate_ascii_over_limit() {
        assert_eq!(truncate_preview("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_unicode() {
        assert_eq!(truncate_preview("你好世界测试", 3), "你好世…");
    }

    #[test]
    fn truncate_exact_boundary() {
        assert_eq!(truncate_preview("abc", 3), "abc");
    }

    #[test]
    fn extract_response_none() {
        assert_eq!(extract_response_text(None), "");
    }

    #[test]
    fn extract_response_parts_text() {
        let resp = json!({"parts": [{"type": "text", "text": "hello"}]});
        assert_eq!(extract_response_text(Some(&resp)), "hello");
    }

    #[test]
    fn extract_response_parts_reasoning_and_text() {
        let resp = json!({"parts": [
            {"type": "reasoning", "text": "think"},
            {"type": "text", "text": "answer"}
        ]});
        let result = extract_response_text(Some(&resp));
        assert!(result.contains("【Reasoning】"));
        assert!(result.contains("think"));
        assert!(result.contains("answer"));
    }

    #[test]
    fn extract_response_data_messages() {
        let resp = json!({"data": {"messages": [
            {"parts": [{"type": "text", "text": "chunk1"}]},
            {"parts": [{"type": "text", "text": "chunk2"}]}
        ]}});
        assert_eq!(extract_response_text(Some(&resp)), "chunk1\nchunk2");
    }

    #[test]
    fn extract_response_content_string() {
        let resp = json!({"content": "direct"});
        assert_eq!(extract_response_text(Some(&resp)), "direct");
    }

    #[test]
    fn extract_response_plain_string() {
        let resp = json!("plain");
        assert_eq!(extract_response_text(Some(&resp)), "plain");
    }

    #[test]
    fn extract_response_fallback() {
        let resp = json!(42);
        assert_eq!(extract_response_text(Some(&resp)), "42");
    }

    #[test]
    fn merge_metadata_injects_keys() {
        let msg = make_inbound(MakeInbound {
            channel: "ch".into(),
            content: "hi".into(),
            metadata: Some(json!({"parent_job_id": "pj1", "subtask_id": "st1", "plan_id": "pl1"})),
            ..Default::default()
        });
        let result = merge_runtime_metadata(&msg, json!({"existing": true}));
        let obj = result.as_object().unwrap();
        assert_eq!(obj["existing"], json!(true));
        assert_eq!(obj["parent_job_id"], json!("pj1"));
        assert_eq!(obj["subtask_id"], json!("st1"));
        assert_eq!(obj["plan_id"], json!("pl1"));
    }

    #[test]
    fn merge_metadata_does_not_overwrite_existing() {
        let msg = make_inbound(MakeInbound {
            channel: "ch".into(),
            content: "hi".into(),
            metadata: Some(json!({"plan_id": "from_meta"})),
            ..Default::default()
        });
        let result = merge_runtime_metadata(&msg, json!({"plan_id": "from_payload"}));
        assert_eq!(result["plan_id"], json!("from_payload"));
    }

    #[test]
    fn merge_metadata_wraps_non_object() {
        let msg = make_inbound(MakeInbound {
            channel: "ch".into(),
            content: "hi".into(),
            ..Default::default()
        });
        let result = merge_runtime_metadata(&msg, json!("raw_string"));
        assert_eq!(result["value"], json!("raw_string"));
    }

    #[test]
    fn runtime_message_id_prefers_subtask() {
        let msg = make_inbound(MakeInbound {
            channel: "ch".into(),
            content: "hi".into(),
            metadata: Some(json!({"subtask_id": "sub1"})),
            ..Default::default()
        });
        assert_eq!(runtime_message_id(&msg), "sub1");
    }

    #[test]
    fn runtime_message_id_falls_back_to_id() {
        let msg = make_inbound(MakeInbound {
            channel: "ch".into(),
            content: "hi".into(),
            ..Default::default()
        });
        assert_eq!(runtime_message_id(&msg), msg.id);
    }
}
