//! Runtime 事件发射器：统一处理 runtime event、timeline、failure、summary 与状态写回

use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::bus::{make_internal, Bus, InboundMessage, MakeInternal};
use crate::runtime::{RuntimeDomainEvent, SharedRuntimeStateStore};
use crate::support::now_ms;
use crate::tasks::{
    RunRecord, RunStatus, SuspensionKind, SuspensionRecord, SuspensionStatus, TaskKind, TaskRecord,
    TaskStatus, TurnRecord, TurnStatus,
};
use crate::tools::execution::ToolInvocationRecord;
use crate::transcript::MessageRecord;

use super::core::{RuntimeErrorInfo, RuntimeTaskPhase};
use super::runtime_helpers::{merge_runtime_metadata, runtime_message_id};
use super::runtime_ids::{
    runtime_requirement_id, runtime_run_id, runtime_suspension_id, runtime_task_id, runtime_turn_id,
};
use super::{
    ExecutionSummary, PendingQuestion, RuntimeFailureSnapshot, RuntimeFailureStatus,
    RuntimeTimelineEvent,
};

pub struct RuntimeEventEmitter {
    bus: Arc<Bus>,
    timeline: Arc<Mutex<Vec<RuntimeTimelineEvent>>>,
    failures: Mutex<RuntimeFailureStatus>,
    execution_summaries: Mutex<Vec<ExecutionSummary>>,
    runtime_state_store: SharedRuntimeStateStore,
}

impl RuntimeEventEmitter {
    pub fn new(
        bus: Arc<Bus>,
        timeline: Arc<Mutex<Vec<RuntimeTimelineEvent>>>,
        runtime_state_store: SharedRuntimeStateStore,
    ) -> Self {
        Self {
            bus,
            timeline,
            failures: Mutex::new(RuntimeFailureStatus::default()),
            execution_summaries: Mutex::new(Vec::new()),
            runtime_state_store,
        }
    }

    pub fn publish(&self, event: &str, inbound_msg: &InboundMessage, payload: Value) {
        let payload = merge_runtime_metadata(inbound_msg, payload);
        let message_id = runtime_message_id(inbound_msg);
        self.record_timeline(event, inbound_msg, payload.clone());
        let _ = self.bus.publish_internal(make_internal(MakeInternal {
            source: "core-agent".into(),
            trace_id: Some(inbound_msg.id.clone()),
            payload: json!({
                "event": event,
                "message_id": message_id,
                "channel": inbound_msg.channel,
                "chat_id": inbound_msg.chat_id,
                "sender_id": inbound_msg.sender_id,
                "payload": payload,
            }),
            ..Default::default()
        }));
    }

    pub fn publish_worker(
        &self,
        event: &str,
        inbound_msg: &InboundMessage,
        worker_snapshots: &[crate::agent::worker::WorkerStatus],
        task_type: &str,
    ) {
        for w in worker_snapshots {
            let status = if event == "task_start" && w.status == "busy" {
                "busy"
            } else if event == "task_end" {
                "idle"
            } else {
                &w.status
            };
            let _ = self.bus.publish_internal(make_internal(MakeInternal {
                source: "core-agent".into(),
                trace_id: Some(inbound_msg.id.clone()),
                payload: json!({
                    "event": format!("worker_{}", event),
                    "worker_id": w.id,
                    "status": status,
                    "task_type": task_type,
                    "channel": inbound_msg.channel,
                    "message_id": inbound_msg.id,
                    "chat_id": inbound_msg.chat_id,
                }),
                ..Default::default()
            }));
        }
    }

    pub fn tool_lifecycle_payload(
        &self,
        invocation: &ToolInvocationRecord,
        details: Value,
    ) -> Value {
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
            "details": details,
        })
    }

    pub fn publish_tool_lifecycle(
        &self,
        event: &str,
        inbound_msg: &InboundMessage,
        payload: Value,
    ) {
        let payload = merge_runtime_metadata(inbound_msg, payload);
        self.record_timeline(event, inbound_msg, payload.clone());
        let _ = self.bus.publish_internal(make_internal(MakeInternal {
            source: "core-agent".into(),
            trace_id: Some(inbound_msg.id.clone()),
            payload: json!({
                "event": event,
                "message_id": runtime_message_id(inbound_msg),
                "channel": inbound_msg.channel,
                "chat_id": inbound_msg.chat_id,
                "sender_id": inbound_msg.sender_id,
                "payload": payload,
            }),
            ..Default::default()
        }));
    }

    pub(crate) fn record_failure(&self, inbound_msg: &InboundMessage, error: &RuntimeErrorInfo) {
        let failure = RuntimeFailureSnapshot {
            error_code: error.code.to_string(),
            error_stage: error.stage.to_string(),
            message_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            occurred_at_ms: now_ms(),
            internal_message: error.internal_message.clone(),
        };
        let mut failures = self.failures.lock();
        let count = failures
            .counts_by_error_code
            .entry(error.code.to_string())
            .or_insert(0);
        *count += 1;
        failures.last_failure = Some(failure);
    }

    pub fn record_timeline(&self, event: &str, inbound_msg: &InboundMessage, payload: Value) {
        let timeline_event = RuntimeTimelineEvent {
            event: event.to_string(),
            trace_id: inbound_msg.id.clone(),
            message_id: runtime_message_id(inbound_msg),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            sender_id: inbound_msg.sender_id.clone(),
            recorded_at_ms: now_ms(),
            payload,
        };
        let mut timeline = self.timeline.lock();
        timeline.push(timeline_event);
        if timeline.len() > 50 {
            let excess = timeline.len() - 50;
            timeline.drain(..excess);
        }
    }

    pub fn record_projection_hint(&self, run_id: String, scope: &str, key: &str, value: Value) {
        self.runtime_state_store
            .append(RuntimeDomainEvent::ProjectionHintRecorded {
                run_id,
                scope: scope.into(),
                key: key.into(),
                value,
            });
    }

    pub fn record_job_lifecycle_hint(
        &self,
        job_id: &str,
        status_label: &str,
        current_step: String,
    ) {
        let run_id = self
            .runtime_state_store
            .snapshot()
            .index
            .run_ids_by_job_id
            .get(job_id)
            .cloned()
            .unwrap_or_else(|| job_id.to_string());
        self.record_projection_hint(
            run_id,
            "job",
            "lifecycle",
            json!({
                "status": status_label,
                "status_label": status_label,
                "current_step": current_step,
            }),
        );
    }

    pub fn record_summary(&self, summary: ExecutionSummary) {
        let run_id = self
            .runtime_state_store
            .snapshot()
            .index
            .run_ids_by_job_id
            .get(&summary.message_id)
            .cloned()
            .unwrap_or_else(|| summary.message_id.clone());
        let summary_value = json!({
            "trace_id": summary.trace_id,
            "message_id": summary.message_id,
            "channel": summary.channel,
            "chat_id": summary.chat_id,
            "plan_type": summary.plan_type,
            "status": summary.status,
            "cache_hit": summary.cache_hit,
            "worker_id": summary.worker_id,
            "error_code": summary.error_code,
            "error_stage": summary.error_stage,
            "task_duration_ms": summary.task_duration_ms,
            "stages": {
                "context_ms": summary.stages.context_ms,
                "session_ms": summary.stages.session_ms,
                "classify_ms": summary.stages.classify_ms,
                "execute_ms": summary.stages.execute_ms,
                "total_ms": summary.stages.total_ms,
            }
        });
        self.runtime_state_store
            .append(RuntimeDomainEvent::ProjectionHintRecorded {
                run_id,
                scope: "execution".into(),
                key: "summary".into(),
                value: summary_value,
            });

        let mut summaries = self.execution_summaries.lock();
        summaries.push(summary);
        if summaries.len() > 20 {
            let excess = summaries.len() - 20;
            summaries.drain(..excess);
        }
    }

    pub fn latest_summary(&self, inbound_msg: &InboundMessage) -> Option<ExecutionSummary> {
        self.execution_summaries
            .lock()
            .iter()
            .rev()
            .find(|summary| {
                summary.trace_id == inbound_msg.id
                    || summary.message_id == runtime_message_id(inbound_msg)
            })
            .cloned()
    }

    pub fn failures_snapshot(&self) -> RuntimeFailureStatus {
        self.failures.lock().clone()
    }

    pub fn timeline_snapshot(&self) -> Vec<RuntimeTimelineEvent> {
        self.timeline.lock().clone()
    }

    pub fn timeline_handle(&self) -> Arc<Mutex<Vec<RuntimeTimelineEvent>>> {
        Arc::clone(&self.timeline)
    }

    pub fn execution_summaries_snapshot(&self) -> Vec<ExecutionSummary> {
        self.execution_summaries.lock().clone()
    }

    pub fn upsert_run(
        &self,
        inbound_msg: &InboundMessage,
        status: RunStatus,
        current_turn_id: Option<String>,
    ) {
        let run_id = runtime_run_id(&inbound_msg.id);
        let snapshot = self.runtime_state_store.snapshot();
        let created_at_ms = snapshot
            .runs
            .get(&run_id)
            .map(|run| run.created_at_ms)
            .unwrap_or_else(now_ms);
        let updated_at_ms = now_ms();
        self.runtime_state_store
            .append(RuntimeDomainEvent::RunUpserted {
                run: RunRecord {
                    run_id,
                    trace_id: inbound_msg.id.clone(),
                    job_id: inbound_msg.id.clone(),
                    chat_id: inbound_msg.chat_id.clone(),
                    channel: inbound_msg.channel.clone(),
                    status,
                    current_turn_id,
                    latest_error: None,
                    created_at_ms,
                    updated_at_ms,
                    completed_at_ms: None,
                },
            });
    }

    pub fn upsert_turn(
        &self,
        inbound_msg: &InboundMessage,
        source: &str,
        status: TurnStatus,
    ) -> String {
        let turn_id = runtime_turn_id(&inbound_msg.id, source);
        let snapshot = self.runtime_state_store.snapshot();
        let started_at_ms = snapshot
            .turns
            .get(&turn_id)
            .map(|turn| turn.started_at_ms)
            .unwrap_or_else(now_ms);
        let updated_at_ms = now_ms();
        self.runtime_state_store
            .append(RuntimeDomainEvent::TurnUpserted {
                turn: TurnRecord {
                    turn_id: turn_id.clone(),
                    run_id: runtime_run_id(&inbound_msg.id),
                    source: source.to_string(),
                    status,
                    transcript_checkpoint: snapshot.transcript.len(),
                    requirement_id: Some(runtime_requirement_id(&inbound_msg.id)),
                    suspension_id: None,
                    started_at_ms,
                    updated_at_ms,
                    completed_at_ms: None,
                },
            });
        self.upsert_run(inbound_msg, RunStatus::Running, Some(turn_id.clone()));
        turn_id
    }

    pub(crate) fn upsert_task(
        &self,
        inbound_msg: &InboundMessage,
        phase: RuntimeTaskPhase,
        status: TaskStatus,
        description: impl Into<String>,
        error: Option<String>,
    ) -> String {
        let task_id = runtime_task_id(&inbound_msg.id, phase);
        let snapshot = self.runtime_state_store.snapshot();
        let existing = snapshot.tasks.get(&task_id).cloned();
        let turn_id = snapshot
            .runs
            .get(&runtime_run_id(&inbound_msg.id))
            .and_then(|run| run.current_turn_id.clone());
        let updated_at_ms = now_ms();
        let (kind, task_type, name) = match phase {
            RuntimeTaskPhase::Main => (TaskKind::Main, "main", "main"),
            RuntimeTaskPhase::Question => (TaskKind::Question, "question", "question"),
            RuntimeTaskPhase::ToolPermission => (
                TaskKind::ToolInvocation,
                "tool_permission",
                "tool_permission",
            ),
            RuntimeTaskPhase::Followup => (TaskKind::Followup, "followup", "followup"),
            RuntimeTaskPhase::Requirement => (TaskKind::Requirement, "requirement", "requirement"),
        };
        self.runtime_state_store
            .append(RuntimeDomainEvent::TaskUpserted {
                task: TaskRecord {
                    task_id: task_id.clone(),
                    run_id: runtime_run_id(&inbound_msg.id),
                    turn_id,
                    parent_task_id: None,
                    trace_id: inbound_msg.id.clone(),
                    job_id: inbound_msg.id.clone(),
                    parent_job_id: None,
                    plan_id: None,
                    kind,
                    name: name.to_string(),
                    task_type: task_type.to_string(),
                    description: description.into(),
                    status,
                    execution_mode: None,
                    result_kind: existing.as_ref().and_then(|task| task.result_kind.clone()),
                    specialist_type: None,
                    dependencies: existing
                        .as_ref()
                        .map(|task| task.dependencies.clone())
                        .unwrap_or_default(),
                    metadata: json!({
                        "channel": inbound_msg.channel.clone(),
                        "chat_id": inbound_msg.chat_id.clone(),
                    }),
                    started_at_ms: existing
                        .as_ref()
                        .and_then(|task| task.started_at_ms)
                        .or(Some(updated_at_ms)),
                    updated_at_ms,
                    completed_at_ms: existing.as_ref().and_then(|task| task.completed_at_ms),
                    error,
                },
            });
        task_id
    }

    pub fn upsert_suspension(
        &self,
        inbound_msg: &InboundMessage,
        question: &PendingQuestion,
        task_id: Option<String>,
        invocation_id: Option<String>,
        kind: SuspensionKind,
        status: SuspensionStatus,
    ) -> String {
        let suspension_id = runtime_suspension_id(&question.question_id);
        let snapshot = self.runtime_state_store.snapshot();
        let created_at_ms = snapshot
            .suspensions
            .get(&suspension_id)
            .map(|record| record.created_at_ms)
            .unwrap_or(question.asked_at_ms);
        let mut raw_payload = question.raw_question.clone();
        if let Value::Object(map) = &mut raw_payload {
            map.entry("opencode_session_id")
                .or_insert_with(|| Value::String(question.opencode_session_id.clone()));
            map.entry("original_user_request")
                .or_insert_with(|| Value::String(inbound_msg.content.clone()));
            map.entry("sender_id")
                .or_insert_with(|| Value::String(inbound_msg.sender_id.clone()));
            if let Some(chat_id) = inbound_msg.chat_id.clone() {
                map.entry("chat_id")
                    .or_insert_with(|| Value::String(chat_id));
            }
            map.entry("channel")
                .or_insert_with(|| Value::String(inbound_msg.channel.clone()));
        }
        self.runtime_state_store
            .append(RuntimeDomainEvent::SuspensionUpserted {
                suspension: SuspensionRecord {
                    suspension_id: suspension_id.clone(),
                    run_id: runtime_run_id(&inbound_msg.id),
                    turn_id: snapshot
                        .runs
                        .get(&runtime_run_id(&inbound_msg.id))
                        .and_then(|run| run.current_turn_id.clone())
                        .unwrap_or_else(|| runtime_turn_id(&inbound_msg.id, "initial")),
                    task_id,
                    question_id: Some(question.question_id.clone()),
                    invocation_id,
                    kind,
                    status,
                    prompt: Some(question.prompt.clone()),
                    options: question.options.clone(),
                    raw_payload,
                    resolution_source: question.resolution_source.clone(),
                    answer_summary: question.answer_summary.clone(),
                    created_at_ms,
                    updated_at_ms: now_ms(),
                    resolved_at_ms: if question.answer_submitted {
                        Some(now_ms())
                    } else {
                        None
                    },
                    cleared_at_ms: None,
                },
            });
        suspension_id
    }

    pub fn append_transcript_message(
        &self,
        inbound_msg: &InboundMessage,
        message: &crate::messages::types::InternalMsg,
    ) {
        let snapshot = self.runtime_state_store.snapshot();
        let turn_id = snapshot
            .runs
            .get(&runtime_run_id(&inbound_msg.id))
            .and_then(|run| run.current_turn_id.clone());
        self.runtime_state_store
            .append(RuntimeDomainEvent::MessageAppended {
                message: MessageRecord::from_internal(
                    runtime_run_id(&inbound_msg.id),
                    turn_id,
                    now_ms(),
                    message,
                ),
            });
    }

    pub fn append_transcript_messages(
        &self,
        inbound_msg: &InboundMessage,
        messages: &[crate::messages::types::InternalMsg],
    ) {
        for message in messages {
            self.append_transcript_message(inbound_msg, message);
        }
    }

    pub fn append_subtask_blocked_transcript(
        &self,
        inbound_msg: &InboundMessage,
        reason: &crate::tasks::SubtaskBlockedReason,
        auto_resolved: bool,
    ) {
        let label = if auto_resolved {
            "subtask_auto_resolved"
        } else {
            "subtask_blocked"
        };
        let text = format!("[{label}] {}", serde_json::to_string(reason).unwrap_or_default());
        let msg = crate::messages::types::InternalMsg {
            role: crate::messages::types::MessageRole::System,
            content: Value::String(text),
            message_id: format!("{}-{label}", runtime_message_id(inbound_msg)),
            tool_use_id: None,
            filtered: false,
            metadata: json!({ "subtask_blocked": true, "auto_resolved": auto_resolved }),
        };
        self.append_transcript_message(inbound_msg, &msg);
    }
}
