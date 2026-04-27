//! OpenCodeProvider round-trip 测试
//! 验证 ChatRequest → executor.send_prompt → ChatResponse 的完整链路

use anima_runtime::messages::types::ContentBlock;
use anima_runtime::provider::opencode::OpenCodeProvider;
use anima_runtime::provider::{ChatMessage, ChatRequest, ChatRole, Provider};
use anima_runtime::worker::executor::TaskExecutor;
use serde_json::{json, Value};
use std::sync::Arc;

/// Mock executor：返回 content 数组格式（模拟真实 LLM 响应）
struct MockLlmExecutor;

impl TaskExecutor for MockLlmExecutor {
    fn send_prompt(
        &self,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        // value_from_blocks 单文本时返回 String，多 block 时返回 Array
        let text = content
            .as_str()
            .map(String::from)
            .or_else(|| {
                content
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|b| b.get("text"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .unwrap_or_default();

        Ok(json!({
            "content": [{"type": "text", "text": format!("[{session_id}] reply to: {text}")}]
        }))
    }

    fn create_session(
        &self,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "mock-session-1"}))
    }
}

#[test]
fn chat_round_trip_with_session_in_metadata() {
    let provider: Arc<dyn Provider> =
        Arc::new(OpenCodeProvider::new(Arc::new(MockLlmExecutor)));

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ContentBlock::Text {
                text: "hello world".into(),
            }],
        }],
        metadata: json!({ "session_id": "sess-42" }),
        ..Default::default()
    };

    let resp = provider.chat(req).expect("chat should succeed");

    // 验证返回了 Text block
    let text = resp.text();
    assert!(
        text.contains("[sess-42]"),
        "response should contain session id, got: {text}"
    );
    assert!(
        text.contains("hello world"),
        "response should echo request text, got: {text}"
    );
}

#[test]
fn chat_round_trip_auto_creates_session() {
    let provider: Arc<dyn Provider> =
        Arc::new(OpenCodeProvider::new(Arc::new(MockLlmExecutor)));

    // 不传 session_id → 自动 create_session
    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ContentBlock::Text {
                text: "任务分解引擎".into(),
            }],
        }],
        ..Default::default()
    };

    let resp = provider.chat(req).expect("chat should succeed");
    let text = resp.text();
    assert!(
        text.contains("[mock-session-1]"),
        "should use auto-created session id, got: {text}"
    );
}

#[test]
fn chat_round_trip_with_preset_session() {
    let provider: Arc<dyn Provider> = Arc::new(OpenCodeProvider::with_session(
        Arc::new(MockLlmExecutor),
        "preset-sess".into(),
    ));

    let req = ChatRequest::from_text("test prompt");

    let resp = provider.chat(req).expect("chat should succeed");
    let text = resp.text();
    assert!(
        text.contains("[preset-sess]"),
        "should use preset session id, got: {text}"
    );
}

#[test]
fn chat_response_has_correct_content_blocks() {
    let provider: Arc<dyn Provider> =
        Arc::new(OpenCodeProvider::new(Arc::new(MockLlmExecutor)));

    let req = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ContentBlock::Text {
                text: "ping".into(),
            }],
        }],
        metadata: json!({ "session_id": "s1" }),
        ..Default::default()
    };

    let resp = provider.chat(req).expect("chat should succeed");

    assert_eq!(resp.content.len(), 1);
    match &resp.content[0] {
        ContentBlock::Text { text } => {
            assert!(text.contains("ping"));
        }
        other => panic!("expected Text block, got: {other:?}"),
    }
    assert!(!resp.has_tool_use());
}
