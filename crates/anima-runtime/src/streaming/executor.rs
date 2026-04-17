//! 流式执行器
//!
//! 包含两部分：
//! 1. `StreamingToolExecutor` — 追踪流式响应中的 tool_use 块（原有）
//! 2. `StreamAccumulator` + `consume_sse_stream` — SSE 流消费核心函数（新增）

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use super::api_parser::parse_sse_event;
use super::types::{ContentBlock, ContentDelta, StreamEvent, TrackedToolState};
use crate::agent::runtime_error::{RuntimeErrorKind, RuntimeErrorStage};
use crate::agent::{TaskExecutorError, UnifiedStreamSource};
use crate::execution::agentic_loop::{ParsedResponse, ParsedToolUse};
use crate::tools::registry::ToolRegistry;

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeStreamEvent {
    MessageStarted {
        message_id: String,
    },
    ContentBlockStarted {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: ContentDelta,
    },
    ContentBlockStopped {
        index: usize,
    },
    MessageDelta {
        stop_reason: Option<String>,
    },
    MessageStopped,
    Ping,
    Error {
        error_type: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct StreamingFinalResult {
    pub parsed: ParsedResponse,
    pub response_value: Value,
}

impl RuntimeStreamEvent {
    fn from_stream_event(event: StreamEvent) -> Self {
        match event {
            StreamEvent::MessageStart { message_id } => Self::MessageStarted { message_id },
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => Self::ContentBlockStarted {
                index,
                content_block,
            },
            StreamEvent::ContentBlockDelta { index, delta } => {
                Self::ContentBlockDelta { index, delta }
            }
            StreamEvent::ContentBlockStop { index } => Self::ContentBlockStopped { index },
            StreamEvent::MessageDelta { stop_reason } => Self::MessageDelta { stop_reason },
            StreamEvent::MessageStop => Self::MessageStopped,
            StreamEvent::Ping => Self::Ping,
            StreamEvent::Error {
                error_type,
                message,
            } => Self::Error {
                error_type,
                message,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// StreamingToolExecutor（原有）
// ---------------------------------------------------------------------------

/// 流式工具执行器：追踪正在接收的 tool_use 块
#[derive(Debug)]
pub struct StreamingToolExecutor {
    /// 按 content block index 追踪的工具状态
    tracked: HashMap<usize, TrackedToolState>,
    /// 工具注册中心引用
    registry: Arc<ToolRegistry>,
}

impl StreamingToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            tracked: HashMap::new(),
            registry,
        }
    }

    /// 开始追踪一个新的 tool_use 块
    pub fn on_tool_use_start(&mut self, index: usize, id: String, name: String) {
        self.tracked.insert(
            index,
            TrackedToolState::ReceivingInput {
                id,
                name,
                accumulated_json: String::new(),
            },
        );
    }

    /// 追加 tool_use 输入的 JSON 增量
    pub fn on_input_delta(&mut self, index: usize, partial_json: &str) {
        if let Some(TrackedToolState::ReceivingInput {
            accumulated_json, ..
        }) = self.tracked.get_mut(&index)
        {
            accumulated_json.push_str(partial_json);
        }
    }

    /// 标记 tool_use 输入完成，解析完整 JSON
    ///
    /// JSON 解析失败时返回 None 并记录错误信息到 state
    pub fn on_tool_use_stop(&mut self, index: usize) -> Option<&TrackedToolState> {
        let state = self.tracked.get(&index)?;
        if let TrackedToolState::ReceivingInput {
            id,
            name,
            accumulated_json,
        } = state
        {
            let new_state = match serde_json::from_str::<Value>(accumulated_json) {
                Ok(input) => TrackedToolState::ReadyToExecute {
                    id: id.clone(),
                    name: name.clone(),
                    input,
                },
                Err(e) => TrackedToolState::ReadyToExecute {
                    id: id.clone(),
                    name: name.clone(),
                    input: json!({
                        "__parse_error": format!("malformed tool JSON: {e}"),
                        "__raw": accumulated_json.clone(),
                    }),
                },
            };
            self.tracked.insert(index, new_state);
        }
        self.tracked.get(&index)
    }

    /// 获取指定 index 的追踪状态
    pub fn get_state(&self, index: usize) -> Option<&TrackedToolState> {
        self.tracked.get(&index)
    }

    /// 获取工具注册中心
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// 列出所有追踪中的 tool index
    pub fn tracked_indices(&self) -> Vec<usize> {
        self.tracked.keys().copied().collect()
    }
}

// ---------------------------------------------------------------------------
// StreamAccumulator（新增）
// ---------------------------------------------------------------------------

/// 轻量工具 JSON 累积器，用于 `consume_sse_stream`
#[derive(Debug)]
struct ToolAccState {
    id: String,
    name: String,
    json_buf: String,
    complete: bool,
}

/// 轻量 SSE 流工具 JSON 累积器
///
/// 与 `StreamingToolExecutor` 不同，这是为一次性流消费设计的轻量结构，
/// 不追踪执行状态，只累积 JSON 并在流结束后一次性输出。
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    tools: HashMap<usize, ToolAccState>,
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 开始追踪一个 tool_use 块
    pub fn on_tool_start(&mut self, index: usize, id: String, name: String) {
        self.tools.insert(
            index,
            ToolAccState {
                id,
                name,
                json_buf: String::new(),
                complete: false,
            },
        );
    }

    /// 追加 JSON 增量
    pub fn on_input_delta(&mut self, index: usize, partial_json: &str) {
        if let Some(state) = self.tools.get_mut(&index) {
            state.json_buf.push_str(partial_json);
        }
    }

    /// 标记 tool_use 完成
    pub fn on_tool_stop(&mut self, index: usize) {
        if let Some(state) = self.tools.get_mut(&index) {
            state.complete = true;
        }
    }

    /// 排空所有已完成的工具，返回 `(id, name, parsed_input)` 列表（按 index 排序）
    ///
    /// JSON 解析失败时返回错误，而非静默使用 Null
    pub fn drain_ready(&mut self) -> Result<Vec<(String, String, Value)>, String> {
        let mut indexed: Vec<(usize, (String, String, Value))> = Vec::new();
        let indices: Vec<usize> = self.tools.keys().copied().collect();
        for idx in indices {
            if self.tools.get(&idx).is_some_and(|s| s.complete) {
                if let Some(state) = self.tools.remove(&idx) {
                    let input: Value = serde_json::from_str(&state.json_buf).map_err(|e| {
                        format!(
                            "malformed tool JSON for '{}' (id={}): {e} — raw: {:?}",
                            state.name, state.id, state.json_buf
                        )
                    })?;
                    indexed.push((idx, (state.id, state.name, input)));
                }
            }
        }
        indexed.sort_by_key(|(idx, _)| *idx);
        Ok(indexed.into_iter().map(|(_, v)| v).collect())
    }
}

// ---------------------------------------------------------------------------
// consume_sse_stream（新增）
// ---------------------------------------------------------------------------

/// 消费 SSE 流，返回 `(ParsedResponse, 等效 response Value)`
///
/// 逻辑：
/// 1. 逐行读取 SSE（跳过非 `data:` 行）
/// 2. `parse_sse_event(data)` 解析为 `StreamEvent`
/// 3. 触发 `on_event` 回调（UI 更新）
/// 4. `TextDelta` → 累积文本
/// 5. `ToolUse` start / delta / stop → `StreamAccumulator` 追踪
/// 6. 流结束后 `drain_ready()` → 组装 `ParsedResponse` 和等效 `response_value`
pub fn consume_sse_stream(
    lines: UnifiedStreamSource,
    on_event: Option<&(dyn Fn(StreamEvent) + Send + Sync)>,
) -> Result<(ParsedResponse, Value), TaskExecutorError> {
    let runtime_callback = |event: RuntimeStreamEvent| {
        if let Some(cb) = on_event {
            cb(runtime_stream_event_to_stream_event(&event));
        }
    };
    let final_result = consume_runtime_stream(lines, Some(&runtime_callback))?;
    Ok((final_result.parsed, final_result.response_value))
}

pub fn consume_runtime_stream<F>(
    lines: UnifiedStreamSource,
    on_event: Option<&F>,
) -> Result<StreamingFinalResult, TaskExecutorError>
where
    F: Fn(RuntimeStreamEvent) + Send + Sync + ?Sized,
{
    let mut text_buf = String::new();
    let mut acc = StreamAccumulator::new();
    let mut message_id = String::new();
    let mut stop_reason: Option<String> = None;

    for line_result in lines {
        let line = line_result?;

        // 跳过空行和非 data 行
        let data = if let Some(stripped) = line.strip_prefix("data: ") {
            stripped
        } else if let Some(stripped) = line.strip_prefix("data:") {
            stripped
        } else {
            continue;
        };

        // 跳过 [DONE] 标记
        if data.trim() == "[DONE]" {
            continue;
        }

        // 解析 SSE 事件
        let event = match parse_sse_event(data) {
            Some(ev) => ev,
            None => continue,
        };

        // 触发回调
        if let Some(cb) = on_event.as_ref() {
            cb(RuntimeStreamEvent::from_stream_event(event.clone()));
        }

        // 处理事件
        match &event {
            StreamEvent::MessageStart { message_id: id } => {
                message_id = id.clone();
            }
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => match content_block {
                ContentBlock::ToolUse { id, name, .. } => {
                    acc.on_tool_start(*index, id.clone(), name.clone());
                }
                ContentBlock::Text { text } => {
                    text_buf.push_str(text);
                }
            },
            StreamEvent::ContentBlockDelta { index, delta } => match delta {
                ContentDelta::TextDelta { text } => {
                    text_buf.push_str(text);
                }
                ContentDelta::InputJsonDelta { partial_json } => {
                    acc.on_input_delta(*index, partial_json);
                }
            },
            StreamEvent::ContentBlockStop { index } => {
                acc.on_tool_stop(*index);
            }
            StreamEvent::MessageDelta {
                stop_reason: sr, ..
            } => {
                stop_reason = sr.clone();
            }
            StreamEvent::Error {
                error_type,
                message,
            } => {
                return Err(TaskExecutorError::new(
                    RuntimeErrorKind::UpstreamStreamFailed,
                    RuntimeErrorStage::PlanExecute,
                    format!("stream error [{error_type}]: {message}"),
                ));
            }
            _ => {}
        }
    }

    // 排空累积的工具调用
    let tools = acc.drain_ready().map_err(|internal_message| {
        TaskExecutorError::new(
            RuntimeErrorKind::ResponseParseFailed,
            RuntimeErrorStage::PlanExecute,
            internal_message,
        )
    })?;

    // 组装 ParsedResponse
    let tool_uses: Vec<ParsedToolUse> = tools
        .iter()
        .map(|(id, name, input)| ParsedToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        })
        .collect();

    let parsed = ParsedResponse {
        text: text_buf.clone(),
        tool_uses,
    };

    // 组装等效的 response Value（用于 build_assistant_msg）
    let mut content_parts: Vec<Value> = Vec::new();
    if !text_buf.is_empty() {
        content_parts.push(json!({"type": "text", "text": text_buf}));
    }
    for (id, name, input) in &tools {
        content_parts.push(json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
        }));
    }

    let response_value = json!({
        "id": message_id,
        "content": content_parts,
        "stop_reason": stop_reason,
    });

    Ok(StreamingFinalResult {
        parsed,
        response_value,
    })
}

fn runtime_stream_event_to_stream_event(event: &RuntimeStreamEvent) -> StreamEvent {
    match event {
        RuntimeStreamEvent::MessageStarted { message_id } => StreamEvent::MessageStart {
            message_id: message_id.clone(),
        },
        RuntimeStreamEvent::ContentBlockStarted {
            index,
            content_block,
        } => StreamEvent::ContentBlockStart {
            index: *index,
            content_block: content_block.clone(),
        },
        RuntimeStreamEvent::ContentBlockDelta { index, delta } => StreamEvent::ContentBlockDelta {
            index: *index,
            delta: delta.clone(),
        },
        RuntimeStreamEvent::ContentBlockStopped { index } => {
            StreamEvent::ContentBlockStop { index: *index }
        }
        RuntimeStreamEvent::MessageDelta { stop_reason } => StreamEvent::MessageDelta {
            stop_reason: stop_reason.clone(),
        },
        RuntimeStreamEvent::MessageStopped => StreamEvent::MessageStop,
        RuntimeStreamEvent::Ping => StreamEvent::Ping,
        RuntimeStreamEvent::Error {
            error_type,
            message,
        } => StreamEvent::Error {
            error_type: error_type.clone(),
            message: message.clone(),
        },
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ---- StreamAccumulator 测试 ----

    #[test]
    fn test_stream_accumulator_single_tool() {
        let mut acc = StreamAccumulator::new();
        acc.on_tool_start(1, "tu_1".into(), "bash".into());
        acc.on_input_delta(1, r#"{"com"#);
        acc.on_input_delta(1, r#"mand":"ls"}"#);
        acc.on_tool_stop(1);

        let tools = acc.drain_ready().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "tu_1");
        assert_eq!(tools[0].1, "bash");
        assert_eq!(tools[0].2, json!({"command": "ls"}));
    }

    #[test]
    fn test_stream_accumulator_multiple_tools_ordered() {
        let mut acc = StreamAccumulator::new();
        acc.on_tool_start(2, "tu_2".into(), "read".into());
        acc.on_tool_start(1, "tu_1".into(), "bash".into());
        acc.on_input_delta(1, r#"{"cmd":"a"}"#);
        acc.on_input_delta(2, r#"{"path":"b"}"#);
        acc.on_tool_stop(1);
        acc.on_tool_stop(2);

        let tools = acc.drain_ready().unwrap();
        assert_eq!(tools.len(), 2);
        // 按 index 排序: 1 < 2
        assert_eq!(tools[0].0, "tu_1");
        assert_eq!(tools[1].0, "tu_2");
    }

    #[test]
    fn test_stream_accumulator_incomplete_not_drained() {
        let mut acc = StreamAccumulator::new();
        acc.on_tool_start(0, "tu_1".into(), "bash".into());
        acc.on_input_delta(0, r#"{"partial"#);
        // 没有 on_tool_stop

        let tools = acc.drain_ready().unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_stream_accumulator_malformed_json_returns_error() {
        let mut acc = StreamAccumulator::new();
        acc.on_tool_start(0, "tu_bad".into(), "bash".into());
        acc.on_input_delta(0, r#"{"broken"#);
        acc.on_tool_stop(0);

        let result = acc.drain_ready();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("malformed tool JSON"));
        assert!(err.contains("bash"));
    }

    // ---- consume_sse_stream 测试 ----

    fn make_lines(raw: &[&str]) -> UnifiedStreamSource {
        let lines: Vec<Result<String, TaskExecutorError>> =
            raw.iter().map(|s| Ok(s.to_string())).collect();
        Box::new(lines.into_iter())
    }

    #[test]
    fn test_consume_text_only() {
        let lines = make_lines(&[
            r#"data: {"type":"message_start","message":{"id":"msg_1"}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world!"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);

        let (parsed, value) = consume_sse_stream(lines, None).unwrap();
        assert_eq!(parsed.text, "Hello world!");
        assert!(parsed.tool_uses.is_empty());
        assert_eq!(value["id"], "msg_1");
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][0]["text"], "Hello world!");
    }

    #[test]
    fn test_consume_with_tool_use() {
        let lines = make_lines(&[
            r#"data: {"type":"message_start","message":{"id":"msg_2"}}"#,
            // text block
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me run"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            // tool_use block (JSON 分块)
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"bash","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"com"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"mand\":\"ls\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);

        let (parsed, value) = consume_sse_stream(lines, None).unwrap();
        assert_eq!(parsed.text, "Let me run");
        assert_eq!(parsed.tool_uses.len(), 1);
        assert_eq!(parsed.tool_uses[0].id, "tu_1");
        assert_eq!(parsed.tool_uses[0].name, "bash");
        assert_eq!(parsed.tool_uses[0].input, json!({"command": "ls"}));

        // 验证 response value 结构
        assert_eq!(value["content"].as_array().unwrap().len(), 2);
        assert_eq!(value["content"][1]["type"], "tool_use");
    }

    #[test]
    fn test_consume_stream_error() {
        let lines = make_lines(&[
            r#"data: {"type":"message_start","message":{"id":"msg_3"}}"#,
            r#"data: {"type":"error","error":{"type":"overloaded","message":"server busy"}}"#,
        ]);

        let result = consume_sse_stream(lines, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().internal_message.contains("overloaded"));
    }

    #[test]
    fn test_on_event_callback() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        let callback = move |_event: StreamEvent| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        };

        let lines = make_lines(&[
            r#"data: {"type":"message_start","message":{"id":"msg_4"}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);

        let _ = consume_sse_stream(lines, Some(&callback)).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn test_consume_skips_non_data_lines() {
        let lines = make_lines(&[
            "",
            ": comment line",
            "event: message",
            r#"data: {"type":"message_start","message":{"id":"msg_5"}}"#,
            "",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_stop"}"#,
            "data: [DONE]",
        ]);

        let (parsed, _) = consume_sse_stream(lines, None).unwrap();
        assert_eq!(parsed.text, "ok");
    }
}
