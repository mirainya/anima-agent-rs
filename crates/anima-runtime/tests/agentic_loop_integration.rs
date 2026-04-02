//! Agentic Loop 集成测试
//!
//! 验证 agentic loop 与 ToolRegistry、MockExecutor、EchoTool 的端到端集成。

use anima_runtime::execution::agentic_loop::{
    run_agentic_loop, AgenticLoopConfig,
};
use anima_runtime::agent::TaskExecutor;
use anima_runtime::messages::types::{InternalMsg, MessageRole};
use anima_runtime::tools::definition::{Tool, ToolContext};
use anima_runtime::tools::registry::ToolRegistry;
use anima_runtime::tools::result::{ToolError, ToolResult};

use anima_sdk::facade::Client as SdkClient;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock Executor：按预设序列返回 API 响应
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
        Ok(json!({"id": "integration-session"}))
    }
}

// ---------------------------------------------------------------------------
// EchoTool：返回输入内容
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {"text": {"type": "string"}}})
    }

    fn validate_input(&self, _input: &Value) -> Result<(), String> {
        Ok(())
    }

    fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let text = input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("(empty)");
        Ok(ToolResult::text(format!("echoed: {text}")))
    }
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

fn make_client() -> SdkClient {
    SdkClient::new("http://localhost:0")
}

fn make_user_msg(text: &str) -> InternalMsg {
    InternalMsg {
        role: MessageRole::User,
        content: json!(text),
        message_id: "user-1".into(),
        tool_use_id: None,
        filtered: false,
        metadata: json!({}),
    }
}

// ---------------------------------------------------------------------------
// 集成测试
// ---------------------------------------------------------------------------

/// EchoTool 注册 → mock executor 第一次返回 tool_use，第二次返回纯文本 → 验证循环正确
#[test]
fn agentic_loop_echo_tool_integration() {
    let executor = SequenceExecutor::new(vec![
        // 第一次：模型请求调用 echo 工具
        json!({
            "content": [
                {"type": "text", "text": "I'll echo that for you."},
                {"type": "tool_use", "id": "tu_echo_1", "name": "echo", "input": {"text": "integration test"}}
            ]
        }),
        // 第二次：模型返回最终文本
        json!({
            "content": [{"type": "text", "text": "Done! The echo returned: integration test"}]
        }),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "integration-session".into(),
        trace_id: "integration-trace".into(),
        compact: None,
        system_prompt: None,
        tool_definitions: None,
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        None,
        vec![make_user_msg("echo 'integration test'")],
        &config,
    )
    .expect("agentic loop should succeed");

    assert_eq!(
        result.final_text,
        "Done! The echo returned: integration test"
    );
    assert_eq!(result.iterations, 2);
    assert!(!result.hit_limit);

    // 验证消息历史：user → assistant(tool_use) → user(tool_result) → assistant(final)
    assert_eq!(result.messages.len(), 4);
    assert_eq!(result.messages[0].role, MessageRole::User);
    assert_eq!(result.messages[1].role, MessageRole::Assistant);
    assert_eq!(result.messages[2].role, MessageRole::User); // tool_result
    assert_eq!(result.messages[3].role, MessageRole::Assistant);
}

/// 工具未注册 → tool_result 中返回 error 但循环继续
#[test]
fn agentic_loop_unknown_tool_returns_error_result() {
    let executor = SequenceExecutor::new(vec![
        // 模型请求调用不存在的工具
        json!({
            "content": [
                {"type": "text", "text": "Let me try this tool"},
                {"type": "tool_use", "id": "tu_1", "name": "nonexistent", "input": {}}
            ]
        }),
        // 模型收到错误结果后给出最终回答
        json!({
            "content": [{"type": "text", "text": "That tool was not available."}]
        }),
    ]);

    // 空 registry，不注册任何工具
    let registry = ToolRegistry::new();
    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: None,
        system_prompt: None,
        tool_definitions: None,
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        None,
        vec![make_user_msg("use nonexistent tool")],
        &config,
    )
    .expect("loop should not fail for unknown tools");

    assert_eq!(result.final_text, "That tool was not available.");
    assert_eq!(result.iterations, 2);

    // 验证 tool_result 消息包含 is_error
    let tool_result_msg = &result.messages[2];
    let blocks = tool_result_msg.content.as_array().expect("should be array");
    assert_eq!(blocks[0]["is_error"], true);
}

/// 多工具同轮调用
#[test]
fn agentic_loop_multiple_tools_in_one_turn() {
    let executor = SequenceExecutor::new(vec![
        // 模型同时请求两个 echo 工具
        json!({
            "content": [
                {"type": "text", "text": "Let me echo two things"},
                {"type": "tool_use", "id": "tu_1", "name": "echo", "input": {"text": "first"}},
                {"type": "tool_use", "id": "tu_2", "name": "echo", "input": {"text": "second"}}
            ]
        }),
        // 最终回复
        json!({
            "content": [{"type": "text", "text": "Both echoes completed."}]
        }),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: None,
        system_prompt: None,
        tool_definitions: None,
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        None,
        vec![make_user_msg("echo two things")],
        &config,
    )
    .expect("should succeed");

    assert_eq!(result.final_text, "Both echoes completed.");
    assert_eq!(result.iterations, 2);
    // user → assistant → tool_result_1 → tool_result_2 → assistant
    assert_eq!(result.messages.len(), 5);
}

// ---------------------------------------------------------------------------
// 捕获 payload 的 Executor（供 system_prompt / tool_definitions 测试使用）
// ---------------------------------------------------------------------------

struct CapturingExecutor {
    responses: Vec<Value>,
    call_count: AtomicUsize,
    captured_payloads: Mutex<Vec<Value>>,
}

impl CapturingExecutor {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
            captured_payloads: Mutex::new(Vec::new()),
        }
    }

    fn payloads(&self) -> Vec<Value> {
        self.captured_payloads.lock().clone()
    }
}

impl TaskExecutor for CapturingExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        self.captured_payloads.lock().push(content);
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(idx)
            .cloned()
            .ok_or_else(|| "no more mock responses".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "capture-session"}))
    }
}

/// 验证 payload 中包含 system prompt
#[test]
fn test_system_prompt_passed_in_payload() {
    let executor = CapturingExecutor::new(vec![json!({
        "content": [{"type": "text", "text": "ok"}]
    })]);

    let registry = ToolRegistry::new();
    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: None,
        system_prompt: Some("You are a helpful assistant.".into()),
        tool_definitions: None,
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        None,
        vec![make_user_msg("hello")],
        &config,
    )
    .expect("should succeed");

    assert_eq!(result.final_text, "ok");

    let payloads = executor.payloads();
    assert_eq!(payloads.len(), 1);
    let payload = &payloads[0];
    assert_eq!(payload["system"], "You are a helpful assistant.");
    assert!(payload.get("tools").is_none());
}

/// 验证 payload 中包含 tool definitions
#[test]
fn test_tool_definitions_passed_in_payload() {
    let executor = CapturingExecutor::new(vec![json!({
        "content": [{"type": "text", "text": "done"}]
    })]);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let tool_defs = registry.tool_definitions();

    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: None,
        system_prompt: Some("identity".into()),
        tool_definitions: Some(tool_defs),
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &ToolRegistry::new(),
        None,
        None,
        vec![make_user_msg("hi")],
        &config,
    )
    .expect("should succeed");

    assert_eq!(result.final_text, "done");

    let payloads = executor.payloads();
    assert_eq!(payloads.len(), 1);
    let payload = &payloads[0];

    // system 字段存在
    assert_eq!(payload["system"], "identity");

    // tools 字段存在且包含 echo 工具
    let tools = payload["tools"].as_array().expect("tools should be array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "echo");
    assert!(tools[0].get("description").is_some());
    assert!(tools[0].get("input_schema").is_some());
}
