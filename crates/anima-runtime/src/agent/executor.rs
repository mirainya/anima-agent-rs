use super::runtime_error::{RuntimeError, RuntimeErrorKind, RuntimeErrorStage};
use anima_sdk::{facade::Client as SdkClient, messages, sessions};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

pub type TaskExecutorError = RuntimeError;
pub type UnifiedStreamLine = Result<String, TaskExecutorError>;
pub type UnifiedStreamSource = Box<dyn Iterator<Item = UnifiedStreamLine>>;

pub trait TaskExecutor: Send + Sync {
    fn send_prompt(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, TaskExecutorError>;

    fn create_session(&self, client: &SdkClient) -> Result<Value, TaskExecutorError>;

    /// 流式发送 prompt，返回统一流输入。
    ///
    /// 默认返回 Err（不支持流式）。实现方需要返回可被 streaming 层消费的
    /// 逐行 SSE/兼容行迭代器，而不是暴露底层 transport response。
    fn send_prompt_streaming(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<UnifiedStreamSource, TaskExecutorError> {
        Err(RuntimeError::new(
            RuntimeErrorKind::TaskExecutionFailed,
            RuntimeErrorStage::PlanExecute,
            "streaming not supported",
        ))
    }
}

#[derive(Debug, Default)]
pub struct SdkTaskExecutor;

fn session_create_error(error: impl std::fmt::Display) -> TaskExecutorError {
    RuntimeError::new(
        RuntimeErrorKind::SessionCreateFailed,
        RuntimeErrorStage::SessionCreate,
        error.to_string(),
    )
}

fn plan_execute_error(error: impl std::fmt::Display) -> TaskExecutorError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let kind = if lower.contains("request timeout")
        || lower.contains("408 request timeout")
        || lower.contains("timed out")
        || lower.contains("timeout")
    {
        RuntimeErrorKind::UpstreamTimeout
    } else if lower.contains("empty_stream")
        || lower.contains("upstream stream closed before first payload")
        || lower.contains("stream disconnected before completion")
        || lower.contains("stream closed before response.completed")
    {
        RuntimeErrorKind::UpstreamStreamFailed
    } else {
        RuntimeErrorKind::TaskExecutionFailed
    };

    RuntimeError::new(kind, RuntimeErrorStage::PlanExecute, message)
}

impl TaskExecutor for SdkTaskExecutor {
    fn send_prompt(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, TaskExecutorError> {
        let response = messages::send_prompt(client, session_id, content, None)
            .map_err(plan_execute_error)?;
        wait_for_message_completion(client, session_id, response)
    }

    fn create_session(&self, client: &SdkClient) -> Result<Value, TaskExecutorError> {
        sessions::create_session(client, None).map_err(session_create_error)
    }

    fn send_prompt_streaming(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<UnifiedStreamSource, TaskExecutorError> {
        let event_lines = open_opencode_event_stream(client)?;
        let message_result_rx =
            spawn_message_request(client.clone(), session_id.to_string(), content);

        Ok(Box::new(OpenCodeEventAdapter::new(
            session_id.to_string(),
            message_result_rx,
            event_lines,
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PartKind {
    Text,
    ToolUse { name: String },
}

struct OpenCodeEventAdapter {
    target_session_id: String,
    target_user_message_id: Option<String>,
    message_id_rx: Receiver<MessageRequestResult>,
    target_assistant_message_id: Option<String>,
    lines: UnifiedStreamSource,
    buffered: VecDeque<String>,
    part_indices: HashMap<String, usize>,
    part_kinds: HashMap<String, PartKind>,
    started_parts: HashSet<String>,
    stopped_parts: HashSet<String>,
    message_started: bool,
    message_stopped: bool,
    stop_reason: Option<String>,
    eof_flushed: bool,
    upstream_error: Option<TaskExecutorError>,
}

impl OpenCodeEventAdapter {
    fn new(
        target_session_id: String,
        message_id_rx: Receiver<MessageRequestResult>,
        lines: UnifiedStreamSource,
    ) -> Self {
        Self {
            target_session_id,
            target_user_message_id: None,
            message_id_rx,
            target_assistant_message_id: None,
            lines,
            buffered: VecDeque::new(),
            part_indices: HashMap::new(),
            part_kinds: HashMap::new(),
            started_parts: HashSet::new(),
            stopped_parts: HashSet::new(),
            message_started: false,
            message_stopped: false,
            stop_reason: None,
            eof_flushed: false,
            upstream_error: None,
        }
    }

    fn queue_sse(&mut self, payload: Value) {
        self.buffered.push_back(format!("data: {payload}"));
    }

    fn target_message_id(&self) -> Option<&str> {
        self.target_assistant_message_id.as_deref()
    }

    fn poll_target_user_message_id(&mut self) {
        if self.target_user_message_id.is_some() || self.upstream_error.is_some() {
            return;
        }
        match self.message_id_rx.try_recv() {
            Ok(Ok(message_id)) => self.target_user_message_id = Some(message_id),
            Ok(Err(err)) => self.upstream_error = Some(err),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.upstream_error =
                    Some(RuntimeError::new(
                        RuntimeErrorKind::TaskExecutionFailed,
                        RuntimeErrorStage::PlanExecute,
                        "streaming protocol error: message sender thread disconnected",
                    ))
            }
        }
    }

    fn ensure_message_started(&mut self) {
        if self.message_started {
            return;
        }
        let Some(message_id) = self.target_message_id().map(ToString::to_string) else {
            return;
        };
        self.message_started = true;
        self.queue_sse(json!({
            "type": "message_start",
            "message": { "id": message_id }
        }));
    }

    fn part_index(&mut self, part_id: &str) -> usize {
        let next_index = self.part_indices.len();
        *self
            .part_indices
            .entry(part_id.to_string())
            .or_insert(next_index)
    }

    fn ensure_part_started(&mut self, part_id: &str, kind: PartKind) {
        self.ensure_message_started();
        self.part_kinds
            .entry(part_id.to_string())
            .or_insert_with(|| kind.clone());
        if !self.started_parts.insert(part_id.to_string()) {
            return;
        }

        let index = self.part_index(part_id);
        let payload = match self.part_kinds.get(part_id).cloned().unwrap_or(kind) {
            PartKind::Text => json!({
                "type": "content_block_start",
                "index": index,
                "content_block": { "type": "text", "text": "" }
            }),
            PartKind::ToolUse { name } => json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {
                    "type": "tool_use",
                    "id": part_id,
                    "name": name,
                    "input": {}
                }
            }),
        };
        self.queue_sse(payload);
    }

    fn stop_part(&mut self, part_id: &str) {
        if !self.started_parts.contains(part_id) || self.stopped_parts.contains(part_id) {
            return;
        }
        self.stopped_parts.insert(part_id.to_string());
        let index = self.part_index(part_id);
        self.queue_sse(json!({
            "type": "content_block_stop",
            "index": index
        }));
    }

    fn stop_all_parts(&mut self) {
        let part_ids: Vec<String> = self.started_parts.iter().cloned().collect();
        for part_id in part_ids {
            self.stop_part(&part_id);
        }
    }

    fn stop_message_if_needed(&mut self) {
        if self.message_stopped || !self.message_started {
            return;
        }
        self.stop_all_parts();
        self.queue_sse(json!({
            "type": "message_delta",
            "delta": { "stop_reason": self.stop_reason }
        }));
        self.queue_sse(json!({ "type": "message_stop" }));
        self.message_stopped = true;
    }

    fn flush_eof_once(&mut self) {
        if self.eof_flushed {
            return;
        }
        self.eof_flushed = true;
        self.stop_message_if_needed();
    }

    fn handle_message_updated(&mut self, raw: &Value) {
        if !matches_target_session(raw, &self.target_session_id) {
            return;
        }

        let Some(message_id) = extract_message_id(raw) else {
            return;
        };
        let role = extract_role(raw).unwrap_or_default();
        let parent_id = extract_parent_message_id(raw);
        if self.target_user_message_id.is_none() && role.eq_ignore_ascii_case("user") {
            adapter_debug(
                "user.locked.from_event",
                &format!(
                    "user_message_id={} session_id={}",
                    message_id, self.target_session_id
                ),
            );
            self.target_user_message_id = Some(message_id.clone());
        }
        adapter_debug(
            "message.updated",
            &format!(
                "session_id={} message_id={} role={} parent_id={:?} target_user_message_id={:?} target_assistant_message_id={:?}",
                self.target_session_id,
                message_id,
                role,
                parent_id,
                self.target_user_message_id,
                self.target_assistant_message_id,
            ),
        );

        let is_same_parent_assistant = role.eq_ignore_ascii_case("assistant")
            && self
                .target_user_message_id
                .as_deref()
                .zip(parent_id.as_deref())
                .is_some_and(|(user_message_id, parent_id)| user_message_id == parent_id);

        if self.target_assistant_message_id.is_none() && is_same_parent_assistant {
            adapter_debug(
                "assistant.locked",
                &format!(
                    "assistant_message_id={} parent_id={:?} user_message_id={:?}",
                    message_id, parent_id, self.target_user_message_id,
                ),
            );
            self.target_assistant_message_id = Some(message_id.clone());
        } else if is_same_parent_assistant
            && self.target_message_id() != Some(message_id.as_str())
            && self
                .stop_reason
                .as_deref()
                .is_some_and(is_tool_calls_stop_reason)
        {
            adapter_debug(
                "assistant.rollover",
                &format!(
                    "previous_assistant_message_id={:?} next_assistant_message_id={} parent_id={:?}",
                    self.target_assistant_message_id,
                    message_id,
                    parent_id,
                ),
            );
            self.stop_all_parts();
            self.target_assistant_message_id = Some(message_id.clone());
            self.stop_reason = None;
        }

        if self.target_message_id() != Some(message_id.as_str()) {
            return;
        }

        self.ensure_message_started();
        if let Some(stop_reason) = extract_stop_reason(raw) {
            if is_tool_calls_stop_reason(&stop_reason) {
                self.stop_reason = Some(stop_reason);
                self.stop_all_parts();
                return;
            }

            self.stop_reason = Some(stop_reason);
            self.stop_message_if_needed();
            return;
        }
        if let Some(status) = extract_status(raw) {
            if is_terminal_status(&status) {
                self.stop_message_if_needed();
            }
        }
    }

    fn handle_message_part_updated(&mut self, raw: &Value) {
        let Some(target_message_id) = self.target_message_id().map(ToString::to_string) else {
            return;
        };
        if !matches_target_message(raw, &self.target_session_id, &target_message_id) {
            return;
        }
        let Some(part_id) = extract_part_id(raw) else {
            return;
        };
        let kind = infer_part_kind(raw).unwrap_or(PartKind::Text);
        self.part_kinds.insert(part_id.clone(), kind.clone());
        self.ensure_part_started(&part_id, kind);

        if let Some(status) = extract_status(raw) {
            if is_terminal_status(&status) {
                self.stop_part(&part_id);
            }
        }
    }

    fn handle_message_part_delta(&mut self, raw: &Value) {
        let Some(target_message_id) = self.target_message_id().map(ToString::to_string) else {
            return;
        };
        if !matches_target_message(raw, &self.target_session_id, &target_message_id) {
            return;
        }
        let Some(part_id) = extract_part_id(raw) else {
            return;
        };
        let field = extract_field(raw).unwrap_or_default();
        let delta = extract_delta(raw).unwrap_or_default();
        let kind = if is_text_field(&field) {
            PartKind::Text
        } else {
            let name = self
                .part_kinds
                .get(&part_id)
                .and_then(|kind| match kind {
                    PartKind::ToolUse { name } => Some(name.clone()),
                    PartKind::Text => None,
                })
                .or_else(|| extract_tool_name(raw))
                .unwrap_or_else(|| "tool".to_string());
            PartKind::ToolUse { name }
        };

        self.ensure_part_started(&part_id, kind);
        let index = self.part_index(&part_id);
        let payload = if is_text_field(&field) {
            json!({
                "type": "content_block_delta",
                "index": index,
                "delta": {
                    "type": "text_delta",
                    "text": delta
                }
            })
        } else {
            json!({
                "type": "content_block_delta",
                "index": index,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": delta
                }
            })
        };
        self.queue_sse(payload);
    }

    fn handle_session_status(&mut self, raw: &Value) {
        if !matches_target_session(raw, &self.target_session_id) {
            return;
        }

        let status = extract_status(raw).unwrap_or_default();
        if status.eq_ignore_ascii_case("error") || status.eq_ignore_ascii_case("failed") {
            let message = extract_error_message(raw)
                .unwrap_or_else(|| "upstream session reported error".to_string());
            self.queue_sse(json!({
                "type": "error",
                "error": {
                    "type": if status.is_empty() { "session_error" } else { &status },
                    "message": message
                }
            }));
            self.message_stopped = true;
            return;
        }

        if is_terminal_status(&status) {
            self.stop_reason = self.stop_reason.take().or_else(|| Some(status.clone()));
            self.stop_message_if_needed();
        }
    }

    fn handle_raw_event(&mut self, raw: &Value) {
        let Some(event_type) = extract_event_type(raw) else {
            return;
        };

        match event_type {
            "message.updated" => self.handle_message_updated(raw),
            "message.part.updated" => self.handle_message_part_updated(raw),
            "message.part.delta" => self.handle_message_part_delta(raw),
            "session.status" => self.handle_session_status(raw),
            _ => {}
        }
    }
}

impl Iterator for OpenCodeEventAdapter {
    type Item = UnifiedStreamLine;

    fn next(&mut self) -> Option<Self::Item> {
        let mut event_data_lines: Vec<String> = Vec::new();

        loop {
            if let Some(line) = self.buffered.pop_front() {
                return Some(Ok(line));
            }

            self.poll_target_user_message_id();
            if let Some(error) = self.upstream_error.take() {
                return Some(Err(error));
            }

            match self.lines.next() {
                Some(Ok(line)) => {
                    adapter_debug("raw-line", &line);
                    let trimmed_line = line.trim_end();
                    if trimmed_line.is_empty() {
                        if event_data_lines.is_empty() {
                            continue;
                        }
                        let payload = event_data_lines.join("\n");
                        event_data_lines.clear();
                        let trimmed = payload.trim();
                        adapter_debug("event-frame", trimmed);
                        if trimmed.is_empty() || trimmed == "[DONE]" {
                            continue;
                        }
                        let raw: Value = match serde_json::from_str(trimmed) {
                            Ok(value) => value,
                            Err(err) => {
                                adapter_debug(
                                    "event-parse-error",
                                    &format!("payload={trimmed} error={err}"),
                                );
                                continue;
                            }
                        };
                        self.handle_raw_event(&raw);
                        if let Some(line) = self.buffered.pop_front() {
                            return Some(Ok(line));
                        }
                        if self.message_stopped {
                            return None;
                        }
                        continue;
                    }

                    if let Some(stripped) = trimmed_line.strip_prefix("data: ") {
                        event_data_lines.push(stripped.to_string());
                    } else if let Some(stripped) = trimmed_line.strip_prefix("data:") {
                        event_data_lines.push(stripped.to_string());
                    }
                }
                Some(Err(err)) => return Some(Err(err)),
                None => {
                    if !event_data_lines.is_empty() {
                        let payload = event_data_lines.join("\n");
                        let trimmed = payload.trim();
                        adapter_debug("event-frame-eof", trimmed);
                        if !trimmed.is_empty() && trimmed != "[DONE]" {
                            match serde_json::from_str::<Value>(trimmed) {
                                Ok(raw) => self.handle_raw_event(&raw),
                                Err(err) => adapter_debug(
                                    "event-parse-error-eof",
                                    &format!("payload={trimmed} error={err}"),
                                ),
                            }
                        }
                    }
                    self.flush_eof_once();
                    if let Some(line) = self.buffered.pop_front() {
                        return Some(Ok(line));
                    }
                    return None;
                }
            }
        }
    }
}

fn open_opencode_event_stream(client: &SdkClient) -> Result<UnifiedStreamSource, TaskExecutorError> {
    let event_response = messages::subscribe_event_stream(client, None, None)
        .map_err(plan_execute_error)?;
    let lines = BufReader::new(event_response)
        .lines()
        .map(|r| r.map_err(plan_execute_error));
    Ok(Box::new(lines))
}

type MessageRequestResult = Result<String, TaskExecutorError>;

const MESSAGE_COMPLETION_POLL_ATTEMPTS: usize = 480;
const MESSAGE_COMPLETION_POLL_INTERVAL_MS: u64 = 250;

fn wait_for_message_completion(
    client: &SdkClient,
    session_id: &str,
    initial_response: Value,
) -> Result<Value, TaskExecutorError> {
    let message_id = extract_message_id_from_response(&initial_response)?;
    let initial_message = messages::get_message(client, session_id, &message_id, None)
        .map_err(plan_execute_error)?;
    if message_has_completed_content(&initial_message) {
        return Ok(initial_message);
    }

    for _ in 0..MESSAGE_COMPLETION_POLL_ATTEMPTS {
        std::thread::sleep(std::time::Duration::from_millis(
            MESSAGE_COMPLETION_POLL_INTERVAL_MS,
        ));
        let message = messages::get_message(client, session_id, &message_id, None)
            .map_err(plan_execute_error)?;
        if message_has_completed_content(&message) {
            return Ok(message);
        }
    }

    Err(RuntimeError::new(
        RuntimeErrorKind::UpstreamTimeout,
        RuntimeErrorStage::PlanExecute,
        format!(
            "timed out waiting for completed message {} in session {}",
            message_id, session_id
        ),
    ))
}

fn message_has_completed_content(value: &Value) -> bool {
    let parts = value
        .get("parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_content = parts.iter().any(|part| {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
        matches!(
            part_type,
            "text" | "reasoning" | "tool" | "tool_use" | "patch"
        )
    });
    if !has_content {
        return false;
    }

    let finish = value
        .get("info")
        .and_then(|info| info.get("finish"))
        .and_then(Value::as_str);
    let status = value
        .get("info")
        .and_then(|info| info.get("status"))
        .and_then(Value::as_str);

    matches!(finish, Some("stop" | "tool-calls" | "tool_calls"))
        || matches!(status, Some("completed" | "complete" | "done" | "stopped"))
}

fn spawn_message_request(
    client: SdkClient,
    session_id: String,
    content: Value,
) -> Receiver<MessageRequestResult> {
    let (message_result_tx, message_result_rx): (
        Sender<MessageRequestResult>,
        Receiver<MessageRequestResult>,
    ) = mpsc::channel();
    thread::spawn(move || {
        let result = messages::send_prompt(&client, &session_id, content, None)
            .map_err(plan_execute_error)
            .and_then(|response| extract_message_id_from_response(&response));
        let _ = message_result_tx.send(result);
    });
    message_result_rx
}

fn extract_message_id_from_response(value: &Value) -> Result<String, TaskExecutorError> {
    string_by_pointers(
        value,
        &[
            "/id",
            "/info/id",
            "/messageID",
            "/message_id",
            "/message/id",
            "/data/id",
            "/data/messageID",
            "/data/message/id",
        ],
    )
    .map(ToString::to_string)
    .ok_or_else(|| {
        RuntimeError::new(
            RuntimeErrorKind::TaskExecutionFailed,
            RuntimeErrorStage::PlanExecute,
            format!("streaming protocol error: POST /message response missing message id: {value}"),
        )
    })
}

fn extract_event_type(value: &Value) -> Option<&str> {
    string_by_pointers(value, &["/type", "/event"])
}

fn matches_target_session(value: &Value, session_id: &str) -> bool {
    string_by_pointers(
        value,
        &[
            "/sessionID",
            "/session_id",
            "/properties/sessionID",
            "/properties/session_id",
            "/properties/info/sessionID",
            "/properties/part/sessionID",
            "/session/id",
            "/properties/session/id",
            "/properties/info/session/id",
        ],
    )
    .is_some_and(|candidate| candidate == session_id)
}

fn matches_target_message(value: &Value, session_id: &str, message_id: &str) -> bool {
    matches_target_session(value, session_id)
        && extract_message_id(value).is_some_and(|candidate| candidate == message_id)
}

fn extract_message_id(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/messageID",
            "/message_id",
            "/properties/messageID",
            "/properties/message_id",
            "/properties/info/id",
            "/properties/part/messageID",
            "/message/id",
            "/properties/message/id",
        ],
    )
    .map(ToString::to_string)
}

fn extract_parent_message_id(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/parentID",
            "/parent_id",
            "/properties/parentID",
            "/properties/parent_id",
            "/properties/info/parentID",
            "/message/parentID",
            "/properties/message/parentID",
        ],
    )
    .map(ToString::to_string)
}

fn extract_role(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/role",
            "/properties/role",
            "/properties/info/role",
            "/message/role",
            "/properties/message/role",
        ],
    )
    .map(ToString::to_string)
}

fn extract_part_id(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/partID",
            "/part_id",
            "/properties/partID",
            "/properties/part_id",
            "/part/id",
            "/properties/part/id",
        ],
    )
    .map(ToString::to_string)
}

fn extract_field(value: &Value) -> Option<String> {
    string_by_pointers(value, &["/field", "/properties/field"]).map(ToString::to_string)
}

fn extract_delta(value: &Value) -> Option<String> {
    string_by_pointers(value, &["/delta", "/properties/delta", "/value/delta"])
        .map(ToString::to_string)
}

fn extract_status(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/status",
            "/status/type",
            "/properties/status",
            "/properties/status/type",
            "/properties/info/status",
            "/properties/info/status/type",
            "/message/status",
            "/message/status/type",
            "/properties/message/status",
            "/properties/message/status/type",
        ],
    )
    .map(ToString::to_string)
}

fn extract_stop_reason(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/stopReason",
            "/stop_reason",
            "/finish",
            "/properties/stopReason",
            "/properties/stop_reason",
            "/properties/info/finish",
            "/message/stopReason",
            "/message/stop_reason",
            "/message/finish",
            "/properties/message/finish",
        ],
    )
    .map(ToString::to_string)
}

fn extract_error_message(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/error/message",
            "/properties/error/message",
            "/message",
            "/properties/message",
        ],
    )
    .map(ToString::to_string)
}

fn infer_part_kind(value: &Value) -> Option<PartKind> {
    let part_type = string_by_pointers(
        value,
        &[
            "/part/type",
            "/properties/part/type",
            "/type_name",
            "/properties/type_name",
        ],
    )?;

    match part_type {
        "text" => Some(PartKind::Text),
        "tool" | "tool_use" => Some(PartKind::ToolUse {
            name: extract_tool_name(value).unwrap_or_else(|| "tool".to_string()),
        }),
        _ => None,
    }
}

fn extract_tool_name(value: &Value) -> Option<String> {
    string_by_pointers(
        value,
        &[
            "/part/tool/name",
            "/properties/part/tool/name",
            "/part/name",
            "/properties/part/name",
            "/tool/name",
            "/properties/tool/name",
        ],
    )
    .map(ToString::to_string)
}

fn string_by_pointers<'a>(value: &'a Value, pointers: &[&str]) -> Option<&'a str> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
}

fn is_text_field(field: &str) -> bool {
    field.eq_ignore_ascii_case("text")
}

fn adapter_debug(label: &str, detail: &str) {
    tracing::debug!(target: "anima_runtime::stream_adapter", label, detail);
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "complete" | "done" | "stopped" | "aborted" | "cancelled"
    )
}

fn is_tool_calls_stop_reason(stop_reason: &str) -> bool {
    stop_reason.eq_ignore_ascii_case("tool-calls") || stop_reason.eq_ignore_ascii_case("tool_calls")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter_from_lines(raw_lines: Vec<&str>) -> OpenCodeEventAdapter {
        let owned_lines: Vec<String> = raw_lines.into_iter().map(|line| line.to_string()).collect();
        let lines = owned_lines.into_iter().map(Ok);
        let (message_result_tx, message_result_rx) = mpsc::channel();
        let _ = message_result_tx.send(Ok("msg-1".to_string()));
        OpenCodeEventAdapter::new("sess-1".into(), message_result_rx, Box::new(lines))
    }

    fn collect_ok_lines(adapter: OpenCodeEventAdapter) -> Vec<String> {
        adapter.collect::<Result<Vec<_>, _>>().unwrap()
    }

    #[test]
    fn extracts_message_id_from_post_response() {
        let value = json!({"id": "msg-123"});
        assert_eq!(extract_message_id_from_response(&value).unwrap(), "msg-123");
    }

    #[test]
    fn adapts_text_delta_events_into_compatible_sse_lines() {
        let lines = collect_ok_lines(adapter_from_lines(vec![
            r#"data: {"type":"message.updated","sessionID":"sess-1","messageID":"msg-2","role":"assistant","parentID":"msg-1"}"#,
            "",
            r#"data: {"type":"message.part.updated","sessionID":"sess-1","messageID":"msg-2","partID":"part-1","part":{"id":"part-1","type":"text"}}"#,
            "",
            r#"data: {"type":"message.part.delta","sessionID":"sess-1","messageID":"msg-2","partID":"part-1","field":"text","delta":"Hello "}"#,
            "",
            r#"data: {"type":"message.part.delta","sessionID":"sess-1","messageID":"msg-2","partID":"part-1","field":"text","delta":"world"}"#,
            "",
            r#"data: {"type":"session.status","sessionID":"sess-1","status":"completed"}"#,
            "",
        ]));

        assert_eq!(
            lines,
            vec![
                r#"data: {"message":{"id":"msg-2"},"type":"message_start"}"#,
                r#"data: {"content_block":{"text":"","type":"text"},"index":0,"type":"content_block_start"}"#,
                r#"data: {"delta":{"text":"Hello ","type":"text_delta"},"index":0,"type":"content_block_delta"}"#,
                r#"data: {"delta":{"text":"world","type":"text_delta"},"index":0,"type":"content_block_delta"}"#,
                r#"data: {"index":0,"type":"content_block_stop"}"#,
                r#"data: {"delta":{"stop_reason":"completed"},"type":"message_delta"}"#,
                r#"data: {"type":"message_stop"}"#,
            ]
        );
    }

    #[test]
    fn filters_out_other_message_events() {
        let lines = collect_ok_lines(adapter_from_lines(vec![
            r#"data: {"type":"message.updated","sessionID":"sess-1","messageID":"msg-other"}"#,
            r#"data: {"type":"message.part.delta","sessionID":"sess-1","messageID":"msg-other","partID":"part-x","field":"text","delta":"ignore me"}"#,
        ]));

        assert!(lines.is_empty());
    }

    #[test]
    fn keeps_part_index_stable_and_maps_tool_input_delta() {
        let lines = collect_ok_lines(adapter_from_lines(vec![
            r#"data: {"type":"message.updated","sessionID":"sess-1","messageID":"msg-2","role":"assistant","parentID":"msg-1"}"#,
            "",
            r#"data: {"type":"message.part.updated","sessionID":"sess-1","messageID":"msg-2","partID":"tool-part","part":{"id":"tool-part","type":"tool","tool":{"name":"bash"}}}"#,
            "",
            r#"data: {"type":"message.part.delta","sessionID":"sess-1","messageID":"msg-2","partID":"tool-part","field":"input","delta":"{\"command\""}"#,
            "",
            r#"data: {"type":"message.part.delta","sessionID":"sess-1","messageID":"msg-2","partID":"tool-part","field":"input","delta":":\"ls\"}"}"#,
            "",
            r#"data: {"type":"message.part.updated","sessionID":"sess-1","messageID":"msg-2","partID":"tool-part","part":{"id":"tool-part","type":"tool","tool":{"name":"bash"}},"status":"completed"}"#,
            "",
            r#"data: {"type":"session.status","sessionID":"sess-1","status":"completed"}"#,
            "",
        ]));

        assert!(lines
            .iter()
            .any(|line| line.contains(r#""index":0,"type":"content_block_start""#)));
        assert!(lines
            .iter()
            .any(|line| line.contains(r#""partial_json":"{\"command\"""#)));
        assert!(lines
            .iter()
            .any(|line| line.contains(r#""partial_json":":\"ls\"}""#)));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains(r#""index":0"#))
                .count(),
            4
        );
    }

    #[test]
    fn maps_session_error_to_error_event() {
        let lines = collect_ok_lines(adapter_from_lines(vec![
            r#"data: {"type":"session.status","sessionID":"sess-1","status":"error","message":"boom"}"#,
        ]));

        assert_eq!(
            lines,
            vec![r#"data: {"error":{"message":"boom","type":"error"},"type":"error"}"#]
        );
    }

    #[test]
    fn rolls_over_to_next_assistant_message_after_tool_calls_finish() {
        let lines = collect_ok_lines(adapter_from_lines(vec![
            r#"data: {"type":"message.updated","sessionID":"sess-1","messageID":"msg-2","role":"assistant","parentID":"msg-1"}"#,
            "",
            r#"data: {"type":"message.part.updated","sessionID":"sess-1","messageID":"msg-2","partID":"tool-part","part":{"id":"tool-part","type":"tool","tool":{"name":"todowrite"}}}"#,
            "",
            r#"data: {"type":"message.updated","sessionID":"sess-1","messageID":"msg-2","role":"assistant","parentID":"msg-1","finish":"tool-calls"}"#,
            "",
            r#"data: {"type":"message.updated","sessionID":"sess-1","messageID":"msg-3","role":"assistant","parentID":"msg-1"}"#,
            "",
            r#"data: {"type":"message.part.updated","sessionID":"sess-1","messageID":"msg-3","partID":"part-2","part":{"id":"part-2","type":"text"}}"#,
            "",
            r#"data: {"type":"message.part.delta","sessionID":"sess-1","messageID":"msg-3","partID":"part-2","field":"text","delta":"done"}"#,
            "",
            r#"data: {"type":"session.status","sessionID":"sess-1","status":"completed"}"#,
            "",
        ]));

        assert_eq!(
            lines,
            vec![
                r#"data: {"message":{"id":"msg-2"},"type":"message_start"}"#,
                r#"data: {"content_block":{"id":"tool-part","input":{},"name":"todowrite","type":"tool_use"},"index":0,"type":"content_block_start"}"#,
                r#"data: {"index":0,"type":"content_block_stop"}"#,
                r#"data: {"content_block":{"text":"","type":"text"},"index":1,"type":"content_block_start"}"#,
                r#"data: {"delta":{"text":"done","type":"text_delta"},"index":1,"type":"content_block_delta"}"#,
                r#"data: {"index":1,"type":"content_block_stop"}"#,
                r#"data: {"delta":{"stop_reason":"completed"},"type":"message_delta"}"#,
                r#"data: {"type":"message_stop"}"#,
            ]
        );
    }

    #[test]
    fn extracts_nested_status_type_from_real_event_shape() {
        let value = json!({
            "type": "session.status",
            "properties": {
                "sessionID": "sess-1",
                "status": { "type": "idle" }
            }
        });

        assert_eq!(extract_status(&value).as_deref(), Some("idle"));
    }

    #[test]
    fn extracts_nested_message_status_type_shape() {
        let value = json!({
            "properties": {
                "message": {
                    "status": { "type": "completed" }
                }
            }
        });

        assert_eq!(extract_status(&value).as_deref(), Some("completed"));
    }
}
