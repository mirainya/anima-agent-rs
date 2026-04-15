//! Agentic Loop 集成测试
//!
//! 验证 agentic loop 与 ToolRegistry、MockExecutor、EchoTool 的端到端集成。

use anima_runtime::agent::TaskExecutor;
use anima_runtime::execution::agentic_loop::{
    continue_agentic_loop, resume_suspended_tool_invocation, run_agentic_loop, AgenticLoopConfig,
    AgenticLoopOutcome,
};
use anima_runtime::hooks::{HookEvent, HookHandler, HookRegistry, HookResult};
use anima_runtime::messages::types::{InternalMsg, MessageRole};
use anima_runtime::permissions::{PermissionChecker, PermissionMode};
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

#[derive(Debug)]
struct TransformingHook {
    replacement_text: &'static str,
}

impl HookHandler for TransformingHook {
    fn handle(&self, event: &HookEvent) -> HookResult {
        match event {
            HookEvent::PreToolUse { input, .. } => {
                let mut next = input.clone();
                if let Some(obj) = next.as_object_mut() {
                    obj.insert("text".into(), Value::String(self.replacement_text.into()));
                }
                HookResult::Transform(next)
            }
            _ => HookResult::Continue,
        }
    }
}

#[derive(Debug)]
struct BlockingHook;

impl HookHandler for BlockingHook {
    fn handle(&self, event: &HookEvent) -> HookResult {
        match event {
            HookEvent::PreToolUse { .. } => HookResult::Block("policy denied".into()),
            _ => HookResult::Continue,
        }
    }
}

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
        ..Default::default()
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

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
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
        ..Default::default()
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

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
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
        ..Default::default()
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

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
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
        system_prompt: Some("You are a helpful assistant.".into()),
        ..Default::default()
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

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
    assert_eq!(result.final_text, "ok");

    let payloads = executor.payloads();
    assert_eq!(payloads.len(), 1);
    let payload = &payloads[0];
    assert_eq!(payload["system"], "You are a helpful assistant.");
    assert!(payload.get("tools").is_none());
}

#[test]
fn agentic_loop_suspends_when_permission_requires_confirmation() {
    let executor = SequenceExecutor::new(vec![json!({
        "content": [
            {"type": "text", "text": "Need confirmation"},
            {"type": "tool_use", "id": "tu_perm_1", "name": "echo", "input": {"text": "suspend me"}},
            {"type": "tool_use", "id": "tu_perm_2", "name": "echo", "input": {"text": "should not run this turn"}}
        ]
    })]);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let checker = PermissionChecker::new(PermissionMode::RuleBased);
    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "perm-session".into(),
        trace_id: "perm-trace".into(),
        ..Default::default()
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        Some(&checker),
        None,
        vec![make_user_msg("please use echo")],
        &config,
    )
    .expect("loop should suspend instead of failing");

    let AgenticLoopOutcome::Suspended(suspension) = result else {
        panic!("loop should suspend on interactive permission");
    };
    let suspension = *suspension;
    assert_eq!(suspension.suspended_tool.tool_use_id, "tu_perm_1");
    assert_eq!(suspension.suspended_tool.tool_name, "echo");
    assert_eq!(
        suspension.suspended_tool.permission_request.options,
        vec!["allow", "deny"]
    );
    assert_eq!(
        suspension.messages.len(),
        2,
        "only user + assistant(tool_use) should exist before approval"
    );
}

#[test]
fn agentic_loop_allow_resumes_original_tool_invocation() {
    let executor = SequenceExecutor::new(vec![
        json!({
            "content": [
                {"type": "text", "text": "Need confirmation"},
                {"type": "tool_use", "id": "tu_perm_allow", "name": "echo", "input": {"text": "approved"}}
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "Approval finished."}]
        }),
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let checker = PermissionChecker::new(PermissionMode::RuleBased);
    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "perm-allow-session".into(),
        trace_id: "perm-allow-trace".into(),
        ..Default::default()
    };

    let initial = run_agentic_loop(
        &client,
        &executor,
        &registry,
        Some(&checker),
        None,
        vec![make_user_msg("please use echo")],
        &config,
    )
    .expect("initial loop should suspend");
    let AgenticLoopOutcome::Suspended(suspension) = initial else {
        panic!("loop should suspend first");
    };
    let suspension = *suspension;

    let resumed_messages =
        resume_suspended_tool_invocation(&suspension, true, &registry, None, &config)
            .expect("approval should resume tool invocation");
    let resumed = continue_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        None,
        resumed_messages,
        suspension.iterations,
        suspension.compact_count,
        &config,
    )
    .expect("continued loop should complete");

    let AgenticLoopOutcome::Completed(result) = resumed else {
        panic!("loop should complete after allow");
    };
    assert_eq!(result.final_text, "Approval finished.");
    let tool_result_msg = &result.messages[2];
    let blocks = tool_result_msg
        .content
        .as_array()
        .expect("tool_result should be array");
    assert_eq!(blocks[0]["tool_use_id"], "tu_perm_allow");
    assert_eq!(blocks[0]["is_error"], false);
    assert!(blocks[0]["content"]
        .as_str()
        .unwrap_or_default()
        .contains("echoed: approved"));
}

#[test]
fn agentic_loop_pre_tool_transform_changes_actual_tool_input() {
    let executor = SequenceExecutor::new(vec![
        json!({
            "content": [
                {"type": "text", "text": "I'll echo that for you."},
                {"type": "tool_use", "id": "tu_transform_1", "name": "echo", "input": {"text": "original"}}
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "Transformed done."}]
        }),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let mut hook_registry = HookRegistry::new();
    hook_registry.register_pre_hook(Arc::new(TransformingHook {
        replacement_text: "transformed",
    }));

    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "transform-session".into(),
        trace_id: "transform-trace".into(),
        ..Default::default()
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        Some(&hook_registry),
        vec![make_user_msg("echo 'original'")],
        &config,
    )
    .expect("agentic loop should succeed");

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
    let tool_result_msg = &result.messages[2];
    let blocks = tool_result_msg.content.as_array().expect("should be array");
    assert_eq!(blocks[0]["is_error"], true == false);
    assert!(blocks[0]["content"]
        .as_str()
        .unwrap_or_default()
        .contains("echoed: transformed"));
}

#[test]
fn agentic_loop_pre_tool_block_returns_error_tool_result() {
    let executor = SequenceExecutor::new(vec![
        json!({
            "content": [
                {"type": "text", "text": "Let me try."},
                {"type": "tool_use", "id": "tu_block_1", "name": "echo", "input": {"text": "blocked"}}
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "Blocked path finished."}]
        }),
    ]);

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let mut hook_registry = HookRegistry::new();
    hook_registry.register_pre_hook(Arc::new(BlockingHook));

    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "block-session".into(),
        trace_id: "block-trace".into(),
        ..Default::default()
    };

    let result = run_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        Some(&hook_registry),
        vec![make_user_msg("echo 'blocked'")],
        &config,
    )
    .expect("agentic loop should succeed");

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
    let tool_result_msg = &result.messages[2];
    let blocks = tool_result_msg.content.as_array().expect("should be array");
    assert_eq!(blocks[0]["is_error"], true);
    assert!(blocks[0]["content"]
        .as_str()
        .unwrap_or_default()
        .contains("blocked by hook: policy denied"));
}

#[test]
fn agentic_loop_resume_path_applies_pre_tool_transform() {
    let executor = SequenceExecutor::new(vec![
        json!({
            "content": [
                {"type": "text", "text": "Need confirmation"},
                {"type": "tool_use", "id": "tu_perm_transform", "name": "echo", "input": {"text": "approved"}}
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "Approval finished."}]
        }),
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let checker = PermissionChecker::new(PermissionMode::RuleBased);
    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "perm-transform-session".into(),
        trace_id: "perm-transform-trace".into(),
        ..Default::default()
    };

    let initial = run_agentic_loop(
        &client,
        &executor,
        &registry,
        Some(&checker),
        None,
        vec![make_user_msg("please use echo")],
        &config,
    )
    .expect("initial loop should suspend");
    let AgenticLoopOutcome::Suspended(suspension) = initial else {
        panic!("loop should suspend first");
    };
    let suspension = *suspension;

    let mut hook_registry = HookRegistry::new();
    hook_registry.register_pre_hook(Arc::new(TransformingHook {
        replacement_text: "transformed-on-resume",
    }));

    let resumed_messages = resume_suspended_tool_invocation(
        &suspension,
        true,
        &registry,
        Some(&hook_registry),
        &config,
    )
    .expect("approval should resume tool invocation");
    let resumed = continue_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        Some(&hook_registry),
        resumed_messages,
        suspension.iterations,
        suspension.compact_count,
        &config,
    )
    .expect("continued loop should complete");

    let AgenticLoopOutcome::Completed(result) = resumed else {
        panic!("loop should complete after allow");
    };
    let tool_result_msg = &result.messages[2];
    let blocks = tool_result_msg.content.as_array().expect("should be array");
    assert!(blocks[0]["content"]
        .as_str()
        .unwrap_or_default()
        .contains("echoed: transformed-on-resume"));
}

#[test]
fn agentic_loop_deny_injects_error_tool_result_and_continues() {
    let executor = SequenceExecutor::new(vec![
        json!({
            "content": [
                {"type": "text", "text": "Need confirmation"},
                {"type": "tool_use", "id": "tu_perm_deny", "name": "echo", "input": {"text": "denied"}}
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "Denied path finished."}]
        }),
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let checker = PermissionChecker::new(PermissionMode::RuleBased);
    let client = make_client();
    let config = AgenticLoopConfig {
        max_iterations: 5,
        session_id: "perm-deny-session".into(),
        trace_id: "perm-deny-trace".into(),
        ..Default::default()
    };

    let initial = run_agentic_loop(
        &client,
        &executor,
        &registry,
        Some(&checker),
        None,
        vec![make_user_msg("please use echo")],
        &config,
    )
    .expect("initial loop should suspend");
    let AgenticLoopOutcome::Suspended(suspension) = initial else {
        panic!("loop should suspend first");
    };
    let suspension = *suspension;

    let resumed_messages =
        resume_suspended_tool_invocation(&suspension, false, &registry, None, &config)
            .expect("denial should still produce a tool_result");
    let resumed = continue_agentic_loop(
        &client,
        &executor,
        &registry,
        None,
        None,
        resumed_messages,
        suspension.iterations,
        suspension.compact_count,
        &config,
    )
    .expect("continued loop should complete");

    let AgenticLoopOutcome::Completed(result) = resumed else {
        panic!("loop should complete after deny");
    };
    assert_eq!(result.final_text, "Denied path finished.");
    let tool_result_msg = &result.messages[2];
    let blocks = tool_result_msg
        .content
        .as_array()
        .expect("tool_result should be array");
    assert_eq!(blocks[0]["tool_use_id"], "tu_perm_deny");
    assert_eq!(blocks[0]["is_error"], true);
    assert!(blocks[0]["content"]
        .as_str()
        .unwrap_or_default()
        .contains("denied by user"));
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
        system_prompt: Some("identity".into()),
        tool_definitions: Some(tool_defs),
        ..Default::default()
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

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("agentic loop should complete");
    };
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
