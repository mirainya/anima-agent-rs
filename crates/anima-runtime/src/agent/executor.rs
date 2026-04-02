use anima_sdk::{client as sdk_http, facade::Client as SdkClient, messages, sessions};
use serde_json::Value;
use std::io::{BufRead, BufReader};

pub trait TaskExecutor: Send + Sync {
    fn send_prompt(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String>;

    fn create_session(&self, client: &SdkClient) -> Result<Value, String>;

    /// 流式发送 prompt，返回 SSE 行迭代器
    ///
    /// 默认返回 Err（不支持流式）。实现方需要用 `BufReader::lines()` 包装响应。
    fn send_prompt_streaming(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Box<dyn Iterator<Item = Result<String, String>>>, String> {
        Err("streaming not supported".into())
    }
}

#[derive(Debug, Default)]
pub struct SdkTaskExecutor;

impl TaskExecutor for SdkTaskExecutor {
    fn send_prompt(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        messages::send_prompt(client, session_id, content, None).map_err(|err| err.to_string())
    }

    fn create_session(&self, client: &SdkClient) -> Result<Value, String> {
        sessions::create_session(client, None).map_err(|err| err.to_string())
    }

    fn send_prompt_streaming(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Box<dyn Iterator<Item = Result<String, String>>>, String> {
        let response = sdk_http::post_request_streaming(
            client,
            &format!("/session/{session_id}/message"),
            &content,
        )
        .map_err(|e| e.to_string())?;

        let reader = BufReader::new(response);
        let lines = reader.lines().map(|r| r.map_err(|e| e.to_string()));
        Ok(Box::new(lines))
    }
}
