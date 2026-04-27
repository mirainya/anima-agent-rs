use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;

use super::error::{ProviderError, ProviderErrorKind};
use super::types::{ChatRequest, ChatResponse, StopReason, Usage};
use super::{ChatStream, Provider};
use crate::messages::types::{blocks_from_value, value_from_blocks, ContentBlock};
use crate::worker::executor::{TaskExecutor, TaskExecutorError};

pub struct OpenCodeProvider {
    executor: Arc<dyn TaskExecutor>,
    session_id: Mutex<Option<String>>,
}

impl OpenCodeProvider {
    pub fn new(executor: Arc<dyn TaskExecutor>) -> Self {
        Self {
            executor,
            session_id: Mutex::new(None),
        }
    }

    pub fn with_session(executor: Arc<dyn TaskExecutor>, session_id: String) -> Self {
        Self {
            executor,
            session_id: Mutex::new(Some(session_id)),
        }
    }

    fn resolve_session_id(&self, req: &ChatRequest) -> Result<String, ProviderError> {
        if let Some(sid) = req.metadata.get("session_id").and_then(Value::as_str) {
            return Ok(sid.to_string());
        }
        {
            let guard = self.session_id.lock();
            if let Some(ref sid) = *guard {
                return Ok(sid.clone());
            }
        }
        let result = self.executor.create_session().map_err(map_executor_error)?;
        let sid = result
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::internal("create_session response missing 'id'"))?
            .to_string();
        *self.session_id.lock() = Some(sid.clone());
        Ok(sid)
    }
}

impl Provider for OpenCodeProvider {
    fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let session_id = self.resolve_session_id(&req)?;
        let content = chat_request_to_value(&req);
        let response = self
            .executor
            .send_prompt(&session_id, content)
            .map_err(map_executor_error)?;
        value_to_chat_response(response)
    }

    fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, ProviderError> {
        let session_id = self.resolve_session_id(&req)?;
        let mut content = chat_request_to_value(&req);
        content["stream"] = json!(true);
        let lines = self
            .executor
            .send_prompt_streaming(&session_id, content)
            .map_err(map_executor_error)?;
        Ok(Box::new(
            lines
                .filter_map(|r| match r {
                    Err(e) => Some(Err(map_executor_error(e))),
                    Ok(line) if line.trim().is_empty() => None,
                    Ok(line) => Some(parse_sse_to_stream_event(line)),
                }),
        ))
    }

    fn label(&self) -> &str {
        self.executor.provider_label()
    }

    fn create_session(&self) -> Result<Value, ProviderError> {
        self.executor.create_session().map_err(map_executor_error)
    }
}

fn chat_request_to_value(req: &ChatRequest) -> Value {
    if req.messages.is_empty() {
        return Value::Null;
    }

    let has_structured_messages = req.messages.len() > 1
        || req.system.is_some()
        || req.tools.is_some();

    if has_structured_messages {
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role,
                    "content": value_from_blocks(&m.content),
                })
            })
            .collect();
        let mut payload = json!({ "messages": messages });
        if let Some(ref sp) = req.system {
            payload["system"] = json!(sp);
        }
        if let Some(ref tools) = req.tools {
            payload["tools"] = json!(tools);
        }
        if let Some(ref model) = req.model {
            payload["model"] = json!(model);
        }
        if let Some(temp) = req.temperature {
            payload["temperature"] = json!(temp);
        }
        if let Some(max) = req.max_tokens {
            payload["max_tokens"] = json!(max);
        }
        payload
    } else {
        value_from_blocks(&req.messages[0].content)
    }
}

fn value_to_chat_response(raw: Value) -> Result<ChatResponse, ProviderError> {
    let content_value = raw.get("content").cloned().unwrap_or(Value::Null);
    let blocks = blocks_from_value(&content_value, None);

    let has_tool_use = blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));

    let stop_reason = if has_tool_use {
        StopReason::ToolUse
    } else {
        raw.get("stop_reason")
            .and_then(Value::as_str)
            .map(|s| match s {
                "max_tokens" => StopReason::MaxTokens,
                "stop_sequence" => StopReason::StopSequence,
                "tool_use" => StopReason::ToolUse,
                _ => StopReason::EndTurn,
            })
            .unwrap_or(StopReason::EndTurn)
    };

    let usage = raw.get("usage").and_then(|u| {
        Some(Usage {
            input_tokens: u.get("input_tokens")?.as_u64()? as u32,
            output_tokens: u.get("output_tokens")?.as_u64()? as u32,
        })
    });

    Ok(ChatResponse {
        content: blocks,
        stop_reason,
        usage,
        raw,
    })
}

fn parse_sse_to_stream_event(line: String) -> Result<crate::streaming::types::StreamEvent, ProviderError> {
    let data = line
        .strip_prefix("data: ")
        .unwrap_or(&line);
    crate::streaming::parse_sse_event(data)
        .ok_or_else(|| ProviderError::internal(format!("unrecognized stream event: {data}")))
}

fn map_executor_error(err: TaskExecutorError) -> ProviderError {
    let kind = match err.kind {
        crate::agent::runtime_error::RuntimeErrorKind::UpstreamTimeout => {
            ProviderErrorKind::Timeout
        }
        crate::agent::runtime_error::RuntimeErrorKind::UpstreamStreamFailed => {
            ProviderErrorKind::StreamFailed
        }
        crate::agent::runtime_error::RuntimeErrorKind::SessionCreateFailed => {
            ProviderErrorKind::Network
        }
        _ => ProviderErrorKind::Internal,
    };
    ProviderError::new(kind, err.internal_message)
}
