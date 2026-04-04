//! # Worker 与 WorkerPool
//!
//! 本模块实现了任务执行的核心工作单元：
//! - `WorkerAgent`：单个工作者，负责接收并执行一个任务（线程安全、支持并发提交）
//! - `WorkerPool`：工作者池，管理多个 WorkerAgent 的生命周期，提供任务分发与负载均衡
//!
//! 设计要点：
//! - 每个 WorkerAgent 同一时刻只处理一个任务（通过 `busy` 原子标志保证）
//! - WorkerPool 使用 Condvar 等待可用 Worker，避免忙轮询
//! - 任务提交后通过 crossbeam channel 异步返回结果

use anima_sdk::facade::Client as SdkClient;
use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub type RuntimeEventPublisher = Arc<dyn Fn(&str, &str, Value) + Send + Sync>;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

use super::executor::TaskExecutor;
use super::types::{make_task_result, MakeTaskResult, Task, TaskResult};
use crate::streaming::executor::consume_sse_stream;
use crate::streaming::types::{ContentBlock, ContentDelta, StreamEvent};
use crate::support::now_ms;

/// Worker 当前正在执行的任务信息
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentTaskInfo {
    pub task_id: String,
    pub trace_id: String,
    pub task_type: String,
    pub started_ms: u64,
    pub content_preview: String,
    pub phase: String,
    pub last_progress_at_ms: u64,
    pub opencode_session_id: Option<String>,
}

/// 单个工作者智能体，封装了 SDK 客户端和任务执行器。
/// 通过原子标志 `running` / `busy` 实现无锁的状态管理。
pub struct WorkerAgent {
    id: String,
    client: SdkClient,
    executor: Arc<dyn TaskExecutor>,
    running: AtomicBool,
    busy: AtomicBool,
    metrics: Mutex<WorkerMetrics>,
    current_task: Mutex<Option<CurrentTaskInfo>>,
    runtime_event_publisher: Option<RuntimeEventPublisher>,
    #[allow(dead_code)] // stored for future task timeout enforcement
    timeout_ms: u64,
}

/// Worker 的运行指标统计
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkerMetrics {
    pub tasks_completed: u64,
    pub timeouts: u64,
    pub errors: u64,
    pub total_duration_ms: u64,
}

/// Worker 的当前状态快照（stopped / busy / idle）
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerStatus {
    pub id: String,
    pub status: String,
    pub metrics: WorkerMetrics,
    pub current_task: Option<CurrentTaskInfo>,
}

impl WorkerAgent {
    /// 创建新的 WorkerAgent，自动生成 UUID 作为唯一标识
    pub fn new(
        client: SdkClient,
        executor: Arc<dyn TaskExecutor>,
        timeout_ms: Option<u64>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            client,
            executor,
            running: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            metrics: Mutex::new(WorkerMetrics::default()),
            current_task: Mutex::new(None),
            runtime_event_publisher: None,
            timeout_ms: timeout_ms.unwrap_or(60_000),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn with_runtime_event_publisher(mut self, publisher: RuntimeEventPublisher) -> Self {
        self.runtime_event_publisher = Some(publisher);
        self
    }

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> WorkerStatus {
        let status = if !self.is_running() {
            "stopped"
        } else if self.is_busy() {
            "busy"
        } else {
            "idle"
        };
        WorkerStatus {
            id: self.id.clone(),
            status: status.to_string(),
            metrics: self.metrics.lock().clone(),
            current_task: self.current_task.lock().clone(),
        }
    }

    fn publish_runtime_event(&self, trace_id: &str, event: &str, payload: Value) {
        if let Some(publisher) = &self.runtime_event_publisher {
            publisher(trace_id, event, payload);
        }
    }

    fn update_current_task_phase(&self, phase: &str) {
        let mut current_task = self.current_task.lock();
        if let Some(task) = current_task.as_mut() {
            task.phase = phase.to_string();
            task.last_progress_at_ms = now_ms();
        }
    }

    fn should_stream_api_call(&self, task: &Task) -> bool {
        task.task_type == "api-call"
            && task
                .metadata
                .get("orchestration_subtask")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            && task
                .metadata
                .get("streaming_observable")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    }

    fn api_call_event_payload(
        &self,
        task: &Task,
        session_id: &str,
        phase: &str,
        occurred_at_ms: u64,
    ) -> Value {
        json!({
            "worker_id": self.id.clone(),
            "task_id": task.id,
            "trace_id": task.trace_id,
            "task_type": task.task_type,
            "opencode_session_id": session_id,
            "worker_timeout_ms": task.timeout_ms,
            "parent_job_id": task.payload.get("parent_job_id").cloned().unwrap_or(Value::Null),
            "plan_id": task.payload.get("plan_id").cloned().unwrap_or(Value::Null),
            "subtask_id": task.payload.get("subtask_id").cloned().unwrap_or(Value::Null),
            "subtask_name": task.payload.get("subtask_name").cloned().unwrap_or(Value::Null),
            "original_task_type": task.payload.get("original_task_type").cloned().unwrap_or(Value::Null),
            "phase": phase,
            "occurred_at_ms": occurred_at_ms,
        })
    }

    fn publish_streaming_diagnostic_event(
        &self,
        task: &Task,
        session_id: &str,
        event: &str,
        phase: &str,
        started_at_ms: Option<u64>,
        details: Option<Value>,
    ) {
        let occurred_at_ms = now_ms();
        self.update_current_task_phase(phase);
        let mut payload = self.api_call_event_payload(task, session_id, phase, occurred_at_ms);
        if let Some(started_at_ms) = started_at_ms {
            payload["started_at_ms"] = json!(started_at_ms);
            payload["elapsed_ms"] = json!(occurred_at_ms.saturating_sub(started_at_ms));
        }
        if let Some(Value::Object(extra)) = details {
            if let Some(payload_obj) = payload.as_object_mut() {
                payload_obj.extend(extra);
            }
        }
        self.publish_runtime_event(&task.trace_id, event, payload);
    }

    fn stream_event_phase(event: &StreamEvent) -> &'static str {
        match event {
            StreamEvent::MessageStart { .. } => "api_call_stream_message_started",
            StreamEvent::ContentBlockStart {
                content_block: ContentBlock::Text { .. },
                ..
            }
            | StreamEvent::ContentBlockDelta {
                delta: ContentDelta::TextDelta { .. },
                ..
            } => "api_call_stream_texting",
            StreamEvent::ContentBlockStart {
                content_block: ContentBlock::ToolUse { .. },
                ..
            }
            | StreamEvent::ContentBlockDelta {
                delta: ContentDelta::InputJsonDelta { .. },
                ..
            } => "api_call_stream_tool_input",
            StreamEvent::ContentBlockStop { .. } => "api_call_stream_block_completed",
            StreamEvent::MessageDelta { .. } => "api_call_stream_message_delta",
            StreamEvent::MessageStop => "api_call_stream_completed",
            StreamEvent::Ping => "api_call_stream_open",
            StreamEvent::Error { .. } => "api_call_failed",
        }
    }

    fn publish_stream_runtime_event(&self, task: &Task, session_id: &str, event: &StreamEvent) {
        let occurred_at_ms = now_ms();
        let phase = Self::stream_event_phase(event);
        self.update_current_task_phase(phase);

        let mut payload = self.api_call_event_payload(task, session_id, phase, occurred_at_ms);
        let event_name = match event {
            StreamEvent::MessageStart { message_id } => {
                payload["message_id"] = Value::String(message_id.clone());
                payload["raw_event_type"] = Value::String("message_start".into());
                "sdk_stream_message_started"
            }
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                payload["content_block_index"] = json!(index);
                match content_block {
                    ContentBlock::Text { text } => {
                        payload["content_block_kind"] = Value::String("text".into());
                        payload["part_id"] = Value::String(format!("text-{index}"));
                        payload["raw_event_type"] = Value::String("content_block_start".into());
                        if !text.is_empty() {
                            payload["text_delta"] = Value::String(text.clone());
                            payload["accumulated_text_preview"] =
                                Value::String(text.chars().take(120).collect());
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        payload["content_block_kind"] = Value::String("tool_use".into());
                        payload["tool_use_id"] = Value::String(id.clone());
                        payload["part_id"] = Value::String(id.clone());
                        payload["tool_name"] = Value::String(name.clone());
                        payload["partial_json"] = input.clone();
                        payload["raw_event_type"] = Value::String("content_block_start".into());
                    }
                }
                "sdk_stream_content_block_started"
            }
            StreamEvent::ContentBlockDelta { index, delta } => {
                payload["content_block_index"] = json!(index);
                payload["raw_event_type"] = Value::String("content_block_delta".into());
                match delta {
                    ContentDelta::TextDelta { text } => {
                        payload["delta_kind"] = Value::String("text_delta".into());
                        payload["part_id"] = Value::String(format!("text-{index}"));
                        payload["text_delta"] = Value::String(text.clone());
                        payload["accumulated_text_preview"] =
                            Value::String(text.chars().take(120).collect());
                    }
                    ContentDelta::InputJsonDelta { partial_json } => {
                        payload["delta_kind"] = Value::String("input_json_delta".into());
                        payload["part_id"] = Value::String(format!("tool-{index}"));
                        payload["partial_json"] = Value::String(partial_json.clone());
                    }
                }
                "sdk_stream_content_block_delta"
            }
            StreamEvent::ContentBlockStop { index } => {
                payload["content_block_index"] = json!(index);
                payload["part_id"] = Value::String(format!("block-{index}"));
                payload["raw_event_type"] = Value::String("content_block_stop".into());
                "sdk_stream_content_block_stopped"
            }
            StreamEvent::MessageDelta { stop_reason } => {
                payload["raw_event_type"] = Value::String("message_delta".into());
                if let Some(stop_reason) = stop_reason {
                    payload["stop_reason"] = Value::String(stop_reason.clone());
                }
                "sdk_stream_message_delta"
            }
            StreamEvent::MessageStop => {
                payload["raw_event_type"] = Value::String("message_stop".into());
                "sdk_stream_message_stopped"
            }
            StreamEvent::Ping => "worker_api_call_streaming_started",
            StreamEvent::Error {
                error_type,
                message,
            } => {
                payload["error_kind"] = Value::String(error_type.clone());
                payload["error"] = Value::String(message.clone());
                "worker_api_call_streaming_failed"
            }
        };

        self.publish_runtime_event(&task.trace_id, event_name, payload);
    }

    fn execute_api_call_blocking(&self, task: &Task, session_id: String, content: Value) -> ExecuteResult {
        let started_at_ms = now_ms();
        self.update_current_task_phase("api_call_inflight");
        let mut started_payload =
            self.api_call_event_payload(task, &session_id, "api_call_inflight", started_at_ms);
        started_payload["started_at_ms"] = json!(started_at_ms);
        self.publish_runtime_event(&task.trace_id, "worker_api_call_started", started_payload);

        let sdk_started_at_ms = now_ms();
        let mut sdk_started_payload = self.api_call_event_payload(
            task,
            &session_id,
            "api_call_inflight",
            sdk_started_at_ms,
        );
        sdk_started_payload["sdk_operation"] = Value::String("send_prompt".into());
        sdk_started_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
        sdk_started_payload["started_at_ms"] = json!(sdk_started_at_ms);
        self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_started", sdk_started_payload);

        match self.executor.send_prompt(&self.client, &session_id, content) {
            Ok(result) => {
                let sdk_finished_at_ms = now_ms();
                let mut sdk_finished_payload = self.api_call_event_payload(
                    task,
                    &session_id,
                    "api_call_finished",
                    sdk_finished_at_ms,
                );
                sdk_finished_payload["sdk_operation"] = Value::String("send_prompt".into());
                sdk_finished_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
                sdk_finished_payload["started_at_ms"] = json!(sdk_started_at_ms);
                sdk_finished_payload["finished_at_ms"] = json!(sdk_finished_at_ms);
                sdk_finished_payload["sdk_duration_ms"] =
                    json!(sdk_finished_at_ms.saturating_sub(sdk_started_at_ms));
                sdk_finished_payload["result_status"] = Value::String("success".into());
                self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_finished", sdk_finished_payload);

                let finished_at_ms = now_ms();
                self.update_current_task_phase("api_call_finished");
                let mut finished_payload =
                    self.api_call_event_payload(task, &session_id, "api_call_finished", finished_at_ms);
                finished_payload["started_at_ms"] = json!(started_at_ms);
                finished_payload["finished_at_ms"] = json!(finished_at_ms);
                finished_payload["api_call_duration_ms"] =
                    json!(finished_at_ms.saturating_sub(started_at_ms));
                finished_payload["result_status"] = Value::String("success".into());
                self.publish_runtime_event(&task.trace_id, "worker_api_call_finished", finished_payload);
                ExecuteResult::success(result)
            }
            Err(error) => {
                let sdk_finished_at_ms = now_ms();
                let error_kind = classify_error_kind(&error);
                let mut sdk_failed_payload = self.api_call_event_payload(
                    task,
                    &session_id,
                    "api_call_failed",
                    sdk_finished_at_ms,
                );
                sdk_failed_payload["sdk_operation"] = Value::String("send_prompt".into());
                sdk_failed_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
                sdk_failed_payload["started_at_ms"] = json!(sdk_started_at_ms);
                sdk_failed_payload["finished_at_ms"] = json!(sdk_finished_at_ms);
                sdk_failed_payload["sdk_duration_ms"] =
                    json!(sdk_finished_at_ms.saturating_sub(sdk_started_at_ms));
                sdk_failed_payload["result_status"] = Value::String("failure".into());
                sdk_failed_payload["error"] = Value::String(error.clone());
                sdk_failed_payload["error_kind"] = Value::String(error_kind.into());
                self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_failed", sdk_failed_payload);

                let finished_at_ms = now_ms();
                self.update_current_task_phase("api_call_failed");
                let mut failed_payload =
                    self.api_call_event_payload(task, &session_id, "api_call_failed", finished_at_ms);
                failed_payload["started_at_ms"] = json!(started_at_ms);
                failed_payload["finished_at_ms"] = json!(finished_at_ms);
                failed_payload["api_call_duration_ms"] =
                    json!(finished_at_ms.saturating_sub(started_at_ms));
                failed_payload["result_status"] = Value::String("failure".into());
                failed_payload["error"] = Value::String(error.clone());
                failed_payload["error_kind"] = Value::String(error_kind.into());
                self.publish_runtime_event(&task.trace_id, "worker_api_call_failed", failed_payload);
                ExecuteResult::failure(error)
            }
        }
    }

    fn execute_api_call_streaming(&self, task: &Task, session_id: String, content: Value) -> ExecuteResult {
        let started_at_ms = now_ms();
        self.update_current_task_phase("api_call_inflight");
        let mut started_payload =
            self.api_call_event_payload(task, &session_id, "api_call_inflight", started_at_ms);
        started_payload["started_at_ms"] = json!(started_at_ms);
        self.publish_runtime_event(&task.trace_id, "worker_api_call_started", started_payload);

        let stream_open_at_ms = now_ms();
        self.update_current_task_phase("api_call_stream_open");
        let mut streaming_started_payload = self.api_call_event_payload(
            task,
            &session_id,
            "api_call_stream_open",
            stream_open_at_ms,
        );
        streaming_started_payload["started_at_ms"] = json!(stream_open_at_ms);
        self.publish_runtime_event(
            &task.trace_id,
            "worker_api_call_streaming_started",
            streaming_started_payload,
        );

        let sdk_started_at_ms = now_ms();
        let mut sdk_started_payload = self.api_call_event_payload(
            task,
            &session_id,
            "api_call_stream_open",
            sdk_started_at_ms,
        );
        sdk_started_payload["sdk_operation"] = Value::String("send_prompt_streaming".into());
        sdk_started_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
        sdk_started_payload["started_at_ms"] = json!(sdk_started_at_ms);
        self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_started", sdk_started_payload);
        self.publish_streaming_diagnostic_event(
            task,
            &session_id,
            "sdk_stream_http_request_started",
            "api_call_stream_http_request_started",
            Some(sdk_started_at_ms),
            Some(json!({
                "sdk_operation": "send_prompt_streaming",
                "sdk_base_url": self.client.base_url.clone(),
            })),
        );

        match self
            .executor
            .send_prompt_streaming(&self.client, &session_id, content.clone())
        {
            Ok(lines) => {
                self.publish_streaming_diagnostic_event(
                    task,
                    &session_id,
                    "sdk_stream_http_response_opened",
                    "api_call_stream_http_response_opened",
                    Some(sdk_started_at_ms),
                    Some(json!({
                        "sdk_operation": "send_prompt_streaming",
                        "sdk_base_url": self.client.base_url.clone(),
                    })),
                );
                self.publish_streaming_diagnostic_event(
                    task,
                    &session_id,
                    "sdk_stream_sse_consume_started",
                    "api_call_stream_sse_consume_started",
                    Some(sdk_started_at_ms),
                    Some(json!({
                        "sdk_operation": "send_prompt_streaming",
                        "sdk_base_url": self.client.base_url.clone(),
                    })),
                );
                let saw_first_event = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let saw_first_event_flag = Arc::clone(&saw_first_event);
                let on_event = |event: StreamEvent| {
                    if !saw_first_event_flag.swap(true, Ordering::SeqCst) {
                        self.publish_streaming_diagnostic_event(
                            task,
                            &session_id,
                            "sdk_stream_first_event_observed",
                            "api_call_stream_first_event_observed",
                            Some(sdk_started_at_ms),
                            Some(json!({
                                "sdk_operation": "send_prompt_streaming",
                                "sdk_base_url": self.client.base_url.clone(),
                                "stream_event_kind": match &event {
                                    StreamEvent::MessageStart { .. } => "message_start",
                                    StreamEvent::ContentBlockStart { .. } => "content_block_start",
                                    StreamEvent::ContentBlockDelta { .. } => "content_block_delta",
                                    StreamEvent::ContentBlockStop { .. } => "content_block_stop",
                                    StreamEvent::MessageDelta { .. } => "message_delta",
                                    StreamEvent::MessageStop => "message_stop",
                                    StreamEvent::Ping => "ping",
                                    StreamEvent::Error { .. } => "error",
                                },
                            })),
                        );
                    }
                    self.publish_stream_runtime_event(task, &session_id, &event);
                };
                match consume_sse_stream(lines, Some(&on_event)) {
                    Ok((_parsed, response_value)) => {
                        let sdk_finished_at_ms = now_ms();
                        let mut sdk_finished_payload = self.api_call_event_payload(
                            task,
                            &session_id,
                            "api_call_stream_completed",
                            sdk_finished_at_ms,
                        );
                        sdk_finished_payload["sdk_operation"] = Value::String("send_prompt_streaming".into());
                        sdk_finished_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
                        sdk_finished_payload["started_at_ms"] = json!(sdk_started_at_ms);
                        sdk_finished_payload["finished_at_ms"] = json!(sdk_finished_at_ms);
                        sdk_finished_payload["sdk_duration_ms"] =
                            json!(sdk_finished_at_ms.saturating_sub(sdk_started_at_ms));
                        sdk_finished_payload["result_status"] = Value::String("success".into());
                        self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_finished", sdk_finished_payload);

                        let streaming_finished_at_ms = now_ms();
                        self.update_current_task_phase("api_call_stream_completed");
                        let mut streaming_finished_payload = self.api_call_event_payload(
                            task,
                            &session_id,
                            "api_call_stream_completed",
                            streaming_finished_at_ms,
                        );
                        streaming_finished_payload["started_at_ms"] = json!(stream_open_at_ms);
                        streaming_finished_payload["finished_at_ms"] = json!(streaming_finished_at_ms);
                        streaming_finished_payload["streaming_duration_ms"] =
                            json!(streaming_finished_at_ms.saturating_sub(stream_open_at_ms));
                        streaming_finished_payload["result_status"] = Value::String("success".into());
                        self.publish_runtime_event(
                            &task.trace_id,
                            "worker_api_call_streaming_finished",
                            streaming_finished_payload,
                        );

                        let finished_at_ms = now_ms();
                        self.update_current_task_phase("api_call_finished");
                        let mut finished_payload =
                            self.api_call_event_payload(task, &session_id, "api_call_finished", finished_at_ms);
                        finished_payload["started_at_ms"] = json!(started_at_ms);
                        finished_payload["finished_at_ms"] = json!(finished_at_ms);
                        finished_payload["api_call_duration_ms"] =
                            json!(finished_at_ms.saturating_sub(started_at_ms));
                        finished_payload["result_status"] = Value::String("success".into());
                        self.publish_runtime_event(&task.trace_id, "worker_api_call_finished", finished_payload);
                        ExecuteResult::success(response_value)
                    }
                    Err(error) => {
                        let sdk_finished_at_ms = now_ms();
                        let error_kind = classify_error_kind(&error);
                        let mut sdk_failed_payload = self.api_call_event_payload(
                            task,
                            &session_id,
                            "api_call_failed",
                            sdk_finished_at_ms,
                        );
                        sdk_failed_payload["sdk_operation"] = Value::String("send_prompt_streaming".into());
                        sdk_failed_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
                        sdk_failed_payload["started_at_ms"] = json!(sdk_started_at_ms);
                        sdk_failed_payload["finished_at_ms"] = json!(sdk_finished_at_ms);
                        sdk_failed_payload["sdk_duration_ms"] =
                            json!(sdk_finished_at_ms.saturating_sub(sdk_started_at_ms));
                        sdk_failed_payload["result_status"] = Value::String("failure".into());
                        sdk_failed_payload["error"] = Value::String(error.clone());
                        sdk_failed_payload["error_kind"] = Value::String(error_kind.into());
                        self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_failed", sdk_failed_payload);

                        let streaming_failed_at_ms = now_ms();
                        self.update_current_task_phase("api_call_failed");
                        let mut streaming_failed_payload = self.api_call_event_payload(
                            task,
                            &session_id,
                            "api_call_failed",
                            streaming_failed_at_ms,
                        );
                        streaming_failed_payload["started_at_ms"] = json!(stream_open_at_ms);
                        streaming_failed_payload["finished_at_ms"] = json!(streaming_failed_at_ms);
                        streaming_failed_payload["streaming_duration_ms"] =
                            json!(streaming_failed_at_ms.saturating_sub(stream_open_at_ms));
                        streaming_failed_payload["result_status"] = Value::String("failure".into());
                        streaming_failed_payload["error"] = Value::String(error.clone());
                        streaming_failed_payload["error_kind"] = Value::String(error_kind.into());
                        self.publish_runtime_event(
                            &task.trace_id,
                            "worker_api_call_streaming_failed",
                            streaming_failed_payload,
                        );

                        let finished_at_ms = now_ms();
                        let mut failed_payload =
                            self.api_call_event_payload(task, &session_id, "api_call_failed", finished_at_ms);
                        failed_payload["started_at_ms"] = json!(started_at_ms);
                        failed_payload["finished_at_ms"] = json!(finished_at_ms);
                        failed_payload["api_call_duration_ms"] =
                            json!(finished_at_ms.saturating_sub(started_at_ms));
                        failed_payload["result_status"] = Value::String("failure".into());
                        failed_payload["error"] = Value::String(error.clone());
                        failed_payload["error_kind"] = Value::String(error_kind.into());
                        self.publish_runtime_event(&task.trace_id, "worker_api_call_failed", failed_payload);
                        ExecuteResult::failure(error)
                    }
                }
            }
            Err(error) => {
                if is_streaming_unsupported(&error) {
                    self.update_current_task_phase("api_call_inflight");
                    return self.execute_api_call_blocking(task, session_id, content);
                }

                let sdk_finished_at_ms = now_ms();
                let error_kind = classify_error_kind(&error);
                let mut sdk_failed_payload = self.api_call_event_payload(
                    task,
                    &session_id,
                    "api_call_failed",
                    sdk_finished_at_ms,
                );
                sdk_failed_payload["sdk_operation"] = Value::String("send_prompt_streaming".into());
                sdk_failed_payload["sdk_base_url"] = Value::String(self.client.base_url.clone());
                sdk_failed_payload["started_at_ms"] = json!(sdk_started_at_ms);
                sdk_failed_payload["finished_at_ms"] = json!(sdk_finished_at_ms);
                sdk_failed_payload["sdk_duration_ms"] =
                    json!(sdk_finished_at_ms.saturating_sub(sdk_started_at_ms));
                sdk_failed_payload["result_status"] = Value::String("failure".into());
                sdk_failed_payload["error"] = Value::String(error.clone());
                sdk_failed_payload["error_kind"] = Value::String(error_kind.into());
                self.publish_runtime_event(&task.trace_id, "sdk_send_prompt_failed", sdk_failed_payload);

                let streaming_failed_at_ms = now_ms();
                self.update_current_task_phase("api_call_failed");
                let mut streaming_failed_payload = self.api_call_event_payload(
                    task,
                    &session_id,
                    "api_call_failed",
                    streaming_failed_at_ms,
                );
                streaming_failed_payload["started_at_ms"] = json!(stream_open_at_ms);
                streaming_failed_payload["finished_at_ms"] = json!(streaming_failed_at_ms);
                streaming_failed_payload["streaming_duration_ms"] =
                    json!(streaming_failed_at_ms.saturating_sub(stream_open_at_ms));
                streaming_failed_payload["result_status"] = Value::String("failure".into());
                streaming_failed_payload["error"] = Value::String(error.clone());
                streaming_failed_payload["error_kind"] = Value::String(error_kind.into());
                self.publish_runtime_event(
                    &task.trace_id,
                    "worker_api_call_streaming_failed",
                    streaming_failed_payload,
                );

                let finished_at_ms = now_ms();
                let mut failed_payload =
                    self.api_call_event_payload(task, &session_id, "api_call_failed", finished_at_ms);
                failed_payload["started_at_ms"] = json!(started_at_ms);
                failed_payload["finished_at_ms"] = json!(finished_at_ms);
                failed_payload["api_call_duration_ms"] =
                    json!(finished_at_ms.saturating_sub(started_at_ms));
                failed_payload["result_status"] = Value::String("failure".into());
                failed_payload["error"] = Value::String(error.clone());
                failed_payload["error_kind"] = Value::String(error_kind.into());
                self.publish_runtime_event(&task.trace_id, "worker_api_call_failed", failed_payload);
                ExecuteResult::failure(error)
            }
        }
    }

    /// 提交任务给此 Worker 执行。
    /// 返回一个 Receiver，调用方可通过它异步获取执行结果。
    /// 如果 Worker 未运行或正忙，会立即返回失败结果。
    pub fn submit_task(
        self: &Arc<Self>,
        task: Task,
        notify: Option<Arc<Condvar>>,
    ) -> crossbeam_channel::Receiver<TaskResult> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if !self.is_running() {
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Worker is not running".into()),
                duration_ms: 0,
                worker_id: Some(self.id.clone()),
                result: None,
            }));
            return rx;
        }

        // 用 swap 原子地检查并设置 busy 标志，防止并发提交
        if self.busy.swap(true, Ordering::SeqCst) {
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Worker is busy".into()),
                duration_ms: 0,
                worker_id: Some(self.id.clone()),
                result: None,
            }));
            return rx;
        }

        let worker = Arc::clone(self);
        thread::spawn(move || {
            // Panic guard: ensure busy is cleared even if execute_task panics
            let _guard = PanicGuard {
                busy: &worker.busy,
                notify: notify.as_deref(),
            };

            let task_id = task.id.clone();
            let trace_id = task.trace_id.clone();
            let task_type = task.task_type.clone();
            let opencode_session_id = task
                .payload
                .get("opencode-session-id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let worker_timeout_ms = task.timeout_ms;

            // 记录当前任务信息
            let content_preview = task
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>();
            let started_ms = now_ms();
            *worker.current_task.lock() = Some(CurrentTaskInfo {
                task_id: task_id.clone(),
                trace_id: trace_id.clone(),
                task_type: task_type.clone(),
                started_ms,
                content_preview,
                phase: "task_started".into(),
                last_progress_at_ms: started_ms,
                opencode_session_id: opencode_session_id.clone(),
            });

            let start = now_ms();
            let result = worker.execute_task(&task);
            let duration_ms = now_ms().saturating_sub(start);

            let busy_cleared = true;
            *worker.current_task.lock() = None;
            let current_task_cleared = worker.current_task.lock().is_none();
            worker.publish_runtime_event(
                &trace_id,
                "worker_task_cleanup_finished",
                json!({
                    "worker_id": worker.id,
                    "task_id": task_id,
                    "trace_id": trace_id,
                    "task_type": task_type,
                    "opencode_session_id": opencode_session_id,
                    "worker_timeout_ms": worker_timeout_ms,
                    "finished_at_ms": now_ms(),
                    "busy_cleared": busy_cleared,
                    "current_task_cleared": current_task_cleared,
                    "result_status": result.status,
                }),
            );
            {
                let mut metrics = worker.metrics.lock();
                metrics.total_duration_ms += duration_ms;
                if result.status == "success" {
                    metrics.tasks_completed += 1;
                } else if result.status == "timeout" {
                    metrics.timeouts += 1;
                } else {
                    metrics.errors += 1;
                }
            }
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: result.status,
                result: result.result,
                error: result.error,
                duration_ms,
                worker_id: Some(worker.id.clone()),
            }));
            // _guard drops here: clears busy + notifies condvar
        });

        rx
    }

    /// 根据任务类型分发执行：api-call / session-create / transform / query
    fn execute_task(&self, task: &Task) -> ExecuteResult {
        match task.task_type.as_str() {
            // 调用 SDK 发送 prompt 到已有会话
            "api-call" => {
                let session_id = task
                    .payload
                    .get("opencode-session-id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let content = task.payload.get("content").cloned();
                match (session_id, content) {
                    (Some(session_id), Some(content)) => {
                        if self.should_stream_api_call(task) {
                            self.execute_api_call_streaming(task, session_id, content)
                        } else {
                            self.execute_api_call_blocking(task, session_id, content)
                        }
                    }
                    _ => ExecuteResult::failure(
                        "Missing required fields: opencode-session-id or content",
                    ),
                }
            }
            // 创建新的 SDK 会话
            "session-create" => match self.executor.create_session(&self.client) {
                Ok(result) => {
                    if let Some(session_id) = result.get("id").and_then(Value::as_str) {
                        ExecuteResult::success(json!({"opencode-session-id": session_id}))
                    } else {
                        ExecuteResult::failure("Failed to create session: no ID returned")
                    }
                }
                Err(error) => ExecuteResult::failure(error),
            },
            // 数据透传，直接返回 payload 中的 data
            "transform" => {
                if let Some(data) = task.payload.get("data") {
                    ExecuteResult::success(data.clone())
                } else {
                    ExecuteResult::failure("Missing transform data")
                }
            }
            // 按路径查询 context 中的嵌套字段
            "query" => {
                let query = task.payload.get("query").and_then(Value::as_array);
                let context = task.payload.get("context");
                match (query, context) {
                    (Some(path), Some(context)) => {
                        let mut current = context;
                        for segment in path {
                            let Some(key) = segment.as_str() else {
                                return ExecuteResult::failure(
                                    "Query path must contain string keys",
                                );
                            };
                            let Some(next) = current.get(key) else {
                                return ExecuteResult::failure("Query path not found in context");
                            };
                            current = next;
                        }
                        ExecuteResult::success(current.clone())
                    }
                    _ => ExecuteResult::failure("Missing query or context"),
                }
            }
            other => ExecuteResult::failure(format!("Unknown task type: {other}")),
        }
    }
}

/// 任务执行的内部结果，仅在 WorkerAgent 内部使用
struct ExecuteResult {
    status: String,
    result: Option<Value>,
    error: Option<String>,
}

/// Panic 安全守卫：无论任务执行是否 panic，都会在 drop 时清除 busy 标志并唤醒等待者。
/// Guard that clears the busy flag and notifies the condvar on drop (including panics).
struct PanicGuard<'a> {
    busy: &'a AtomicBool,
    notify: Option<&'a Condvar>,
}

impl Drop for PanicGuard<'_> {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::SeqCst);
        if let Some(cv) = self.notify {
            cv.notify_one();
        }
    }
}

impl ExecuteResult {
    fn success(result: Value) -> Self {
        Self {
            status: "success".into(),
            result: Some(result),
            error: None,
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            status: "failure".into(),
            result: None,
            error: Some(error.into()),
        }
    }
}

fn classify_error_kind(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else {
        "upstream_error"
    }
}

fn is_streaming_unsupported(error: &str) -> bool {
    error.to_ascii_lowercase().contains("streaming not supported")
}

/// 工作者池，管理一组 WorkerAgent 的生命周期。
/// 使用 round-robin + Condvar 实现负载均衡和等待机制。
pub struct WorkerPool {
    workers: Mutex<Vec<Arc<WorkerAgent>>>,
    worker_available: Arc<Condvar>,
    client: SdkClient,
    executor: Arc<dyn TaskExecutor>,
    runtime_event_publisher: Option<RuntimeEventPublisher>,
    worker_timeout_ms: Option<u64>,
    next_index: AtomicU64,
    max_wait_ms: u64,
    running: AtomicBool,
    min_size: usize,
    max_size: usize,
}

/// WorkerPool 的状态快照
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerPoolStatus {
    pub status: String,
    pub size: usize,
    pub workers: Vec<WorkerStatus>,
}

impl WorkerPool {
    /// 创建工作者池，初始化 `initial_size` 个 Worker（默认 2，最少 1）
    pub fn new(
        client: SdkClient,
        executor: Arc<dyn TaskExecutor>,
        initial_size: Option<usize>,
        timeout_ms: Option<u64>,
        max_wait_ms: Option<u64>,
    ) -> Self {
        let size = initial_size.unwrap_or(2).max(1);
        let mut workers = Vec::with_capacity(size);
        for _ in 0..size {
            workers.push(Arc::new(WorkerAgent::new(
                client.clone(),
                executor.clone(),
                timeout_ms,
            )));
        }
        Self {
            workers: Mutex::new(workers),
            worker_available: Arc::new(Condvar::new()),
            client,
            executor,
            runtime_event_publisher: None,
            worker_timeout_ms: timeout_ms,
            next_index: AtomicU64::new(0),
            max_wait_ms: max_wait_ms.unwrap_or(5_000),
            running: AtomicBool::new(false),
            min_size: 1,
            max_size: 16,
        }
    }

    /// 设置动态伸缩的最小/最大 Worker 数量
    /// Set min/max bounds for dynamic scaling.
    pub fn with_bounds(mut self, min_size: usize, max_size: usize) -> Self {
        self.min_size = min_size.max(1);
        self.max_size = max_size.max(self.min_size);
        self
    }

    pub fn with_runtime_event_publisher(mut self, publisher: RuntimeEventPublisher) -> Self {
        self.runtime_event_publisher = Some(publisher.clone());
        let workers = self.workers.get_mut();
        for worker in workers.iter_mut() {
            if let Some(inner) = Arc::get_mut(worker) {
                inner.runtime_event_publisher = Some(publisher.clone());
            }
        }
        self
    }

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
        let workers = self.workers.lock();
        for worker in workers.iter() {
            worker.start();
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let workers = self.workers.lock();
        for worker in workers.iter() {
            worker.stop();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn size(&self) -> usize {
        self.workers.lock().len()
    }

    pub fn status(&self) -> WorkerPoolStatus {
        let workers = self.workers.lock();
        WorkerPoolStatus {
            status: if self.is_running() {
                "running"
            } else {
                "stopped"
            }
            .to_string(),
            size: workers.len(),
            workers: workers.iter().map(|worker| worker.status()).collect(),
        }
    }

    /// 动态调整池大小到 target（受 min/max 约束），返回调整后的实际大小
    /// Dynamically scale the pool to `target` workers (clamped to min/max bounds).
    pub fn scale_to(&self, target: usize) -> usize {
        let target = target.clamp(self.min_size, self.max_size);
        let mut workers = self.workers.lock();
        let current = workers.len();

        if target > current {
            // Scale up
            for _ in current..target {
                let mut worker = WorkerAgent::new(
                    self.client.clone(),
                    self.executor.clone(),
                    self.worker_timeout_ms,
                );
                if let Some(publisher) = self.runtime_event_publisher.clone() {
                    worker = worker.with_runtime_event_publisher(publisher);
                }
                let worker = Arc::new(worker);
                if self.is_running() {
                    worker.start();
                }
                workers.push(worker);
            }
        } else if target < current {
            // Scale down — stop and remove excess workers from the end
            for _ in target..current {
                if let Some(worker) = workers.pop() {
                    worker.stop();
                }
            }
        }
        workers.len()
    }

    /// 向池中提交任务。会等待可用 Worker（最多 max_wait_ms），超时则返回失败。
    /// 内部使用 Condvar 避免忙轮询，Worker 完成任务后会唤醒等待者。
    pub fn submit_task(&self, task: Task) -> crossbeam_channel::Receiver<TaskResult> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if !self.is_running() {
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Worker pool is not running".into()),
                duration_ms: 0,
                worker_id: None,
                result: None,
            }));
            return rx;
        }

        let started = now_ms();
        let remaining_ms = self.max_wait_ms;

        // Use condvar to wait for an available worker instead of spinning
        let mut dummy = self.workers.lock();
        loop {
            // Drop the lock before scanning workers (next_available_worker takes its own lock)
            drop(dummy);

            if let Some(worker) = self.next_available_worker() {
                return worker.submit_task(task, Some(Arc::clone(&self.worker_available)));
            }

            let elapsed = now_ms().saturating_sub(started);
            if elapsed >= remaining_ms {
                let _ = tx.send(make_task_result(MakeTaskResult {
                    task_id: task.id,
                    trace_id: task.trace_id,
                    status: "failure".into(),
                    error: Some("No available worker".into()),
                    duration_ms: elapsed,
                    worker_id: None,
                    result: None,
                }));
                return rx;
            }

            let wait_ms = (remaining_ms - elapsed).min(50);
            dummy = self.workers.lock();
            self.worker_available
                .wait_for(&mut dummy, Duration::from_millis(wait_ms));
        }
    }

    /// 使用 round-robin 策略查找下一个空闲 Worker
    fn next_available_worker(&self) -> Option<Arc<WorkerAgent>> {
        let workers = self.workers.lock();
        if workers.is_empty() {
            return None;
        }
        let len = workers.len() as u64;
        let start = self.next_index.fetch_add(1, Ordering::SeqCst);
        for offset in 0..len {
            let idx = ((start + offset) % len) as usize;
            let worker = &workers[idx];
            if worker.is_running() && !worker.is_busy() {
                return Some(Arc::clone(worker));
            }
        }
        None
    }
}
