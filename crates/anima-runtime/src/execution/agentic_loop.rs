//! Agentic Loop：模型驱动的工具循环
//!
//! 对标 claude-code-main 的 `queryLoop()` 模式。
//! 模型自主调用工具、观察结果、继续推理，直到给出最终回答或达到迭代上限。

use serde_json::{json, Value};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

use crate::provider::{ChatRequest, ChatResponse, Provider, ProviderError};
use crate::provider::types::{ChatMessage, ChatRole};
use crate::agent::runtime_error::{RuntimeError, RuntimeErrorKind, RuntimeErrorStage};
use crate::hooks::HookRegistry;
use crate::messages::compact::{compact_if_needed, CompactConfig};
use crate::messages::normalize::normalize_messages_for_api;
use crate::messages::pairing::ensure_tool_result_pairing;
use crate::messages::types::{blocks_from_value, ApiMsg, ContentBlock, InternalMsg, MessageRole};
use crate::permissions::{PermissionChecker, PermissionRequest};
use crate::streaming::types::StreamEvent;
use crate::tools::definition::ToolContext;
use crate::tools::execution::{
    execute_tool_after_permission, run_tool_use, AwaitingToolPermission, RunToolOptions,
    RunToolUseOutcome, ToolInvocationPhase, ToolInvocationRecord, ToolLifecycleEventCallback,
};
use crate::tools::registry::ToolRegistry;
use crate::tools::result::{ToolError, ToolResult};

// ---------------------------------------------------------------------------
// 类型定义
// ---------------------------------------------------------------------------

/// Agentic loop 配置
#[derive(Clone)]
pub struct AgenticLoopConfig {
    /// 最大迭代次数（默认 10）
    pub max_iterations: usize,
    /// 会话 ID
    pub session_id: String,
    /// 追踪 ID
    pub trace_id: String,
    /// 上下文压缩配置，None = 不压缩
    pub compact: Option<CompactConfig>,
    /// 系统提示词，None = 不发送
    pub system_prompt: Option<String>,
    /// 工具定义列表，None = 不发送
    pub tool_definitions: Option<Vec<Value>>,
    /// 是否启用流式响应（默认 false）
    pub streaming: bool,
    /// 流式事件回调（可选），CLI/Web 可用于实时 UI 更新
    pub on_stream_event: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    /// 工具生命周期事件回调（可选），供 runtime timeline / projection 订阅
    pub on_tool_lifecycle_event: Option<ToolLifecycleEventCallback>,
}

impl std::fmt::Debug for AgenticLoopConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgenticLoopConfig")
            .field("max_iterations", &self.max_iterations)
            .field("session_id", &self.session_id)
            .field("trace_id", &self.trace_id)
            .field("compact", &self.compact)
            .field("system_prompt", &self.system_prompt)
            .field("tool_definitions", &self.tool_definitions)
            .field("streaming", &self.streaming)
            .field("on_stream_event", &self.on_stream_event.is_some())
            .field(
                "on_tool_lifecycle_event",
                &self.on_tool_lifecycle_event.is_some(),
            )
            .finish()
    }
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            session_id: String::new(),
            trace_id: String::new(),
            compact: None,
            system_prompt: None,
            tool_definitions: None,
            streaming: false,
            on_stream_event: None,
            on_tool_lifecycle_event: None,
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
    /// 压缩执行次数
    pub compact_count: usize,
}

#[derive(Debug, Clone)]
pub struct SuspendedToolInvocation {
    pub tool_use_id: String,
    pub tool_name: String,
    pub tool_input: Value,
    pub permission_request: PermissionRequest,
    pub invocation: ToolInvocationRecord,
}

#[derive(Debug, Clone)]
pub struct AgenticLoopSuspension {
    pub suspended_tool: SuspendedToolInvocation,
    pub messages: Vec<InternalMsg>,
    pub iterations: usize,
    pub compact_count: usize,
}

#[derive(Debug, Clone)]
pub enum AgenticLoopOutcome {
    Completed(AgenticLoopResult),
    Suspended(Box<AgenticLoopSuspension>),
}

/// Agentic loop 错误类型
#[derive(Debug, Clone)]
pub enum AgenticLoopError {
    /// API 调用失败
    ApiCall { error: ProviderError },
    /// 响应解析错误
    ResponseParse { internal_message: String },
    /// 工具执行失败
    ToolExecution { error: ToolError },
    /// 所有工具执行均失败
    AllToolsFailed { internal_message: String },
}

impl AgenticLoopError {
    pub(crate) fn to_runtime_error(&self) -> RuntimeError {
        match self {
            Self::ApiCall { error } => {
                use crate::provider::ProviderErrorKind;
                let kind = match error.kind {
                    ProviderErrorKind::Timeout => RuntimeErrorKind::UpstreamTimeout,
                    ProviderErrorKind::StreamFailed => RuntimeErrorKind::UpstreamStreamFailed,
                    ProviderErrorKind::Network => RuntimeErrorKind::SessionCreateFailed,
                    _ => RuntimeErrorKind::TaskExecutionFailed,
                };
                RuntimeError::new(kind, RuntimeErrorStage::PlanExecute, &error.message)
            }
            Self::ResponseParse { internal_message } => RuntimeError::new(
                RuntimeErrorKind::ResponseParseFailed,
                RuntimeErrorStage::PlanExecute,
                internal_message.clone(),
            ),
            Self::ToolExecution { error } => {
                RuntimeError::from_tool_error(error, RuntimeErrorStage::PlanExecute)
            }
            Self::AllToolsFailed { internal_message } => RuntimeError::new(
                RuntimeErrorKind::ToolExecutionFailed,
                RuntimeErrorStage::PlanExecute,
                internal_message.clone(),
            ),
        }
    }
}

impl std::fmt::Display for AgenticLoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiCall { error } => write!(f, "API call failed: {}", error.message),
            Self::ResponseParse { internal_message } => {
                write!(f, "response parse error: {internal_message}")
            }
            Self::ToolExecution { error } => write!(f, "tool execution failed: {error}"),
            Self::AllToolsFailed { internal_message } => {
                write!(f, "all tools failed: {internal_message}")
            }
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
    /// 模型的思考过程
    pub thinking: Option<String>,
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
            thinking: None,
        });
    }

    // 格式2: content 是数组
    if let Some(Value::Array(parts)) = content {
        let mut text_parts = Vec::new();
        let mut tool_uses = Vec::new();
        let mut thinking_parts = Vec::new();

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
                    let input = part
                        .get("input")
                        .cloned()
                        .unwrap_or(Value::Object(Default::default()));
                    tool_uses.push(ParsedToolUse { id, name, input });
                }
                "thinking" => {
                    if let Some(t) = part.get("thinking").and_then(Value::as_str) {
                        thinking_parts.push(t.to_string());
                    }
                }
                _ => {}
            }
        }

        let thinking = if thinking_parts.is_empty() {
            None
        } else {
            Some(thinking_parts.join("\n"))
        };

        return Ok(ParsedResponse {
            text: text_parts.join("\n"),
            tool_uses,
            thinking,
        });
    }

    // 格式3: 顶层没有 content 字段，尝试直接作为纯文本
    if let Some(text) = response.as_str() {
        return Ok(ParsedResponse {
            text: text.to_string(),
            tool_uses: vec![],
            thinking: None,
        });
    }

    Err(AgenticLoopError::ResponseParse {
        internal_message: format!(
            "unexpected response structure: {}",
            serde_json::to_string(response).unwrap_or_default()
        ),
    })
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
        blocks: blocks_from_value(&content, None),
        message_id: Uuid::new_v4().to_string(),
        tool_use_id: None,
        filtered: false,
        metadata: json!({}),
    }
}

/// 构建 tool_result 消息
pub fn build_tool_result_msg(
    tool_use_id: &str,
    result: &ToolResult,
    invocation: Option<&ToolInvocationRecord>,
) -> InternalMsg {
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

    let mut metadata = json!({"auto_generated": true});
    if let Some(invocation) = invocation {
        metadata["tool_invocation"] = json!({
            "invocation_id": invocation.invocation_id,
            "tool_name": invocation.tool_name,
            "tool_use_id": invocation.tool_use_id,
            "phase": invocation.phase,
            "permission_state": invocation.permission_state,
            "started_at_ms": invocation.started_at_ms,
            "finished_at_ms": invocation.finished_at_ms,
            "result_summary": invocation.result_summary,
            "error_summary": invocation.error_summary,
        });
    }

    InternalMsg {
        role: MessageRole::User,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: Value::String(content_text),
            is_error: result.is_error,
        }],
        message_id: Uuid::new_v4().to_string(),
        tool_use_id: Some(tool_use_id.to_string()),
        filtered: false,
        metadata,
    }
}

/// 构建包含 system prompt 和 tool definitions 的完整 API payload
///
/// 当 `system_prompt` 或 `tool_definitions` 为 Some 时，返回的 Value
/// 包含 `"system"` / `"tools"` 顶层字段；否则仅含 `"messages"`。
/// 当 `streaming` 为 true 时，注入 `"stream": true`。
pub fn build_api_payload(
    api_msgs: &[ApiMsg],
    system_prompt: Option<&str>,
    tool_definitions: Option<&[Value]>,
    streaming: bool,
) -> Value {
    let messages: Vec<Value> = api_msgs
        .iter()
        .map(|msg| {
            json!({
                "role": msg.role,
                "content": msg.content,
            })
        })
        .collect();

    let mut payload = json!({ "messages": messages });
    if let Some(sp) = system_prompt {
        payload["system"] = json!(sp);
    }
    if let Some(tools) = tool_definitions {
        payload["tools"] = json!(tools);
    }
    if streaming {
        payload["stream"] = json!(true);
    }
    payload
}

fn build_chat_request(
    api_msgs: &[ApiMsg],
    system_prompt: Option<&str>,
    tool_definitions: Option<&[Value]>,
    session_id: &str,
) -> ChatRequest {
    let messages = api_msgs
        .iter()
        .map(|msg| {
            let role = match msg.role {
                MessageRole::User | MessageRole::System => ChatRole::User,
                MessageRole::Assistant => ChatRole::Assistant,
            };
            ChatMessage {
                role,
                content: blocks_from_value(&msg.content, None),
            }
        })
        .collect();

    ChatRequest {
        messages,
        system: system_prompt.map(String::from),
        tools: tool_definitions.map(|t| t.to_vec()),
        metadata: json!({ "session_id": session_id }),
        ..Default::default()
    }
}

fn parse_chat_response(response: &ChatResponse) -> Result<ParsedResponse, AgenticLoopError> {
    let mut text_parts = Vec::new();
    let mut tool_uses = Vec::new();
    let mut thinking_parts = Vec::new();

    for block in &response.content {
        match block {
            ContentBlock::Text { text } => text_parts.push(text.clone()),
            ContentBlock::ToolUse { id, name, input } => {
                tool_uses.push(ParsedToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            ContentBlock::Thinking { thinking } => thinking_parts.push(thinking.clone()),
            _ => {}
        }
    }

    let thinking = if thinking_parts.is_empty() {
        None
    } else {
        Some(thinking_parts.join("\n"))
    };

    Ok(ParsedResponse {
        text: text_parts.join("\n"),
        tool_uses,
        thinking,
    })
}

// ---------------------------------------------------------------------------
// 单工具执行
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum SingleToolExecutionOutcome {
    Completed {
        invocation: ToolInvocationRecord,
        result: ToolResult,
    },
    AwaitingPermission(Box<AwaitingToolPermission>),
}

#[derive(Debug, Clone)]
struct IndexedToolUse {
    index: usize,
    tool_use: ParsedToolUse,
}

#[derive(Debug, Clone)]
struct IndexedToolOutcome {
    index: usize,
    tool_use_id: String,
    outcome: SingleToolExecutionOutcome,
}

/// 执行单个 tool_use；真实错误转为 ToolResult::error，但保留 AwaitingPermission 语义
fn execute_single_tool(
    tool_use: ParsedToolUse,
    tool_registry: &ToolRegistry,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    config: &AgenticLoopConfig,
) -> SingleToolExecutionOutcome {
    let callback = config.on_tool_lifecycle_event.as_ref();
    let base_invocation = ToolInvocationRecord::new(
        format!("toolinv_{}", Uuid::new_v4()),
        tool_use.name.clone(),
        Some(tool_use.id.clone()),
    );

    if let Some(cb) = callback {
        cb(
            "tool_invocation_detected",
            json!({
                "invocation_id": base_invocation.invocation_id,
                "tool_name": base_invocation.tool_name,
                "tool_use_id": base_invocation.tool_use_id,
                "phase": base_invocation.phase,
                "permission_state": base_invocation.permission_state,
                "started_at_ms": base_invocation.started_at_ms,
                "finished_at_ms": base_invocation.finished_at_ms,
                "result_summary": base_invocation.result_summary,
                "error_summary": base_invocation.error_summary,
                "details": {
                    "tool_input": tool_use.input,
                    "source": "agentic_loop",
                }
            }),
        );
    }

    let tool = match tool_registry.get(&tool_use.name) {
        Some(t) => t,
        None => {
            let mut invocation = base_invocation;
            invocation.mark_phase(ToolInvocationPhase::Failed);
            invocation.set_error(format!("tool '{}' not found in registry", tool_use.name));
            if let Some(cb) = callback {
                cb(
                    "tool_execution_failed",
                    json!({
                        "invocation_id": invocation.invocation_id,
                        "tool_name": invocation.tool_name,
                        "tool_use_id": invocation.tool_use_id,
                        "phase": invocation.phase,
                        "permission_state": invocation.permission_state,
                        "started_at_ms": invocation.started_at_ms,
                        "finished_at_ms": invocation.finished_at_ms,
                        "result_summary": invocation.result_summary,
                        "error_summary": invocation.error_summary,
                        "details": {
                            "error": invocation.error_summary,
                            "stage": "tool_lookup",
                        }
                    }),
                );
            }
            return SingleToolExecutionOutcome::Completed {
                invocation,
                result: ToolResult::error(format!(
                    "tool '{}' not found in registry",
                    tool_use.name
                )),
            };
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
        invocation_id: Some(base_invocation.invocation_id.clone()),
    };

    match run_tool_use(
        tool,
        options,
        permission_checker,
        hook_registry,
        config.on_tool_lifecycle_event.as_ref(),
    ) {
        Ok(RunToolUseOutcome::Completed {
            mut invocation,
            result,
        }) => {
            invocation.tool_use_id = Some(tool_use.id.clone());
            SingleToolExecutionOutcome::Completed { invocation, result }
        }
        Ok(RunToolUseOutcome::AwaitingPermission(mut awaiting)) => {
            awaiting.invocation.tool_use_id = Some(tool_use.id.clone());
            SingleToolExecutionOutcome::AwaitingPermission(awaiting)
        }
        Err(err) => {
            let mut invocation = base_invocation;
            invocation.mark_phase(ToolInvocationPhase::Failed);
            invocation.set_error(err.to_string());
            SingleToolExecutionOutcome::Completed {
                invocation,
                result: ToolResult::error(format!("tool execution error: {err}")),
            }
        }
    }
}

fn is_tool_use_concurrency_safe(tool_use: &ParsedToolUse, tool_registry: &ToolRegistry) -> bool {
    tool_registry
        .get(&tool_use.name)
        .map(|tool| tool.is_concurrency_safe())
        .unwrap_or(false)
}

fn execute_tool_segment(
    segment: &[IndexedToolUse],
    tool_registry: &ToolRegistry,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    config: &AgenticLoopConfig,
) -> Vec<IndexedToolOutcome> {
    if segment.is_empty() {
        return Vec::new();
    }

    if segment.len() == 1 {
        let item = &segment[0];
        return vec![IndexedToolOutcome {
            index: item.index,
            tool_use_id: item.tool_use.id.clone(),
            outcome: execute_single_tool(
                item.tool_use.clone(),
                tool_registry,
                permission_checker,
                hook_registry,
                config,
            ),
        }];
    }

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(segment.len());
        for item in segment {
            handles.push(scope.spawn(move || IndexedToolOutcome {
                index: item.index,
                tool_use_id: item.tool_use.id.clone(),
                outcome: execute_single_tool(
                    item.tool_use.clone(),
                    tool_registry,
                    permission_checker,
                    hook_registry,
                    config,
                ),
            }));
        }

        let mut outcomes = handles
            .into_iter()
            .map(|handle| handle.join().expect("tool execution worker panicked"))
            .collect::<Vec<_>>();
        outcomes.sort_by_key(|item| item.index);
        outcomes
    })
}

pub fn resume_suspended_tool_invocation(
    suspension: &AgenticLoopSuspension,
    allowed: bool,
    tool_registry: &ToolRegistry,
    hook_registry: Option<&HookRegistry>,
    config: &AgenticLoopConfig,
) -> Result<Vec<InternalMsg>, AgenticLoopError> {
    let mut messages = suspension.messages.clone();
    let (mut invocation, tool_result) = if allowed {
        let tool = tool_registry
            .get(&suspension.suspended_tool.tool_name)
            .ok_or_else(|| {
                AgenticLoopError::AllToolsFailed {
                    internal_message: format!(
                        "tool '{}' not found in registry during resume",
                        suspension.suspended_tool.tool_name
                    ),
                }
            })?;
        let options = RunToolOptions {
            tool_name: suspension.suspended_tool.tool_name.clone(),
            input: suspension.suspended_tool.tool_input.clone(),
            context: ToolContext {
                session_id: config.session_id.clone(),
                trace_id: config.trace_id.clone(),
                metadata: json!({}),
            },
            invocation_id: Some(suspension.suspended_tool.invocation.invocation_id.clone()),
        };
        execute_tool_after_permission(
            tool,
            options,
            hook_registry,
            suspension.suspended_tool.invocation.clone(),
            config.on_tool_lifecycle_event.as_ref(),
        )
        .map_err(|error| AgenticLoopError::ToolExecution { error })?
    } else {
        let mut invocation = suspension.suspended_tool.invocation.clone();
        invocation.set_permission_state(crate::tools::execution::ToolPermissionState::Denied);
        invocation.mark_phase(ToolInvocationPhase::Failed);
        invocation.set_error(format!(
            "tool execution denied by user: {}",
            suspension.suspended_tool.permission_request.prompt
        ));
        if let Some(callback) = config.on_tool_lifecycle_event.as_ref() {
            callback(
                "tool_permission_resolved",
                json!({
                    "invocation_id": invocation.invocation_id,
                    "tool_name": invocation.tool_name,
                    "tool_use_id": invocation.tool_use_id,
                    "phase": invocation.phase,
                    "permission_state": invocation.permission_state,
                    "started_at_ms": invocation.started_at_ms,
                    "finished_at_ms": invocation.finished_at_ms,
                    "result_summary": invocation.result_summary,
                    "error_summary": invocation.error_summary,
                    "details": {
                        "decision": "deny",
                        "source": "resume",
                    }
                }),
            );
        }
        (
            invocation,
            ToolResult::error(format!(
                "tool execution denied by user: {}",
                suspension.suspended_tool.permission_request.prompt
            )),
        )
    };

    invocation.mark_phase(ToolInvocationPhase::ResultRecorded);
    if let Some(callback) = config.on_tool_lifecycle_event.as_ref() {
        callback(
            "tool_result_recorded",
            json!({
                "invocation_id": invocation.invocation_id,
                "tool_name": invocation.tool_name,
                "tool_use_id": invocation.tool_use_id,
                "phase": invocation.phase,
                "permission_state": invocation.permission_state,
                "started_at_ms": invocation.started_at_ms,
                "finished_at_ms": invocation.finished_at_ms,
                "result_summary": invocation.result_summary,
                "error_summary": invocation.error_summary,
                "details": {
                    "is_error": tool_result.is_error,
                }
            }),
        );
    }

    messages.push(build_tool_result_msg(
        &suspension.suspended_tool.tool_use_id,
        &tool_result,
        Some(&invocation),
    ));
    Ok(messages)
}

// ---------------------------------------------------------------------------
// 核心循环
// ---------------------------------------------------------------------------

/// 运行 Agentic Loop
///
/// 模型驱动的工具循环：模型调用工具 → 观察结果 → 继续推理，
/// 直到模型给出最终回答（不再请求工具）或达到迭代上限。
pub fn run_agentic_loop(
    provider: &dyn Provider,
    tool_registry: &ToolRegistry,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    initial_messages: Vec<InternalMsg>,
    config: &AgenticLoopConfig,
) -> Result<AgenticLoopOutcome, AgenticLoopError> {
    continue_agentic_loop(
        provider,
        tool_registry,
        permission_checker,
        hook_registry,
        initial_messages,
        0,
        0,
        config,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn continue_agentic_loop(
    provider: &dyn Provider,
    tool_registry: &ToolRegistry,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    mut messages: Vec<InternalMsg>,
    mut iterations: usize,
    mut compact_count: usize,
    config: &AgenticLoopConfig,
) -> Result<AgenticLoopOutcome, AgenticLoopError> {
    loop {
        if iterations >= config.max_iterations {
            let final_text = extract_last_assistant_text(&messages);
            return Ok(AgenticLoopOutcome::Completed(AgenticLoopResult {
                final_text,
                messages,
                iterations,
                hit_limit: true,
                compact_count,
            }));
        }

        ensure_tool_result_pairing(&mut messages);

        if let Some(ref compact_config) = config.compact {
            let result = compact_if_needed(&mut messages, compact_config);
            if result.did_compact {
                compact_count += 1;
            }
        }

        let api_msgs = normalize_messages_for_api(&messages);
        let chat_request = build_chat_request(
            &api_msgs,
            config.system_prompt.as_deref(),
            config.tool_definitions.as_deref(),
            &config.session_id,
        );

        let parsed = if config.streaming {
            let stream = provider
                .chat_stream(chat_request)
                .map_err(|error| AgenticLoopError::ApiCall { error })?;
            let (parsed, response_value) =
                crate::streaming::executor::consume_stream_events(
                    stream,
                    config.on_stream_event.as_deref(),
                )
                .map_err(|error| AgenticLoopError::ApiCall { error })?;
            messages.push(build_assistant_msg(&response_value));
            parsed
        } else {
            let response = provider
                .chat(chat_request)
                .map_err(|error| AgenticLoopError::ApiCall { error })?;
            let parsed = parse_chat_response(&response)?;
            messages.push(build_assistant_msg(&response.raw));
            parsed
        };

        if parsed.tool_uses.is_empty() {
            return Ok(AgenticLoopOutcome::Completed(AgenticLoopResult {
                final_text: parsed.text,
                messages,
                iterations: iterations + 1,
                hit_limit: false,
                compact_count,
            }));
        }

        let indexed_tool_uses = parsed
            .tool_uses
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, tool_use)| IndexedToolUse { index, tool_use })
            .collect::<Vec<_>>();

        let mut cursor = 0;
        while cursor < indexed_tool_uses.len() {
            let concurrency_safe =
                is_tool_use_concurrency_safe(&indexed_tool_uses[cursor].tool_use, tool_registry);
            let mut segment_end = cursor + 1;
            while segment_end < indexed_tool_uses.len()
                && is_tool_use_concurrency_safe(&indexed_tool_uses[segment_end].tool_use, tool_registry)
                    == concurrency_safe
            {
                segment_end += 1;
            }

            let segment = &indexed_tool_uses[cursor..segment_end];
            let outcomes = if concurrency_safe {
                execute_tool_segment(
                    segment,
                    tool_registry,
                    permission_checker,
                    hook_registry,
                    config,
                )
            } else {
                segment
                    .iter()
                    .map(|item| IndexedToolOutcome {
                        index: item.index,
                        tool_use_id: item.tool_use.id.clone(),
                        outcome: execute_single_tool(
                            item.tool_use.clone(),
                            tool_registry,
                            permission_checker,
                            hook_registry,
                            config,
                        ),
                    })
                    .collect::<Vec<_>>()
            };

            let earliest_permission = outcomes.iter().find_map(|item| match &item.outcome {
                SingleToolExecutionOutcome::AwaitingPermission(awaiting) => Some((
                    item.tool_use_id.clone(),
                    awaiting.clone(),
                )),
                _ => None,
            });

            if let Some((tool_use_id, awaiting)) = earliest_permission {
                return Ok(AgenticLoopOutcome::Suspended(Box::new(AgenticLoopSuspension {
                    suspended_tool: SuspendedToolInvocation {
                        tool_use_id,
                        tool_name: awaiting.invocation.tool_name.clone(),
                        tool_input: awaiting.input,
                        permission_request: awaiting.permission_request,
                        invocation: awaiting.invocation,
                    },
                    messages,
                    iterations,
                    compact_count,
                })));
            }

            for item in outcomes {
                match item.outcome {
                    SingleToolExecutionOutcome::Completed {
                        mut invocation,
                        result,
                    } => {
                        invocation.mark_phase(ToolInvocationPhase::ResultRecorded);
                        if let Some(callback) = config.on_tool_lifecycle_event.as_ref() {
                            callback(
                                "tool_result_recorded",
                                json!({
                                    "invocation_id": invocation.invocation_id,
                                    "tool_name": invocation.tool_name,
                                    "tool_use_id": invocation.tool_use_id,
                                    "phase": invocation.phase,
                                    "permission_state": invocation.permission_state,
                                    "started_at_ms": invocation.started_at_ms,
                                    "finished_at_ms": invocation.finished_at_ms,
                                    "result_summary": invocation.result_summary,
                                    "error_summary": invocation.error_summary,
                                    "details": {
                                        "is_error": result.is_error,
                                    }
                                }),
                            );
                        }
                        let tool_result_msg =
                            build_tool_result_msg(&item.tool_use_id, &result, Some(&invocation));
                        messages.push(tool_result_msg);
                    }
                    SingleToolExecutionOutcome::AwaitingPermission(_) => unreachable!(),
                }
            }

            cursor = segment_end;
        }

        iterations += 1;
    }
}

/// 从消息历史中提取最后一条 assistant 消息的文本
fn extract_last_assistant_text(messages: &[InternalMsg]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant)
        .map(|m| {
            m.blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatResponse, ProviderError};
    use crate::provider::types::StopReason;
    use crate::tools::definition::Tool;
    use crate::tools::result::{ToolError, ToolResult};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    // ---- Mock Provider ----

    struct MockProvider {
        responses: Vec<Value>,
        call_count: AtomicUsize,
    }

    impl MockProvider {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                responses,
                call_count: AtomicUsize::new(0),
            }
        }
    }

    fn value_to_response(raw: Value) -> ChatResponse {
        let content = raw.get("content").cloned().unwrap_or(Value::Null);
        let blocks = blocks_from_value(&content, None);
        let has_tool_use = blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        ChatResponse {
            content: blocks,
            stop_reason: if has_tool_use { StopReason::ToolUse } else { StopReason::EndTurn },
            usage: None,
            raw,
        }
    }

    impl Provider for MockProvider {
        fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            self.responses
                .get(idx)
                .cloned()
                .map(value_to_response)
                .ok_or_else(|| ProviderError::internal("no more mock responses"))
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
            let text = input.get("text").and_then(Value::as_str).unwrap_or("echo");
            Ok(ToolResult::text(format!("echoed: {text}")))
        }
    }

    fn make_config() -> AgenticLoopConfig {
        AgenticLoopConfig {
            max_iterations: 10,
            session_id: "test-session".into(),
            trace_id: "test-trace".into(),
            ..Default::default()
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
        let msg = build_tool_result_msg("tu_1", &result, None);
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.tool_use_id.as_deref(), Some("tu_1"));

        assert_eq!(msg.blocks.len(), 1);
        match &msg.blocks[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "tu_1");
                assert_eq!(content, &Value::String("hello result".into()));
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult block"),
        }
    }

    #[test]
    fn test_build_tool_result_msg_error() {
        let result = ToolResult::error("something went wrong");
        let msg = build_tool_result_msg("tu_2", &result, None);

        match &msg.blocks[0] {
            ContentBlock::ToolResult {
                is_error, content, ..
            } => {
                assert!(is_error);
                assert_eq!(content, &Value::String("something went wrong".into()));
            }
            _ => panic!("expected ToolResult block"),
        }
    }

    // ---- 循环测试 ----

    #[test]
    fn test_loop_no_tools() {
        // 模型直接返回文本，不请求工具 → 单轮返回
        let executor = MockProvider::new(vec![json!({
            "content": [{"type": "text", "text": "The answer is 42."}]
        })]);

        let registry = ToolRegistry::new();
        let config = make_config();
        let initial = vec![make_user_msg("What is the answer?")];

        let result =
            run_agentic_loop(&executor, &registry, None, None, initial, &config).unwrap();

        let AgenticLoopOutcome::Completed(result) = result else {
            panic!("loop should complete");
        };
        assert_eq!(result.final_text, "The answer is 42.");
        assert_eq!(result.iterations, 1);
        assert!(!result.hit_limit);
    }

    #[test]
    fn test_loop_one_tool_round() {
        // 第一次：模型请求调用 echo 工具
        // 第二次：模型返回最终文本
        let executor = MockProvider::new(vec![
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

        let config = make_config();
        let initial = vec![make_user_msg("Echo hello for me")];

        let result =
            run_agentic_loop(&executor, &registry, None, None, initial, &config).unwrap();

        let AgenticLoopOutcome::Completed(result) = result else {
            panic!("loop should complete");
        };
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
        let executor = MockProvider::new(responses);

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));


        let config = AgenticLoopConfig {
            max_iterations: 3,
            session_id: "test".into(),
            trace_id: "test".into(),
            ..Default::default()
        };
        let initial = vec![make_user_msg("Keep looping")];

        let result =
            run_agentic_loop(&executor, &registry, None, None, initial, &config).unwrap();

        let AgenticLoopOutcome::Completed(result) = result else {
            panic!("loop should complete");
        };
        assert!(result.hit_limit);
        assert_eq!(result.iterations, 3);
    }

    // ---- StreamingMockProvider ----

    struct StreamingMockProvider {
        event_sequences: Vec<Vec<StreamEvent>>,
        responses: Vec<Value>,
        streaming_call_count: AtomicUsize,
        sync_call_count: AtomicUsize,
    }

    impl StreamingMockProvider {
        fn from_sse(sse_sequences: Vec<Vec<String>>, responses: Vec<Value>) -> Self {
            let event_sequences = sse_sequences
                .into_iter()
                .map(|lines| {
                    lines
                        .into_iter()
                        .filter_map(|line| {
                            let data = line.strip_prefix("data: ").unwrap_or(&line);
                            crate::streaming::parse_sse_event(data)
                        })
                        .collect()
                })
                .collect();
            Self {
                event_sequences,
                responses,
                streaming_call_count: AtomicUsize::new(0),
                sync_call_count: AtomicUsize::new(0),
            }
        }
    }

    impl Provider for StreamingMockProvider {
        fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let idx = self.sync_call_count.fetch_add(1, Ordering::SeqCst);
            self.responses
                .get(idx)
                .cloned()
                .map(value_to_response)
                .ok_or_else(|| ProviderError::internal("no more sync mock responses"))
        }

        fn chat_stream(
            &self,
            _req: ChatRequest,
        ) -> Result<crate::provider::ChatStream, ProviderError> {
            let idx = self.streaming_call_count.fetch_add(1, Ordering::SeqCst);
            let events = self
                .event_sequences
                .get(idx)
                .cloned()
                .ok_or_else(|| ProviderError::internal("no more streaming mock sequences"))?;
            Ok(Box::new(events.into_iter().map(Ok)))
        }
    }

    fn sse(data: &str) -> String {
        format!("data: {data}")
    }

    // ---- 流式循环测试 ----

    #[test]
    fn test_streaming_loop_no_tools() {
        let sse_lines = vec![
            sse(r#"{"type":"message_start","message":{"id":"msg_s1"}}"#),
            sse(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Streaming "}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"answer!"}}"#,
            ),
            sse(r#"{"type":"content_block_stop","index":0}"#),
            sse(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#),
            sse(r#"{"type":"message_stop"}"#),
        ];

        let provider = StreamingMockProvider::from_sse(vec![sse_lines], vec![]);
        let registry = ToolRegistry::new();

        let config = AgenticLoopConfig {
            max_iterations: 10,
            session_id: "test-stream".into(),
            trace_id: "test-stream".into(),
            streaming: true,
            ..Default::default()
        };
        let initial = vec![make_user_msg("Hello streaming")];

        let result =
            run_agentic_loop(&provider, &registry, None, None, initial, &config).unwrap();

        let AgenticLoopOutcome::Completed(result) = result else {
            panic!("loop should complete");
        };
        assert_eq!(result.final_text, "Streaming answer!");
        assert_eq!(result.iterations, 1);
        assert!(!result.hit_limit);
    }

    #[test]
    fn test_streaming_loop_one_tool() {
        // 第一次：流式返回 text + tool_use
        let sse_round1 = vec![
            sse(r#"{"type":"message_start","message":{"id":"msg_s2"}}"#),
            sse(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me echo"}}"#,
            ),
            sse(r#"{"type":"content_block_stop","index":0}"#),
            sse(
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_s1","name":"echo","input":{}}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"text\""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":":\"stream\"}"}}"#,
            ),
            sse(r#"{"type":"content_block_stop","index":1}"#),
            sse(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#),
            sse(r#"{"type":"message_stop"}"#),
        ];

        // 第二次：流式返回最终文本
        let sse_round2 = vec![
            sse(r#"{"type":"message_start","message":{"id":"msg_s3"}}"#),
            sse(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done streaming!"}}"#,
            ),
            sse(r#"{"type":"content_block_stop","index":0}"#),
            sse(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#),
            sse(r#"{"type":"message_stop"}"#),
        ];

        let provider = StreamingMockProvider::from_sse(vec![sse_round1, sse_round2], vec![]);

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));


        let config = AgenticLoopConfig {
            max_iterations: 10,
            session_id: "test-stream".into(),
            trace_id: "test-stream".into(),
            streaming: true,
            ..Default::default()
        };
        let initial = vec![make_user_msg("Echo stream for me")];

        let result =
            run_agentic_loop(&provider, &registry, None, None, initial, &config).unwrap();

        let AgenticLoopOutcome::Completed(result) = result else {
            panic!("loop should complete");
        };
        assert_eq!(result.final_text, "Done streaming!");
        assert_eq!(result.iterations, 2);
        assert!(!result.hit_limit);

        // 验证消息历史：user → assistant(tool_use) → user(tool_result) → assistant(final)
        assert_eq!(result.messages.len(), 4);
        assert_eq!(result.messages[0].role, MessageRole::User);
        assert_eq!(result.messages[1].role, MessageRole::Assistant);
        assert_eq!(result.messages[2].role, MessageRole::User); // tool_result
        assert_eq!(result.messages[3].role, MessageRole::Assistant);
    }

    #[test]
    fn test_streaming_loop_with_callback() {
        let event_count = Arc::new(AtomicUsize::new(0));
        let count_clone = event_count.clone();
        let callback = move |_event: StreamEvent| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        };

        let sse_lines = vec![
            sse(r#"{"type":"message_start","message":{"id":"msg_cb"}}"#),
            sse(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
            ),
            sse(r#"{"type":"content_block_stop","index":0}"#),
            sse(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#),
            sse(r#"{"type":"message_stop"}"#),
        ];

        let provider = StreamingMockProvider::from_sse(vec![sse_lines], vec![]);
        let registry = ToolRegistry::new();

        let config = AgenticLoopConfig {
            max_iterations: 10,
            session_id: "test-cb".into(),
            trace_id: "test-cb".into(),
            streaming: true,
            on_stream_event: Some(Arc::new(callback)),
            ..Default::default()
        };
        let initial = vec![make_user_msg("test callback")];

        let result =
            run_agentic_loop(&provider, &registry, None, None, initial, &config).unwrap();

        let AgenticLoopOutcome::Completed(result) = result else {
            panic!("loop should complete");
        };
        assert_eq!(result.final_text, "hi");
        assert!(
            event_count.load(Ordering::SeqCst) > 0,
            "callback should have fired"
        );
    }
}
