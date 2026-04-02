//! SSE 流式 API 响应解析器
//!
//! 将原始 SSE 文本行解析为 `StreamEvent` 序列。

use serde_json::Value;

use super::types::{ContentBlock, ContentDelta, StreamEvent};

/// 解析单行 SSE data 字段为 StreamEvent
///
/// 输入格式：`data: { "type": "content_block_start", ... }`
pub fn parse_sse_event(data: &str) -> Option<StreamEvent> {
    let json: Value = serde_json::from_str(data).ok()?;
    let event_type = json.get("type")?.as_str()?;

    match event_type {
        "message_start" => {
            let message_id = json
                .pointer("/message/id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(StreamEvent::MessageStart { message_id })
        }
        "content_block_start" => {
            let index = json.get("index")?.as_u64()? as usize;
            let cb = json.get("content_block")?;
            let block = parse_content_block(cb)?;
            Some(StreamEvent::ContentBlockStart {
                index,
                content_block: block,
            })
        }
        "content_block_delta" => {
            let index = json.get("index")?.as_u64()? as usize;
            let delta_obj = json.get("delta")?;
            let delta = parse_content_delta(delta_obj)?;
            Some(StreamEvent::ContentBlockDelta { index, delta })
        }
        "content_block_stop" => {
            let index = json.get("index")?.as_u64()? as usize;
            Some(StreamEvent::ContentBlockStop { index })
        }
        "message_delta" => {
            let stop_reason = json
                .pointer("/delta/stop_reason")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(StreamEvent::MessageDelta { stop_reason })
        }
        "message_stop" => Some(StreamEvent::MessageStop),
        "ping" => Some(StreamEvent::Ping),
        "error" => {
            let error_type = json
                .pointer("/error/type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let message = json
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(StreamEvent::Error {
                error_type,
                message,
            })
        }
        _ => None,
    }
}

fn parse_content_block(val: &Value) -> Option<ContentBlock> {
    let block_type = val.get("type")?.as_str()?;
    match block_type {
        "text" => {
            let text = val.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(ContentBlock::Text { text })
        }
        "tool_use" => {
            let id = val.get("id")?.as_str()?.to_string();
            let name = val.get("name")?.as_str()?.to_string();
            let input = val.get("input").cloned().unwrap_or(Value::Object(Default::default()));
            Some(ContentBlock::ToolUse { id, name, input })
        }
        _ => None,
    }
}

fn parse_content_delta(val: &Value) -> Option<ContentDelta> {
    let delta_type = val.get("type")?.as_str()?;
    match delta_type {
        "text_delta" => {
            let text = val.get("text")?.as_str()?.to_string();
            Some(ContentDelta::TextDelta { text })
        }
        "input_json_delta" => {
            let partial = val.get("partial_json")?.as_str()?.to_string();
            Some(ContentDelta::InputJsonDelta {
                partial_json: partial,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_start() {
        let data = r#"{"type":"message_start","message":{"id":"msg_123"}}"#;
        let event = parse_sse_event(data).unwrap();
        assert!(matches!(event, StreamEvent::MessageStart { message_id } if message_id == "msg_123"));
    }

    #[test]
    fn parse_text_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#;
        let event = parse_sse_event(data).unwrap();
        assert!(matches!(event, StreamEvent::ContentBlockDelta { index: 0, delta: ContentDelta::TextDelta { text } } if text == "hello"));
    }

    #[test]
    fn parse_tool_use_block() {
        let data = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"bash","input":{}}}"#;
        let event = parse_sse_event(data).unwrap();
        assert!(matches!(event, StreamEvent::ContentBlockStart { index: 1, content_block: ContentBlock::ToolUse { .. } }));
    }

    #[test]
    fn parse_message_stop() {
        let data = r#"{"type":"message_stop"}"#;
        assert!(matches!(parse_sse_event(data), Some(StreamEvent::MessageStop)));
    }

    #[test]
    fn parse_unknown_returns_none() {
        let data = r#"{"type":"unknown_event"}"#;
        assert!(parse_sse_event(data).is_none());
    }
}
