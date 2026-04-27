//! 集成测试：Agentic Loop 上下文压缩

use anima_runtime::execution::agentic_loop::{
    run_agentic_loop, AgenticLoopConfig, AgenticLoopOutcome,
};
use anima_runtime::messages::compact::CompactConfig;
use anima_runtime::messages::types::{blocks_from_value, ContentBlock, InternalMsg, MessageRole};
use anima_runtime::provider::{ChatResponse, Provider, ProviderError, StopReason};
use anima_runtime::provider::types::ChatRequest;
use anima_runtime::tools::definition::{Tool, ToolContext};
use anima_runtime::tools::registry::ToolRegistry;
use anima_runtime::tools::result::{ToolError, ToolResult};

use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct MockExecutor {
    responses: Vec<Value>,
    call_count: AtomicUsize,
}

impl MockExecutor {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
        }
    }
}

impl Provider for MockExecutor {
    fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        let raw = self.responses.get(idx).cloned()
            .ok_or_else(|| ProviderError::internal("no more mock responses"))?;
        let content_value = raw.get("content").cloned().unwrap_or(Value::Null);
        let blocks = blocks_from_value(&content_value, None);
        let has_tool_use = blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        let stop_reason = if has_tool_use { StopReason::ToolUse } else { StopReason::EndTurn };
        Ok(ChatResponse { content: blocks, stop_reason, usage: None, raw })
    }
}

#[derive(Debug)]
struct BigEchoTool;

impl Tool for BigEchoTool {
    fn name(&self) -> &str {
        "big_echo"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn validate_input(&self, _input: &Value) -> Result<(), String> {
        Ok(())
    }

    fn call(&self, _input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        // 返回大量内容以触发压缩
        Ok(ToolResult::text("x".repeat(8000)))
    }
}


fn make_user_msg(text: &str) -> InternalMsg {
    InternalMsg {
        role: MessageRole::User,
        blocks: vec![ContentBlock::Text { text: text.into() }],
        message_id: Uuid::new_v4().to_string(),
        tool_use_id: None,
        filtered: false,
        metadata: json!({}),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 启用压缩但消息量小，行为不变
#[test]
fn test_loop_compact_enabled_no_trigger() {
    let executor = MockExecutor::new(vec![json!({
        "content": [{"type": "text", "text": "done"}]
    })]);

    let registry = ToolRegistry::new();
    let config = AgenticLoopConfig {
        max_iterations: 10,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: Some(CompactConfig::default()),
        ..Default::default()
    };
    let initial = vec![make_user_msg("hello")];

    let result =
        run_agentic_loop(&executor, &registry, None, None, initial, &config).unwrap();

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("loop should complete");
    };
    assert_eq!(result.final_text, "done");
    assert_eq!(result.compact_count, 0);
}

/// 大量 tool_result → 压缩触发 → 循环继续正常
#[test]
fn test_loop_compact_triggered() {
    // 5 轮工具调用 + 最终回复
    let mut responses = Vec::new();
    for i in 0..5 {
        responses.push(json!({
            "content": [
                {"type": "text", "text": format!("call {i}")},
                {"type": "tool_use", "id": format!("tu_{i}"), "name": "big_echo", "input": {}}
            ]
        }));
    }
    responses.push(json!({
        "content": [{"type": "text", "text": "all done"}]
    }));

    let executor = MockExecutor::new(responses);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BigEchoTool));
    let config = AgenticLoopConfig {
        max_iterations: 10,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: Some(CompactConfig {
            context_window: 5000,
            threshold_ratio: 0.5,
            buffer_tokens: 500,
            preserve_recent_turns: 1,
        }),
        ..Default::default()
    };
    let initial = vec![make_user_msg("run big echo 5 times")];

    let result =
        run_agentic_loop(&executor, &registry, None, None, initial, &config).unwrap();

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("loop should complete");
    };
    assert_eq!(result.final_text, "all done");
    assert!(result.compact_count > 0, "compaction should have triggered");
}

/// 最新 turn 的 tool_result 未被清除
#[test]
fn test_loop_compact_preserves_latest_turn() {
    // 2 轮工具调用 + 最终回复
    let responses = vec![
        json!({
            "content": [
                {"type": "text", "text": "call1"},
                {"type": "tool_use", "id": "tu_0", "name": "big_echo", "input": {}}
            ]
        }),
        json!({
            "content": [
                {"type": "text", "text": "call2"},
                {"type": "tool_use", "id": "tu_1", "name": "big_echo", "input": {}}
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "final"}]
        }),
    ];

    let executor = MockExecutor::new(responses);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BigEchoTool));
    let config = AgenticLoopConfig {
        max_iterations: 10,
        session_id: "test".into(),
        trace_id: "test".into(),
        compact: Some(CompactConfig {
            context_window: 3000,
            threshold_ratio: 0.5,
            buffer_tokens: 200,
            preserve_recent_turns: 1,
        }),
        ..Default::default()
    };
    let initial = vec![make_user_msg("test")];

    let result =
        run_agentic_loop(&executor, &registry, None, None, initial, &config).unwrap();

    let AgenticLoopOutcome::Completed(result) = result else {
        panic!("loop should complete");
    };
    assert_eq!(result.final_text, "final");

    // 最后一个 tool_result 不应被 filtered
    let last_tool_result = result
        .messages
        .iter()
        .rev()
        .find(|m| m.tool_use_id.is_some())
        .expect("should have a tool_result");
    assert!(
        !last_tool_result.filtered,
        "latest tool_result should not be filtered"
    );
}
