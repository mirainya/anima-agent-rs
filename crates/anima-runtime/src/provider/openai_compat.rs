use std::io::BufRead;

use reqwest::blocking::Client;
use serde_json::{json, Value};

use super::error::{ProviderError, ProviderErrorKind};
use super::types::{ChatRequest, ChatResponse, ChatRole, StopReason, Usage};
use super::{ChatStream, Provider};
use crate::messages::types::ContentBlock;
use crate::streaming::types::{ContentBlock as StreamContentBlock, ContentDelta, StreamEvent};

const DEFAULT_BASE_URL: &str = "https://api.openai.com";

pub struct OpenAiCompatProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
}

impl OpenAiCompatProvider {
    pub fn new(api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            base_url: DEFAULT_BASE_URL.to_string(),
            max_tokens,
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    pub fn from_config(
        config: &anima_types::config::ProviderConfig,
    ) -> Result<Self, ProviderError> {
        let api_key = std::env::var(&config.api_key_env).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Authentication,
                format!("env var '{}' not set", config.api_key_env),
            )
        })?;
        let mut provider = Self::new(api_key, config.model.clone(), config.max_tokens);
        if let Some(ref url) = config.base_url {
            provider.base_url = url.clone();
        }
        Ok(provider)
    }

    fn build_body(&self, req: &ChatRequest) -> Value {
        let mut messages: Vec<Value> = Vec::new();

        if let Some(ref system) = req.system {
            messages.push(json!({"role": "system", "content": system}));
        }

        for m in &req.messages {
            let role_str = match m.role {
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
            };

            let mut tool_results: Vec<&ContentBlock> = Vec::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            let mut text_parts: Vec<String> = Vec::new();

            for block in &m.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": input.to_string(),
                            }
                        }));
                    }
                    ContentBlock::ToolResult { .. } => {
                        tool_results.push(block);
                    }
                    ContentBlock::Thinking { .. } | ContentBlock::Json { .. } => {}
                }
            }

            if !tool_calls.is_empty() {
                let mut msg = json!({"role": "assistant"});
                if !text_parts.is_empty() {
                    msg["content"] = json!(text_parts.join("\n"));
                }
                msg["tool_calls"] = json!(tool_calls);
                messages.push(msg);
            } else if !text_parts.is_empty() && tool_results.is_empty() {
                messages.push(json!({"role": role_str, "content": text_parts.join("\n")}));
            } else if text_parts.is_empty() && tool_results.is_empty() {
                messages.push(json!({"role": role_str, "content": ""}));
            }

            for block in tool_results {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    let content_str = match content {
                        Value::String(s) => s.clone(),
                        _ => content.to_string(),
                    };
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content_str,
                    }));
                }
            }
        }

        let mut body = json!({
            "model": req.model.as_deref().unwrap_or(&self.model),
            "messages": messages,
            "max_tokens": req.max_tokens.unwrap_or(self.max_tokens),
        });

        if let Some(ref tools) = req.tools {
            let openai_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| {
                    let name = t.get("name")?.as_str()?;
                    let desc = t.get("description").and_then(Value::as_str).unwrap_or("");
                    let schema = t
                        .get("input_schema")
                        .cloned()
                        .unwrap_or(json!({"type": "object"}));
                    Some(json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": desc,
                            "parameters": schema,
                        }
                    }))
                })
                .collect();
            if !openai_tools.is_empty() {
                body["tools"] = json!(openai_tools);
            }
        }
        if let Some(temp) = req.temperature {
            body["temperature"] = json!(temp);
        }
        body
    }

    fn send_request(&self, body: Value) -> Result<reqwest::blocking::Response, ProviderError> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::timeout(e.to_string())
                } else {
                    ProviderError::new(ProviderErrorKind::Network, e.to_string())
                }
            })
    }

    fn parse_response(&self, raw: &Value) -> Result<ChatResponse, ProviderError> {
        let choice = raw
            .pointer("/choices/0/message")
            .ok_or_else(|| ProviderError::internal("missing choices[0].message"))?;

        let mut blocks: Vec<ContentBlock> = Vec::new();

        if let Some(text) = choice.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                blocks.push(ContentBlock::Text {
                    text: text.to_string(),
                });
            }
        }

        if let Some(tool_calls) = choice.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let id = tc.get("id").and_then(Value::as_str).unwrap_or_default();
                let func = tc.get("function").unwrap_or(&Value::Null);
                let name = func.get("name").and_then(Value::as_str).unwrap_or_default();
                let args_str = func
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let input: Value =
                    serde_json::from_str(args_str).unwrap_or(Value::Object(Default::default()));
                blocks.push(ContentBlock::ToolUse {
                    id: id.to_string(),
                    name: name.to_string(),
                    input,
                });
            }
        }

        let finish_reason = raw
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("stop");

        let stop_reason = match finish_reason {
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            "stop" => StopReason::EndTurn,
            _ => StopReason::EndTurn,
        };

        let usage = raw.get("usage").and_then(|u| {
            Some(Usage {
                input_tokens: u.get("prompt_tokens")?.as_u64()? as u32,
                output_tokens: u.get("completion_tokens")?.as_u64()? as u32,
            })
        });

        Ok(ChatResponse {
            content: blocks,
            stop_reason,
            usage,
            raw: raw.clone(),
        })
    }
}

impl Provider for OpenAiCompatProvider {
    fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = self.build_body(&req);
        let resp = self.send_request(body)?;
        let status = resp.status();
        let raw: Value = resp
            .json()
            .map_err(|e| ProviderError::internal(e.to_string()))?;

        if !status.is_success() {
            let kind = match status.as_u16() {
                401 => ProviderErrorKind::Authentication,
                429 => ProviderErrorKind::RateLimit,
                400 => ProviderErrorKind::InvalidRequest,
                _ => ProviderErrorKind::Internal,
            };
            let msg = raw
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(ProviderError::new(kind, msg));
        }

        self.parse_response(&raw)
    }

    fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, ProviderError> {
        let mut body = self.build_body(&req);
        body["stream"] = json!(true);
        let resp = self.send_request(body)?;
        let status = resp.status();
        if !status.is_success() {
            let raw: Value = resp
                .json()
                .map_err(|e| ProviderError::internal(e.to_string()))?;
            let msg = raw
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("stream request failed");
            return Err(ProviderError::new(ProviderErrorKind::StreamFailed, msg));
        }

        let reader = std::io::BufReader::new(resp);
        let mut started = false;
        let mut text_block_open = false;
        // track tool call indices we've seen a start for
        let mut tool_block_indices: std::collections::HashSet<u64> =
            std::collections::HashSet::new();
        // next content block index to assign
        let mut next_block_idx: usize = 0;

        let iter = reader.lines().filter_map(move |line_result| {
            let line = line_result.ok()?;
            let data = line.strip_prefix("data: ")?;
            if data.trim().is_empty() || data.trim() == "[DONE]" {
                return None;
            }
            let json: Value = serde_json::from_str(data).ok()?;
            let choice = json.pointer("/choices/0")?;
            let delta = choice.get("delta")?;

            let finish_reason = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());

            let mut events: Vec<StreamEvent> = Vec::new();

            // message_start on first chunk
            if !started {
                started = true;
                let msg_id = json
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                events.push(StreamEvent::MessageStart { message_id: msg_id });
            }

            // text content delta
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    if !text_block_open {
                        text_block_open = true;
                        let idx = next_block_idx;
                        next_block_idx += 1;
                        events.push(StreamEvent::ContentBlockStart {
                            index: idx,
                            content_block: StreamContentBlock::Text {
                                text: String::new(),
                            },
                        });
                    }
                    events.push(StreamEvent::ContentBlockDelta {
                        index: if next_block_idx > 0 {
                            next_block_idx - 1
                        } else {
                            0
                        },
                        delta: ContentDelta::TextDelta {
                            text: text.to_string(),
                        },
                    });
                }
            }

            // tool_calls delta
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                // close text block before tool blocks
                if text_block_open {
                    text_block_open = false;
                    events.push(StreamEvent::ContentBlockStop {
                        index: next_block_idx - 1,
                    });
                }
                for tc in tool_calls {
                    let tc_index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                    if !tool_block_indices.contains(&tc_index) {
                        tool_block_indices.insert(tc_index);
                        let id = tc
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = tc
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let idx = next_block_idx;
                        next_block_idx += 1;
                        events.push(StreamEvent::ContentBlockStart {
                            index: idx,
                            content_block: StreamContentBlock::ToolUse {
                                id,
                                name,
                                input: Value::Object(Default::default()),
                            },
                        });
                    }
                    if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                        if !args.is_empty() {
                            // find the block index for this tool call
                            let block_idx = tool_block_indices
                                .iter()
                                .position(|&i| i == tc_index)
                                .map(|pos| {
                                    if text_block_open {
                                        pos + 1
                                    } else {
                                        let text_offset =
                                            if next_block_idx > tool_block_indices.len() {
                                                next_block_idx - tool_block_indices.len()
                                            } else {
                                                0
                                            };
                                        pos + text_offset
                                    }
                                })
                                .unwrap_or(next_block_idx - 1);
                            events.push(StreamEvent::ContentBlockDelta {
                                index: block_idx,
                                delta: ContentDelta::InputJsonDelta {
                                    partial_json: args.to_string(),
                                },
                            });
                        }
                    }
                }
            }

            // finish
            if let Some(reason) = finish_reason {
                // close any open blocks
                if text_block_open {
                    events.push(StreamEvent::ContentBlockStop {
                        index: next_block_idx - 1,
                    });
                }
                for i in 0..tool_block_indices.len() {
                    let text_offset = if next_block_idx > tool_block_indices.len() {
                        next_block_idx - tool_block_indices.len()
                    } else {
                        0
                    };
                    events.push(StreamEvent::ContentBlockStop {
                        index: i + text_offset,
                    });
                }
                let stop = match reason {
                    "tool_calls" => Some("tool_use".to_string()),
                    "length" => Some("max_tokens".to_string()),
                    "stop" => Some("end_turn".to_string()),
                    other => Some(other.to_string()),
                };
                events.push(StreamEvent::MessageDelta { stop_reason: stop });
                events.push(StreamEvent::MessageStop);
            }

            if events.is_empty() {
                None
            } else {
                Some(events)
            }
        });

        let flat = iter.flatten().map(Ok);
        Ok(Box::new(flat))
    }

    fn label(&self) -> &str {
        "openai_compat"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::types::ContentBlock;
    use crate::provider::types::{ChatMessage, ChatRequest, ChatRole};

    fn make_provider() -> OpenAiCompatProvider {
        OpenAiCompatProvider::new("test-key".into(), "gpt-4".into(), 4096)
    }

    #[test]
    fn build_body_simple_text() {
        let p = make_provider();
        let req = ChatRequest::from_text("hello");
        let body = p.build_body(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hello");
        assert_eq!(body["model"], "gpt-4");
    }

    #[test]
    fn build_body_with_system() {
        let p = make_provider();
        let mut req = ChatRequest::from_text("hi");
        req.system = Some("you are helpful".into());
        let body = p.build_body(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "you are helpful");
    }

    #[test]
    fn build_body_tool_use_and_result() {
        let p = make_provider();
        let req = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: ChatRole::User,
                    content: vec![ContentBlock::Text {
                        text: "run ls".into(),
                    }],
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: vec![ContentBlock::ToolUse {
                        id: "call_1".into(),
                        name: "bash".into(),
                        input: json!({"cmd": "ls"}),
                    }],
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: json!("file1\nfile2"),
                        is_error: false,
                    }],
                },
            ],
            system: None,
            tools: None,
            model: None,
            temperature: None,
            max_tokens: None,
            metadata: Value::Null,
        };
        let body = p.build_body(&req);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        // assistant with tool_calls
        assert!(msgs[1]["tool_calls"].is_array());
        let tc = &msgs[1]["tool_calls"][0];
        assert_eq!(tc["id"], "call_1");
        assert_eq!(tc["function"]["name"], "bash");
        // tool result
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "call_1");
    }

    #[test]
    fn build_body_converts_tools_to_openai_format() {
        let p = make_provider();
        let mut req = ChatRequest::from_text("hi");
        req.tools = Some(vec![json!({
            "name": "bash",
            "description": "run shell",
            "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}
        })]);
        let body = p.build_body(&req);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "bash");
        assert!(tools[0]["function"]["parameters"].is_object());
    }

    #[test]
    fn parse_response_text_only() {
        let p = make_provider();
        let raw = json!({
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2}
        });
        let resp = p.parse_response(&raw).unwrap();
        assert_eq!(resp.text(), "hi");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.usage.as_ref().unwrap().input_tokens, 10);
    }

    #[test]
    fn parse_response_with_tool_calls() {
        let p = make_provider();
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "bash", "arguments": "{\"cmd\":\"ls\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let resp = p.parse_response(&raw).unwrap();
        assert!(resp.has_tool_use());
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert!(matches!(&resp.content[0], ContentBlock::ToolUse { name, .. } if name == "bash"));
    }

    #[test]
    fn parse_response_max_tokens() {
        let p = make_provider();
        let raw = json!({
            "choices": [{"message": {"role": "assistant", "content": "partial"}, "finish_reason": "length"}]
        });
        let resp = p.parse_response(&raw).unwrap();
        assert_eq!(resp.stop_reason, StopReason::MaxTokens);
    }
}
