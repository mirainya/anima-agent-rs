//! Agentic Loop：模型驱动的工具循环
//!
//! 对标 claude-code-main 的 `queryLoop()` 模式。
//! 模型自主调用工具、观察结果、继续推理，直到给出最终回答或达到迭代上限。

use serde_json::{json, Value};
use uuid::Uuid;

use crate::agent::executor::TaskExecutor;
use crate::hooks::HookRegistry;
use crate::messages::normalize::normalize_messages_for_api;
use crate::messages::pairing::ensure_tool_result_pairing;
use crate::messages::types::{ApiMsg, InternalMsg, MessageRole};
use crate::permissions::PermissionChecker;
use crate::tools::definition::ToolContext;
use crate::tools::execution::{run_tool_use, RunToolOptions};
use crate::tools::registry::ToolRegistry;
use crate::tools::result::ToolResult;

use anima_sdk::facade::Client as SdkClient;

// ---------------------------------------------------------------------------
// 类型定义
// ---------------------------------------------------------------------------

/// Agentic loop 配置
#[derive(Debug, Clone)]
pub struct AgenticLoopConfig {
    /// 最大迭代次数（默认 10）
    pub max_iterations: usize,
    /// 会话 ID
    pub session_id: String,
    /// 追踪 ID
    pub trace_id: String,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            session_id: String::new(),
            trace_id: String::new(),
        }
    }
}

/// 循环执行结果
#[derive(Debug, Clone)]
pub struct AgenticLoopResult {
    /// 模型最终的文本回复
    pub final_text: String,
    /// 完整消息历史
    pub messages: Vec<InternalMsg>,
    /// 实际执行的迭代次数
    pub iterations: usize,
    /// 是否因达到迭代上限而停止
    pub hit_limit: bool,
}

/// Agentic loop 错误类型
#[derive(Debug, Clone)]
pub enum AgenticLoopError {
    /// API 调用失败
    ApiCallFailed(String),
    /// 响应解析错误
    ResponseParseError(String),
    /// 所有工具执行均失败
    AllToolsFailed(String),
}

impl std::fmt::Display for AgenticLoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiCallFailed(msg) => write!(f, "API call failed: {msg}"),
            Self::ResponseParseError(msg) => write!(f, "response parse error: {msg}"),
            Self::AllToolsFailed(msg) => write!(f, "all tools failed: {msg}"),
        }
    }
}

impl std::error::Error for AgenticLoopError {}

// ---------------------------------------------------------------------------
// 响应解析
// ---------------------------------------------------------------------------

/// 解析后的模型响应
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    /// 模型回复的纯文本部分
    pub text: String,
    /// 模型请求的工具调用
    pub tool_uses: Vec<ParsedToolUse>,
}

/// 解析后的单个工具调用
#[derive(Debug, Clone)]
pub struct ParsedToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// 解析 API 响应，提取文本和 tool_use
///
/// 支持两种格式：
/// - `{"content": [{"type":"text","text":"..."}, {"type":"tool_use",...}]}`
/// - `{"content": "plain text"}`
pub fn parse_response(response: &Value) -> Result<ParsedResponse, AgenticLoopError> {
    let content = response.get("content");

    // 格式1: content 是字符串
    if let Some(Value::String(text)) = content {
        return Ok(ParsedResponse {
            text: text.clone(),
            tool_uses: vec![],
        });
    }

    // 格式2: content 是数组
    if let Some(Value::Array(parts)) = content {
        let mut text_parts = Vec::new();
        let mut tool_uses = Vec::new();

        for part in parts {
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
            match part_type {
                "text" => {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        text_parts.push(t.to_string());
                    }
                }
                "tool_use" => {
                    let id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let name = part
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let input = part.get("input").cloned().unwrap_or(Value::Object(Default::default()));
                    tool_uses.push(ParsedToolUse { id, name, input });
                }
                _ => {}
            }
        }

        return Ok(ParsedResponse {
            text: text_parts.join("\n"),
            tool_uses,
        });
    }

    // 格式3: 顶层没有 content 字段，尝试直接作为纯文本
    if let Some(text) = response.as_str() {
        return Ok(ParsedResponse {
            text: text.to_string(),
            tool_uses: vec![],
        });
    }

    Err(AgenticLoopError::ResponseParseError(format!(
        "unexpected response structure: {}",
        serde_json::to_string(response).unwrap_or_default()
    )))
}

// ---------------------------------------------------------------------------
// 消息构建辅助
// ---------------------------------------------------------------------------

/// 构建 assistant 消息（保留原始响应内容）
pub fn build_assistant_msg(response: &Value) -> InternalMsg {
    let content = response
        .get("content")
        .cloned()
        .unwrap_or_else(|| Value::String(String::new()));
    InternalMsg {
        role: MessageRole::Assistant,
        content,
        message_id: Uuid::new_v4().to_string(),
        tool_use_id: None,
        filtered: false,
        metadata: json!({}),
    }
}

/// 构建 tool_result 消息
pub fn build_tool_result_msg(tool_use_id: &str, result: &ToolResult) -> InternalMsg {
    let content_text = result
        .blocks
        .iter()
        .map(|block| match block {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            crate::tools::result::ToolResultBlock::Json(v) => {
                serde_json::to_string(v).unwrap_or_default()
            }
            crate::tools::result::ToolResultBlock::Binary { mime_type, data } => {
                format!("[binary: {mime_type}, {} bytes]", data.len())
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    InternalMsg {
        role: MessageRole::User,
        content: json!([{
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content_text,
            "is_error": result.is_error
        }]),
        message_id: Uuid::new_v4().to_string(),
        tool_use_id: Some(tool_use_id.to_string()),
        filtered: false,
        metadata: json!({"auto_generated": true}),
    }
}

/// 将 ApiMsg 列表序列化为 API 期望的 messages Value
pub fn build_api_content(api_msgs: &[ApiMsg]) -> Value {
    let messages: Vec<Value> = api_msgs
        .iter()
        .map(|msg| {
            json!({
                "role": msg.role,
                "content": msg.content,
            })
        })
        .collect();
    json!(messages)
}

// ---------------------------------------------------------------------------
// 单工具执行
// ---------------------------------------------------------------------------

/// 执行单个 tool_use，所有错误转为 ToolResult::error（不会 panic）
fn execute_single_tool(
    tool_use: &ParsedToolUse,
    tool_registry: &ToolRegistry,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    config: &AgenticLoopConfig,
) -> ToolResult {
    let tool = match tool_registry.get(&tool_use.name) {
        Some(t) => t,
        None => {
            return ToolResult::error(format!("tool '{}' not found in registry", tool_use.name));
        }
    };

    let options = RunToolOptions {
        tool_name: tool_use.name.clone(),
        input: tool_use.input.clone(),
        context: ToolContext {
            session_id: config.session_id.clone(),
            trace_id: config.trace_id.clone(),
            metadata: json!({}),
        },
    };

    match run_tool_use(tool, options, permission_checker, hook_registry) {
        Ok(result) => result,
        Err(err) => ToolResult::error(format!("tool execution error: {err}")),
    }
}

// ---------------------------------------------------------------------------
// 核心循环
// ---------------------------------------------------------------------------

/// 运行 Agentic Loop
///
/// 模型驱动的工具循环：模型调用工具 → 观察结果 → 继续推理，
/// 直到模型给出最终回答（不再请求工具）或达到迭代上限。
pub fn run_agentic_loop(
    client: &SdkClient,
    executor: &dyn TaskExecutor,
    tool_registry: &ToolRegistry,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    initial_messages: Vec<InternalMsg>,
    config: &AgenticLoopConfig,
) -> Result<AgenticLoopResult, AgenticLoopError> {
    let mut messages = initial_messages;
    let mut iterations = 0;

    loop {
        // 1. 守卫：达到最大迭代次数
        if iterations >= config.max_iterations {
            let final_text = extract_last_assistant_text(&messages);
            return Ok(AgenticLoopResult {
                final_text,
                messages,
                iterations,
                hit_limit: true,
            });
        }

        // 2. 确保 tool_use / tool_result 配对
        ensure_tool_result_pairing(&mut messages);

        // 3. 标准化消息
        let api_msgs = normalize_messages_for_api(&messages);

        // 4. 构建 API 请求内容
        let content = build_api_content(&api_msgs);

        // 5. 调用 API
        let response = executor
            .send_prompt(client, &config.session_id, content)
            .map_err(AgenticLoopError::ApiCallFailed)?;

        // 6. 解析响应
        let parsed = parse_response(&response)?;

        // 7. 追加 assistant 消息
        messages.push(build_assistant_msg(&response));

        // 8. 无 tool_use → 最终回复
        if parsed.tool_uses.is_empty() {
            return Ok(AgenticLoopResult {
                final_text: parsed.text,
                messages,
                iterations: iterations + 1,
                hit_limit: false,
            });
        }

        // 9. 执行每个 tool_use
        for tool_use in &parsed.tool_uses {
            let result = execute_single_tool(
                tool_use,
                tool_registry,
                permission_checker,
                hook_registry,
                config,
            );
            let tool_result_msg = build_tool_result_msg(&tool_use.id, &result);
            messages.push(tool_result_msg);
        }

        // 10. 递增迭代计数，继续循环
        iterations += 1;
    }
}

/// 从消息历史中提取最后一条 assistant 消息的文本
fn extract_last_assistant_text(messages: &[InternalMsg]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant)
        .map(|m| match &m.content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(Value::as_str) == Some("text") {
                        p.get("text").and_then(Value::as_str).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            other => serde_json::to_string(other).unwrap_or_default(),
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::definition::Tool;
    use crate::tools::result::{ToolError, ToolResult};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ---- Mock TaskExecutor ----

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

    impl TaskExecutor for MockExecutor {
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
                .ok_or_else(|| "no more mock responses".to_string())
        }

        fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
            Ok(json!({"id": "mock-session"}))
        }
    }

    // ---- Mock Tool ----

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

        fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
            let text = input
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("echo");
            Ok(ToolResult::text(format!("echoed: {text}")))
        }
    }

    fn make_config() -> AgenticLoopConfig {
        AgenticLoopConfig {
            max_iterations: 10,
            session_id: "test-session".into(),
            trace_id: "test-trace".into(),
        }
    }

    fn make_user_msg(text: &str) -> InternalMsg {
        InternalMsg {
            role: MessageRole::User,
            content: json!(text),
            message_id: Uuid::new_v4().to_string(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }
    }

    fn make_client() -> SdkClient {
        // 使用一个空配置创建 mock client（测试不会真正调用网络）
        SdkClient::new("http://localhost:0")
    }

    // ---- parse_response 测试 ----

    #[test]
    fn test_parse_response_text_only() {
        let resp = json!({
            "content": [{"type": "text", "text": "Hello, world!"}]
        });
        let parsed = parse_response(&resp).unwrap();
        assert_eq!(parsed.text, "Hello, world!");
        assert!(parsed.tool_uses.is_empty());
    }

    #[test]
    fn test_parse_response_string_content() {
        let resp = json!({"content": "simple text"});
        let parsed = parse_response(&resp).unwrap();
        assert_eq!(parsed.text, "simple text");
        assert!(parsed.tool_uses.is_empty());
    }

    #[test]
    fn test_parse_response_with_tool_use() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "Let me help"},
                {"type": "tool_use", "id": "tu_1", "name": "echo", "input": {"text": "hi"}}
            ]
        });
        let parsed = parse_response(&resp).unwrap();
        assert_eq!(parsed.text, "Let me help");
        assert_eq!(parsed.tool_uses.len(), 1);
        assert_eq!(parsed.tool_uses[0].id, "tu_1");
        assert_eq!(parsed.tool_uses[0].name, "echo");
        assert_eq!(parsed.tool_uses[0].input, json!({"text": "hi"}));
    }

    #[test]
    fn test_parse_response_multiple_tool_uses() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "Running tools"},
                {"type": "tool_use", "id": "tu_1", "name": "echo", "input": {"text": "a"}},
                {"type": "tool_use", "id": "tu_2", "name": "echo", "input": {"text": "b"}}
            ]
        });
        let parsed = parse_response(&resp).unwrap();
        assert_eq!(parsed.tool_uses.len(), 2);
        assert_eq!(parsed.tool_uses[0].id, "tu_1");
        assert_eq!(parsed.tool_uses[1].id, "tu_2");
    }

    // ---- build_tool_result_msg 测试 ----

    #[test]
    fn test_build_tool_result_msg() {
        let result = ToolResult::text("hello result");
        let msg = build_tool_result_msg("tu_1", &result);
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.tool_use_id.as_deref(), Some("tu_1"));

        let content_arr = msg.content.as_array().unwrap();
        assert_eq!(content_arr.len(), 1);
        let block = &content_arr[0];
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "tu_1");
        assert_eq!(block["content"], "hello result");
        assert_eq!(block["is_error"], false);
    }

    #[test]
    fn test_build_tool_result_msg_error() {
        let result = ToolResult::error("something went wrong");
        let msg = build_tool_result_msg("tu_2", &result);

        let block = &msg.content.as_array().unwrap()[0];
        assert_eq!(block["is_error"], true);
        assert_eq!(block["content"], "something went wrong");
    }

    // ---- 循环测试 ----

    #[test]
    fn test_loop_no_tools() {
        // 模型直接返回文本，不请求工具 → 单轮返回
        let executor = MockExecutor::new(vec![json!({
            "content": [{"type": "text", "text": "The answer is 42."}]
        })]);

        let registry = ToolRegistry::new();
        let client = make_client();
        let config = make_config();
        let initial = vec![make_user_msg("What is the answer?")];

        let result =
            run_agentic_loop(&client, &executor, &registry, None, None, initial, &config)
                .unwrap();

        assert_eq!(result.final_text, "The answer is 42.");
        assert_eq!(result.iterations, 1);
        assert!(!result.hit_limit);
    }

    #[test]
    fn test_loop_one_tool_round() {
        // 第一次：模型请求调用 echo 工具
        // 第二次：模型返回最终文本
        let executor = MockExecutor::new(vec![
            json!({
                "content": [
                    {"type": "text", "text": "Let me call echo"},
                    {"type": "tool_use", "id": "tu_1", "name": "echo", "input": {"text": "hello"}}
                ]
            }),
            json!({
                "content": [{"type": "text", "text": "Echo returned: hello"}]
            }),
        ]);

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let client = make_client();
        let config = make_config();
        let initial = vec![make_user_msg("Echo hello for me")];

        let result =
            run_agentic_loop(&client, &executor, &registry, None, None, initial, &config)
                .unwrap();

        assert_eq!(result.final_text, "Echo returned: hello");
        assert_eq!(result.iterations, 2);
        assert!(!result.hit_limit);
    }

    #[test]
    fn test_loop_hits_max_iterations() {
        // 模型每次都请求 tool_use，永远不返回纯文本
        let tool_response = json!({
            "content": [
                {"type": "text", "text": "calling again"},
                {"type": "tool_use", "id": "tu_1", "name": "echo", "input": {"text": "loop"}}
            ]
        });

        // 提供足够多的相同响应
        let responses: Vec<Value> = (0..20).map(|_| tool_response.clone()).collect();
        let executor = MockExecutor::new(responses);

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let client = make_client();
        let config = AgenticLoopConfig {
            max_iterations: 3,
            session_id: "test".into(),
            trace_id: "test".into(),
        };
        let initial = vec![make_user_msg("Keep looping")];

        let result =
            run_agentic_loop(&client, &executor, &registry, None, None, initial, &config)
                .unwrap();

        assert!(result.hit_limit);
        assert_eq!(result.iterations, 3);
    }
}
