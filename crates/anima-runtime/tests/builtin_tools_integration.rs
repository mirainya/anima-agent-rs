//! 内置工具集成测试
//!
//! 验证 register_all 注册、以及内置工具与 agentic loop 的端到端集成。

use anima_runtime::execution::agentic_loop::{run_agentic_loop, AgenticLoopConfig};
use anima_runtime::agent::TaskExecutor;
use anima_runtime::messages::types::{InternalMsg, MessageRole};
use anima_runtime::tools::builtins::register_all;
use anima_runtime::tools::registry::ToolRegistry;

use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Mock Executor
// ---------------------------------------------------------------------------

struct SequenceExecutor {
    responses: Vec<Value>,
    call_count: AtomicUsize,
}

impl SequenceExecutor {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
        }
    }
}

impl TaskExecutor for SequenceExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(idx)
            .cloned()
            .ok_or_else(|| "no more mock responses".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session"}))
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[test]
fn test_register_all_populates_registry() {
    let mut registry = ToolRegistry::new();
    register_all(&mut registry);

    assert_eq!(registry.len(), 6);

    let names = registry.list_names();
    for expected in &[
        "bash_exec",
        "file_read",
        "file_write",
        "file_edit",
        "glob_search",
        "grep_search",
    ] {
        assert!(
            names.contains(expected),
            "missing tool: {expected}"
        );
    }

    // 验证每个工具都能生成 tool_definitions
    let defs = registry.tool_definitions();
    assert_eq!(defs.len(), 6);
    for def in &defs {
        assert!(def.get("name").is_some());
        assert!(def.get("description").is_some());
        assert!(def.get("input_schema").is_some());
    }
}

#[test]
fn test_agentic_loop_with_bash_tool() {
    // 模拟：模型第一轮请求 bash_exec 工具，第二轮返回文本结束
    let responses = vec![
        // 第一轮：模型请求 bash_exec
        json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "tu_1",
                    "name": "bash_exec",
                    "input": {
                        "command": "echo integration_test_ok"
                    }
                }
            ],
            "model": "test",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 10}
        }),
        // 第二轮：模型返回最终文本
        json!({
            "id": "msg_2",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "done"
                }
            ],
            "model": "test",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }),
    ];

    let executor = SequenceExecutor::new(responses);
    let client = SdkClient::new("test-key");

    let mut registry = ToolRegistry::new();
    register_all(&mut registry);

    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "test-session".into(),
        trace_id: "test-trace".into(),
        system_prompt: Some("You are a test agent.".into()),
        ..Default::default()
    };

    let initial = vec![InternalMsg {
        role: MessageRole::User,
        content: json!([{"type": "text", "text": "run echo"}]),
        message_id: "msg_init".into(),
        tool_use_id: None,
        filtered: false,
        metadata: json!({}),
    }];

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None, // no permission checker
        None, // no hook registry
        initial,
        &config,
    );

    assert!(result.is_ok(), "agentic loop failed: {:?}", result.err());
    let loop_result = result.unwrap();
    // 应该有 2 轮（工具调用 + 最终回复）
    assert_eq!(loop_result.iterations, 2);
    // 消息历史中应包含工具结果
    let has_tool_result = loop_result.messages.iter().any(|msg| {
        let content_str = msg.content.to_string();
        content_str.contains("integration_test_ok")
    });
    assert!(
        has_tool_result,
        "tool result should be in message history"
    );
}
