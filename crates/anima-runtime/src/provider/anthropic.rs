use std::io::BufRead;

use reqwest::blocking::Client;
use serde_json::{json, Value};

use super::error::{ProviderError, ProviderErrorKind};
use super::types::{ChatRequest, ChatResponse, ChatRole, StopReason, Usage};
use super::{ChatStream, Provider};
use crate::messages::types::{blocks_from_value, value_from_blocks, ContentBlock};
use crate::streaming::parse_sse_event;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
}

impl AnthropicProvider {
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

    pub fn from_config(config: &anima_types::config::ProviderConfig) -> Result<Self, ProviderError> {
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
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": match m.role { ChatRole::User => "user", ChatRole::Assistant => "assistant" },
                    "content": value_from_blocks(&m.content),
                })
            })
            .collect();

        let mut body = json!({
            "model": req.model.as_deref().unwrap_or(&self.model),
            "messages": messages,
            "max_tokens": req.max_tokens.unwrap_or(self.max_tokens),
        });

        if let Some(ref system) = req.system {
            body["system"] = json!(system);
        }
        if let Some(ref tools) = req.tools {
            body["tools"] = json!(tools);
        }
        if let Some(temp) = req.temperature {
            body["temperature"] = json!(temp);
        }
        body
    }

    fn send_request(&self, body: Value) -> Result<reqwest::blocking::Response, ProviderError> {
        let url = format!("{}/v1/messages", self.base_url);
        self.client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
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
}

impl Provider for AnthropicProvider {
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

        let content_value = raw.get("content").cloned().unwrap_or(Value::Null);
        let blocks = blocks_from_value(&content_value, None);
        let has_tool_use = blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));

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
        let iter = reader.lines().filter_map(|line_result| {
            let line = line_result.ok()?;
            let data = line.strip_prefix("data: ")?;
            if data.trim().is_empty() || data == "[DONE]" {
                return None;
            }
            Some(
                parse_sse_event(data)
                    .ok_or_else(|| ProviderError::internal(format!("unrecognized event: {data}"))),
            )
        });

        Ok(Box::new(iter))
    }

    fn label(&self) -> &str {
        "anthropic"
    }
}
