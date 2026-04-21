//! 挂起状态协调器：管理 PendingQuestion 和 SuspendedToolInvocation 的生命周期

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use super::event_emitter::RuntimeEventEmitter;
use crate::bus::InboundMessage;
use crate::execution::agentic_loop::{AgenticLoopSuspension, SuspendedToolInvocation};
use crate::execution::turn_coordinator::prepare_question_asked_payload;
use crate::permissions::{PermissionRequest, PermissionRiskLevel};
use crate::runtime::{RuntimeDomainEvent, RuntimeStateSnapshot, SharedRuntimeStateStore};
use crate::support::now_ms;
use crate::tasks::{
    SubtaskBlockedReason, SuspensionKind, SuspensionRecord, SuspensionStatus, TaskStatus,
    ToolInvocationRuntimeRecord,
};
use crate::tools::execution::ToolInvocationRecord;

use super::core::RuntimeTaskPhase;
use super::runtime_helpers::runtime_message_id;
use super::runtime_ids::{runtime_run_id, runtime_task_id};

// ── 公开类型 ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionKind {
    Confirm,
    Choice,
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionDecisionMode {
    AutoAllowed,
    UserRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionRiskLevel {
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingQuestionSourceKind {
    UpstreamQuestion,
    ToolPermission,
    SubtaskBlocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub question_id: String,
    pub job_id: String,
    pub opencode_session_id: String,
    pub question_kind: QuestionKind,
    pub prompt: String,
    pub options: Vec<String>,
    pub raw_question: Value,
    pub decision_mode: QuestionDecisionMode,
    pub risk_level: QuestionRiskLevel,
    pub requires_user_confirmation: bool,
    pub source_kind: PendingQuestionSourceKind,
    pub continuation_token: Option<String>,
    pub asked_at_ms: u64,
    pub answer_submitted: bool,
    pub answer_summary: Option<String>,
    pub resolution_source: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub inbound: Option<InboundMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionAnswerInput {
    pub question_id: String,
    pub source: String,
    pub answer_type: String,
    pub answer: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SuspendedToolInvocationState {
    pub(crate) suspension: AgenticLoopSuspension,
    pub(crate) inbound: InboundMessage,
    pub(crate) opencode_session_id: String,
}

impl SuspendedToolInvocationState {
    pub(crate) fn invocation_id(&self) -> String {
        self.suspension
            .suspended_tool
            .invocation
            .invocation_id
            .clone()
    }

    pub(crate) fn tool_name(&self) -> String {
        self.suspension.suspended_tool.tool_name.clone()
    }

    pub(crate) fn tool_use_id(&self) -> Option<String> {
        Some(self.suspension.suspended_tool.tool_use_id.clone())
    }

    pub(crate) fn input_preview(&self) -> Option<String> {
        Some(
            self.suspension
                .suspended_tool
                .permission_request
                .input_preview
                .clone(),
        )
    }
}

// ── 自由函数 ──────────────────────────────────────────────

pub fn question_kind_str(kind: &QuestionKind) -> &'static str {
    match kind {
        QuestionKind::Confirm => "confirm",
        QuestionKind::Choice => "choice",
        QuestionKind::Input => "input",
    }
}

pub fn question_decision_mode_str(mode: &QuestionDecisionMode) -> &'static str {
    match mode {
        QuestionDecisionMode::AutoAllowed => "auto_allowed",
        QuestionDecisionMode::UserRequired => "user_required",
    }
}

pub fn question_risk_level_str(level: &QuestionRiskLevel) -> &'static str {
    match level {
        QuestionRiskLevel::Low => "low",
        QuestionRiskLevel::High => "high",
    }
}

pub fn permission_risk_to_question_risk(level: &PermissionRiskLevel) -> QuestionRiskLevel {
    match level {
        PermissionRiskLevel::Low => QuestionRiskLevel::Low,
        PermissionRiskLevel::High => QuestionRiskLevel::High,
    }
}

pub(crate) fn detect_pending_question(
    result: Option<&Value>,
    opencode_session_id: &str,
) -> Option<PendingQuestion> {
    let response = result?;

    if response.get("type").and_then(Value::as_str) == Some("question") {
        let question = response.get("question")?;
        return build_pending_question(question, opencode_session_id);
    }

    if let Some(question) = response.get("question") {
        return build_pending_question(question, opencode_session_id);
    }

    None
}

fn build_pending_question(question: &Value, opencode_session_id: &str) -> Option<PendingQuestion> {
    let prompt = question
        .get("prompt")
        .or_else(|| question.get("message"))
        .or_else(|| question.get("text"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let options = question
        .get("options")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| {
                    value
                        .get("value")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| {
                            value
                                .get("label")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        })
                        .or_else(|| value.as_str().map(ToString::to_string))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let question_kind = classify_question_kind(question, &options);
    let decision_mode = classify_question_decision_mode(&prompt, &options);
    let risk_level = classify_question_risk_level(&prompt, &options);
    let requires_user_confirmation = matches!(decision_mode, QuestionDecisionMode::UserRequired)
        || matches!(question_kind, QuestionKind::Confirm);
    let source_kind = if question.get("type").and_then(Value::as_str) == Some("tool_permission") {
        PendingQuestionSourceKind::ToolPermission
    } else {
        PendingQuestionSourceKind::UpstreamQuestion
    };
    Some(PendingQuestion {
        question_id: question
            .get("question_id")
            .or_else(|| question.get("id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        job_id: String::new(),
        opencode_session_id: opencode_session_id.to_string(),
        question_kind,
        prompt,
        options,
        raw_question: question.clone(),
        decision_mode,
        risk_level,
        requires_user_confirmation,
        source_kind,
        continuation_token: None,
        asked_at_ms: now_ms(),
        answer_submitted: false,
        answer_summary: None,
        resolution_source: None,
        inbound: None,
    })
}

fn classify_question_kind(question: &Value, options: &[String]) -> QuestionKind {
    match question.get("kind").and_then(Value::as_str) {
        Some("confirm") => QuestionKind::Confirm,
        Some("choice") => QuestionKind::Choice,
        Some("input") => QuestionKind::Input,
        _ if !options.is_empty() => QuestionKind::Choice,
        _ => QuestionKind::Input,
    }
}

fn classify_question_decision_mode(prompt: &str, options: &[String]) -> QuestionDecisionMode {
    if classify_question_requires_user_confirmation(prompt, options) {
        QuestionDecisionMode::UserRequired
    } else {
        QuestionDecisionMode::AutoAllowed
    }
}

fn classify_question_risk_level(prompt: &str, options: &[String]) -> QuestionRiskLevel {
    if classify_question_requires_user_confirmation(prompt, options) {
        QuestionRiskLevel::High
    } else {
        QuestionRiskLevel::Low
    }
}

pub(crate) fn classify_question_requires_user_confirmation(
    prompt: &str,
    options: &[String],
) -> bool {
    let normalized = format!(
        "{} {}",
        prompt.to_ascii_lowercase(),
        options.join(" ").to_ascii_lowercase()
    );
    [
        "delete",
        "remove",
        "overwrite",
        "rollback",
        "force",
        "deploy",
        "production",
        "database",
        "sql",
        "drop table",
        "grant",
        "permission",
        "credential",
        "secret",
        "token",
        "send email",
        "send message",
        "post",
        "external",
        "write operation",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

// ── SuspensionCoordinator ─────────────────────────────────

pub struct SuspensionCoordinator {
    runtime_state_store: SharedRuntimeStateStore,
    emitter: std::sync::Arc<RuntimeEventEmitter>,
    pending_questions: Mutex<HashMap<String, PendingQuestion>>,
    suspended_tool_invocations: Mutex<HashMap<String, SuspendedToolInvocationState>>,
}

impl SuspensionCoordinator {
    pub fn new(
        runtime_state_store: SharedRuntimeStateStore,
        emitter: std::sync::Arc<RuntimeEventEmitter>,
    ) -> Self {
        Self {
            runtime_state_store,
            emitter,
            pending_questions: Mutex::new(HashMap::new()),
            suspended_tool_invocations: Mutex::new(HashMap::new()),
        }
    }

    pub fn store_question(&self, job_id: String, question: PendingQuestion) {
        let suspension_kind = match question.source_kind {
            PendingQuestionSourceKind::ToolPermission => SuspensionKind::ToolPermission,
            PendingQuestionSourceKind::SubtaskBlocked => SuspensionKind::SubtaskBlocked,
            _ => SuspensionKind::Question,
        };
        let is_tool_permission =
            matches!(question.source_kind, PendingQuestionSourceKind::ToolPermission);
        let invocation_id = question
            .raw_question
            .get("invocation_id")
            .and_then(Value::as_str)
            .map(String::from);
        let task_phase = if is_tool_permission {
            RuntimeTaskPhase::ToolPermission
        } else {
            RuntimeTaskPhase::Question
        };
        let task_id = if let Some(inbound) = question.inbound.as_ref() {
            self.emitter.upsert_task(
                inbound,
                task_phase,
                TaskStatus::Suspended,
                question.prompt.clone(),
                None,
            )
        } else {
            runtime_task_id(&job_id, task_phase)
        };
        if let Some(inbound) = question.inbound.as_ref() {
            self.emitter.upsert_suspension(
                inbound,
                &question,
                Some(task_id),
                invocation_id,
                suspension_kind,
                SuspensionStatus::Active,
            );
        }
        self.pending_questions.lock().insert(job_id, question);
    }

    pub fn register_subtask_blocked(
        &self,
        job_id: &str,
        inbound: &InboundMessage,
        opencode_session_id: &str,
        reason: &SubtaskBlockedReason,
    ) {
        let (prompt, options, question_kind) = match reason {
            SubtaskBlockedReason::MissingParameter { name, description } => (
                format!("子任务需要参数 `{name}`: {description}"),
                Vec::new(),
                QuestionKind::Input,
            ),
            SubtaskBlockedReason::MissingContext { what_needed } => (
                format!("子任务缺少上下文: {what_needed}"),
                Vec::new(),
                QuestionKind::Input,
            ),
            SubtaskBlockedReason::MultipleOptions { options, prompt } => {
                (prompt.clone(), options.clone(), QuestionKind::Choice)
            }
            SubtaskBlockedReason::NeedsDecision { reason } => (
                format!("子任务需要决策: {reason}"),
                Vec::new(),
                QuestionKind::Input,
            ),
        };
        let question_id = Uuid::new_v4().to_string();
        let question = PendingQuestion {
            question_id: question_id.clone(),
            job_id: job_id.to_string(),
            opencode_session_id: opencode_session_id.to_string(),
            question_kind,
            prompt: prompt.clone(),
            options,
            raw_question: serde_json::to_value(reason).unwrap_or_default(),
            decision_mode: QuestionDecisionMode::UserRequired,
            risk_level: QuestionRiskLevel::Low,
            requires_user_confirmation: true,
            source_kind: PendingQuestionSourceKind::SubtaskBlocked,
            continuation_token: Some(question_id),
            asked_at_ms: now_ms(),
            answer_submitted: false,
            answer_summary: None,
            resolution_source: None,
            inbound: Some(inbound.clone()),
        };
        let task_id = self.emitter.upsert_task(
            inbound,
            RuntimeTaskPhase::Question,
            TaskStatus::Suspended,
            prompt,
            None,
        );
        self.emitter.upsert_suspension(
            inbound,
            &question,
            Some(task_id),
            None,
            SuspensionKind::SubtaskBlocked,
            SuspensionStatus::Active,
        );
        self.pending_questions
            .lock()
            .insert(job_id.to_string(), question);
    }

    pub(crate) fn store_tool_invocation(
        &self,
        question_id: String,
        state: SuspendedToolInvocationState,
    ) {
        self.emitter.upsert_task(
            &state.inbound,
            RuntimeTaskPhase::ToolPermission,
            TaskStatus::Suspended,
            format!("awaiting permission for tool {}", state.tool_name()),
            None,
        );
        let tool_permission_question = PendingQuestion {
            question_id: question_id.clone(),
            job_id: runtime_message_id(&state.inbound),
            opencode_session_id: state.opencode_session_id.clone(),
            question_kind: QuestionKind::Confirm,
            prompt: state
                .suspension
                .suspended_tool
                .permission_request
                .prompt
                .clone(),
            options: vec!["allow".into(), "deny".into()],
            raw_question: json!({
                "type": "tool_permission",
                "tool_name": state.tool_name(),
                "tool_use_id": state.tool_use_id(),
                "tool_input": state.suspension.suspended_tool.tool_input,
                "raw_input": state.suspension.suspended_tool.permission_request.raw_input,
                "prompt": state.suspension.suspended_tool.permission_request.prompt,
                "input_preview": state.input_preview(),
                "invocation_id": state.invocation_id(),
            }),
            decision_mode: QuestionDecisionMode::UserRequired,
            risk_level: permission_risk_to_question_risk(
                &state
                    .suspension
                    .suspended_tool
                    .permission_request
                    .risk_level,
            ),
            requires_user_confirmation: true,
            source_kind: PendingQuestionSourceKind::ToolPermission,
            continuation_token: Some(question_id.clone()),
            asked_at_ms: now_ms(),
            answer_submitted: false,
            answer_summary: None,
            resolution_source: None,
            inbound: Some(state.inbound.clone()),
        };
        self.emitter.upsert_suspension(
            &state.inbound,
            &tool_permission_question,
            Some(runtime_task_id(
                &runtime_message_id(&state.inbound),
                RuntimeTaskPhase::ToolPermission,
            )),
            Some(state.invocation_id()),
            SuspensionKind::ToolPermission,
            SuspensionStatus::Active,
        );
        self.upsert_runtime_tool_invocation(
            &state.inbound,
            &state.suspension.suspended_tool.invocation,
            state.input_preview(),
        );
        self.suspended_tool_invocations
            .lock()
            .insert(question_id, state);
    }

    pub fn question_state(&self, job_id: &str, hydrate_cache: bool) -> Option<PendingQuestion> {
        let snapshot = self.runtime_state_store.snapshot();
        if snapshot.index.run_ids_by_job_id.contains_key(job_id) {
            let rebuilt = Self::rebuild_question(&snapshot, job_id);
            let mut pending_questions = self.pending_questions.lock();
            match (&rebuilt, hydrate_cache) {
                (Some(question), true) => {
                    pending_questions.insert(job_id.to_string(), question.clone());
                }
                (Some(_), false) => {}
                (None, _) => {
                    pending_questions.remove(job_id);
                }
            }
            return rebuilt;
        }
        self.pending_questions.lock().get(job_id).cloned()
    }

    pub(crate) fn tool_invocation_state(
        &self,
        question_id: &str,
    ) -> Option<SuspendedToolInvocationState> {
        let snapshot = self.runtime_state_store.snapshot();
        if snapshot
            .index
            .suspension_ids_by_question_id
            .contains_key(question_id)
        {
            let rebuilt = Self::rebuild_tool_invocation(&snapshot, question_id);
            let mut suspended = self.suspended_tool_invocations.lock();
            if let Some(state) = rebuilt.as_ref() {
                suspended.insert(question_id.to_string(), state.clone());
            } else {
                suspended.remove(question_id);
            }
            return rebuilt;
        }
        self.suspended_tool_invocations
            .lock()
            .get(question_id)
            .cloned()
    }

    pub fn clear_question(&self, job_id: &str) {
        let removed = self.pending_questions.lock().remove(job_id);
        let rebuilt = removed
            .clone()
            .or_else(|| self.rebuild_question_from_store(job_id));
        if let Some(question) = rebuilt
            .as_ref()
            .filter(|question| question.inbound.is_some())
        {
            if let Some(inbound) = question.inbound.as_ref() {
                self.emitter.upsert_task(
                    inbound,
                    RuntimeTaskPhase::Question,
                    TaskStatus::Completed,
                    question.prompt.clone(),
                    None,
                );
                let kind = match question.source_kind {
                    PendingQuestionSourceKind::SubtaskBlocked => SuspensionKind::SubtaskBlocked,
                    _ => SuspensionKind::Question,
                };
                self.emitter.upsert_suspension(
                    inbound,
                    question,
                    Some(runtime_task_id(job_id, RuntimeTaskPhase::Question)),
                    None,
                    kind,
                    SuspensionStatus::Cleared,
                );
            }
        }
    }

    pub fn clear_tool_invocation(&self, question_id: &str) {
        let snapshot = self.runtime_state_store.snapshot();
        let existing = snapshot
            .index
            .suspension_ids_by_question_id
            .get(question_id)
            .and_then(|suspension_id| snapshot.suspensions.get(suspension_id))
            .cloned();
        let removed = self.suspended_tool_invocations.lock().remove(question_id);
        let rebuilt = removed
            .clone()
            .or_else(|| self.rebuild_tool_invocation_from_store(question_id));
        if let Some(state) = rebuilt {
            let prompt = state
                .suspension
                .suspended_tool
                .permission_request
                .prompt
                .clone();
            let question = PendingQuestion {
                question_id: question_id.to_string(),
                job_id: runtime_message_id(&state.inbound),
                opencode_session_id: state.opencode_session_id.clone(),
                question_kind: QuestionKind::Confirm,
                prompt,
                options: vec!["allow".into(), "deny".into()],
                raw_question: json!({
                    "type": "tool_permission",
                    "tool_name": state.tool_name(),
                    "tool_use_id": state.tool_use_id(),
                }),
                decision_mode: QuestionDecisionMode::UserRequired,
                risk_level: QuestionRiskLevel::High,
                requires_user_confirmation: true,
                source_kind: PendingQuestionSourceKind::ToolPermission,
                continuation_token: Some(question_id.to_string()),
                asked_at_ms: now_ms(),
                answer_submitted: existing
                    .as_ref()
                    .is_some_and(|record| record.answer_summary.is_some()),
                answer_summary: existing
                    .as_ref()
                    .and_then(|record| record.answer_summary.clone()),
                resolution_source: existing
                    .as_ref()
                    .and_then(|record| record.resolution_source.clone()),
                inbound: Some(state.inbound.clone()),
            };
            self.emitter.upsert_suspension(
                &state.inbound,
                &question,
                Some(runtime_task_id(
                    &runtime_message_id(&state.inbound),
                    RuntimeTaskPhase::ToolPermission,
                )),
                Some(state.invocation_id()),
                SuspensionKind::ToolPermission,
                SuspensionStatus::Cleared,
            );
        }
    }

    pub fn rebuild_question(
        snapshot: &RuntimeStateSnapshot,
        job_id: &str,
    ) -> Option<PendingQuestion> {
        let run = snapshot.runs.get(&runtime_run_id(job_id))?;
        let suspension = snapshot
            .suspensions
            .values()
            .filter(|record| record.run_id == run.run_id)
            .find(|record| matches!(record.status, SuspensionStatus::Active))?;
        let question_id = suspension.question_id.clone()?;
        let raw_question = suspension.raw_payload.clone();
        let source_kind = if suspension.kind == SuspensionKind::ToolPermission
            || raw_question.get("type").and_then(Value::as_str) == Some("tool_permission")
        {
            PendingQuestionSourceKind::ToolPermission
        } else {
            PendingQuestionSourceKind::UpstreamQuestion
        };
        let question_kind = if suspension.options.len() > 2 {
            QuestionKind::Choice
        } else {
            QuestionKind::Confirm
        };
        let risk_level = match raw_question
            .get("risk_level")
            .and_then(Value::as_str)
            .unwrap_or("high")
        {
            "low" => QuestionRiskLevel::Low,
            _ => QuestionRiskLevel::High,
        };
        let decision_mode = match risk_level {
            QuestionRiskLevel::Low => QuestionDecisionMode::AutoAllowed,
            QuestionRiskLevel::High => QuestionDecisionMode::UserRequired,
        };
        Some(PendingQuestion {
            question_id,
            job_id: job_id.to_string(),
            opencode_session_id: suspension
                .raw_payload
                .get("opencode_session_id")
                .and_then(Value::as_str)
                .or_else(|| {
                    suspension
                        .raw_payload
                        .get("session_id")
                        .and_then(Value::as_str)
                })
                .unwrap_or_default()
                .to_string(),
            question_kind,
            prompt: suspension.prompt.clone().unwrap_or_default(),
            options: suspension.options.clone(),
            raw_question,
            decision_mode,
            risk_level,
            requires_user_confirmation: matches!(
                source_kind,
                PendingQuestionSourceKind::ToolPermission
            ) || suspension
                .raw_payload
                .get("requires_user_confirmation")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            source_kind,
            continuation_token: Some(suspension.suspension_id.clone()),
            asked_at_ms: suspension.created_at_ms,
            answer_submitted: matches!(suspension.status, SuspensionStatus::Resolved),
            answer_summary: suspension
                .raw_payload
                .get("answer_summary")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            resolution_source: suspension
                .raw_payload
                .get("resolution_source")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            inbound: Some(InboundMessage {
                id: job_id.to_string(),
                channel: suspension
                    .raw_payload
                    .get("channel")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                sender_id: suspension
                    .raw_payload
                    .get("sender_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                chat_id: run.chat_id.clone(),
                content: suspension
                    .raw_payload
                    .get("original_user_request")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                session_key: None,
                media: Vec::new(),
                metadata: json!({}),
            }),
        })
    }

    fn rebuild_question_from_store(&self, job_id: &str) -> Option<PendingQuestion> {
        let snapshot = self.runtime_state_store.snapshot();
        Self::rebuild_question(&snapshot, job_id)
    }

    pub(crate) fn rebuild_tool_invocation(
        snapshot: &RuntimeStateSnapshot,
        question_id: &str,
    ) -> Option<SuspendedToolInvocationState> {
        let suspension = snapshot
            .index
            .suspension_ids_by_question_id
            .get(question_id)
            .and_then(|suspension_id| snapshot.suspensions.get(suspension_id))
            .filter(|record| matches!(record.status, SuspensionStatus::Active))?;
        let invocation_id = suspension.invocation_id.clone()?;
        let invocation_record = snapshot.tool_invocations.get(&invocation_id)?.clone();
        let run = snapshot.runs.get(&suspension.run_id)?;
        let tool_name = invocation_record.tool_name.clone();
        let tool_use_id = invocation_record.tool_use_id.clone().or_else(|| {
            suspension
                .raw_payload
                .get("tool_use_id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })?;
        let tool_input = suspension
            .raw_payload
            .get("tool_input")
            .cloned()
            .or_else(|| suspension.raw_payload.get("raw_input").cloned())
            .unwrap_or(Value::Null);
        let risk_level = match suspension
            .raw_payload
            .get("risk_level")
            .and_then(Value::as_str)
            .unwrap_or("high")
        {
            "low" => PermissionRiskLevel::Low,
            _ => PermissionRiskLevel::High,
        };
        let turn_messages = Self::rebuild_turn_messages(snapshot, suspension);
        Some(SuspendedToolInvocationState {
            suspension: AgenticLoopSuspension {
                suspended_tool: SuspendedToolInvocation {
                    tool_use_id,
                    tool_name: tool_name.clone(),
                    tool_input: tool_input.clone(),
                    permission_request: PermissionRequest {
                        tool_name,
                        prompt: suspension.prompt.clone().unwrap_or_default(),
                        options: suspension.options.clone(),
                        risk_level,
                        requires_user_confirmation: true,
                        raw_input: tool_input.clone(),
                        input_preview: invocation_record.input_preview.clone().unwrap_or_default(),
                    },
                    invocation: ToolInvocationRecord {
                        invocation_id,
                        tool_name: invocation_record.tool_name.clone(),
                        tool_use_id: Some(invocation_record.tool_use_id.unwrap_or_default()),
                        phase: serde_json::from_str(&format!("\"{}\"", invocation_record.phase))
                            .unwrap_or(crate::tools::execution::ToolInvocationPhase::Detected),
                        permission_state: serde_json::from_str(&format!(
                            "\"{}\"",
                            invocation_record.permission_state
                        ))
                        .unwrap_or(crate::tools::execution::ToolPermissionState::Checking),
                        started_at_ms: invocation_record.started_at_ms,
                        finished_at_ms: invocation_record.finished_at_ms,
                        result_summary: invocation_record.result_summary,
                        error_summary: invocation_record.error_summary,
                    },
                },
                messages: turn_messages,
                iterations: 0,
                compact_count: 0,
            },
            inbound: InboundMessage {
                id: run.job_id.clone(),
                channel: suspension
                    .raw_payload
                    .get("channel")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                sender_id: suspension
                    .raw_payload
                    .get("sender_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                chat_id: run.chat_id.clone(),
                content: suspension
                    .raw_payload
                    .get("original_user_request")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                session_key: None,
                media: Vec::new(),
                metadata: json!({}),
            },
            opencode_session_id: suspension
                .raw_payload
                .get("opencode_session_id")
                .and_then(Value::as_str)
                .or_else(|| {
                    suspension
                        .raw_payload
                        .get("session_id")
                        .and_then(Value::as_str)
                })
                .unwrap_or_default()
                .to_string(),
        })
    }

    fn rebuild_tool_invocation_from_store(
        &self,
        question_id: &str,
    ) -> Option<SuspendedToolInvocationState> {
        let snapshot = self.runtime_state_store.snapshot();
        Self::rebuild_tool_invocation(&snapshot, question_id)
    }

    pub fn rebuild_turn_messages(
        snapshot: &RuntimeStateSnapshot,
        suspension: &SuspensionRecord,
    ) -> Vec<crate::messages::types::InternalMsg> {
        let effective_turn_id: &str = &suspension.turn_id;
        let effective_turn_id = if effective_turn_id.is_empty() {
            snapshot
                .runs
                .get(&suspension.run_id)
                .and_then(|run| run.current_turn_id.as_deref())
        } else {
            Some(effective_turn_id)
        };
        let turn_checkpoint = effective_turn_id
            .and_then(|tid| snapshot.turns.get(tid))
            .map(|turn| turn.transcript_checkpoint)
            .unwrap_or(0);
        snapshot
            .transcript
            .iter()
            .skip(turn_checkpoint)
            .filter(|record| {
                record.run_id == suspension.run_id && record.turn_id.as_deref() == effective_turn_id
            })
            .map(|record| record.to_internal())
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_question_asked(
        &self,
        publish_runtime_event: &dyn Fn(&str, &InboundMessage, Value),
        inbound_msg: &InboundMessage,
        key: &str,
        question: &PendingQuestion,
        worker_id: Option<String>,
        total_ms: u64,
        task_duration_ms: u64,
        cache_hit: bool,
        plan_type: &str,
    ) {
        let asked_payload = prepare_question_asked_payload(question);
        if matches!(question.decision_mode, QuestionDecisionMode::UserRequired) {
            publish_runtime_event(
                "question_escalated_to_user",
                inbound_msg,
                json!({
                    "memory_key": key,
                    "plan_type": plan_type,
                    "cached": cache_hit,
                    "worker_id": worker_id,
                    "task_duration_ms": task_duration_ms,
                    "message_latency_ms": total_ms,
                    "question_id": asked_payload.question_id,
                    "question_kind": question_kind_str(&question.question_kind),
                    "prompt": asked_payload.prompt.clone(),
                    "options": asked_payload.options.clone(),
                    "decision_mode": question_decision_mode_str(&question.decision_mode),
                    "risk_level": question_risk_level_str(&question.risk_level),
                    "requires_user_confirmation": asked_payload.requires_user_confirmation,
                    "opencode_session_id": asked_payload.opencode_session_id.clone(),
                    "raw_question": asked_payload.raw_question.clone(),
                }),
            );
        }
        publish_runtime_event(
            "question_asked",
            inbound_msg,
            json!({
                "memory_key": key,
                "plan_type": plan_type,
                "cached": cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": total_ms,
                "question_id": asked_payload.question_id,
                "question_kind": question_kind_str(&question.question_kind),
                "prompt": asked_payload.prompt,
                "options": asked_payload.options,
                "decision_mode": question_decision_mode_str(&question.decision_mode),
                "risk_level": question_risk_level_str(&question.risk_level),
                "requires_user_confirmation": asked_payload.requires_user_confirmation,
                "opencode_session_id": asked_payload.opencode_session_id,
                "raw_question": asked_payload.raw_question,
            }),
        );
    }

    pub fn evict_cache_for_testing(&self, job_id: &str) {
        if let Some(question) = self.pending_questions.lock().remove(job_id) {
            self.suspended_tool_invocations
                .lock()
                .remove(&question.question_id);
        }
    }

    pub fn submit_answer(
        &self,
        job_id: &str,
        answer: &QuestionAnswerInput,
        answer_summary: String,
    ) -> Result<PendingQuestion, String> {
        let mut pending = self
            .question_state(job_id, true)
            .ok_or_else(|| format!("No pending question for job: {job_id}"))?;

        if pending.question_id != answer.question_id {
            return Err(format!(
                "Question ID mismatch for job {job_id}: expected {}, got {}",
                pending.question_id, answer.question_id
            ));
        }

        pending.answer_submitted = true;
        pending.answer_summary = Some(answer_summary);
        pending.resolution_source = Some(answer.source.clone());
        self.pending_questions
            .lock()
            .insert(job_id.to_string(), pending.clone());

        let inbound = pending.inbound.as_ref().ok_or_else(|| {
            format!("Missing inbound context for pending question on job: {job_id}")
        })?;

        if matches!(
            pending.source_kind,
            PendingQuestionSourceKind::UpstreamQuestion
                | PendingQuestionSourceKind::SubtaskBlocked
        ) {
            let kind = match pending.source_kind {
                PendingQuestionSourceKind::SubtaskBlocked => SuspensionKind::SubtaskBlocked,
                _ => SuspensionKind::Question,
            };
            self.emitter.upsert_suspension(
                inbound,
                &pending,
                Some(runtime_task_id(job_id, RuntimeTaskPhase::Question)),
                None,
                kind,
                SuspensionStatus::Resolved,
            );
            self.emitter.upsert_task(
                inbound,
                RuntimeTaskPhase::Question,
                TaskStatus::Completed,
                format!("question answered: {}", pending.prompt),
                None,
            );
        }

        Ok(pending)
    }

    // ── 内部辅助 ──────────────────────────────────────────

    fn upsert_runtime_tool_invocation(
        &self,
        inbound_msg: &InboundMessage,
        invocation: &ToolInvocationRecord,
        input_preview: Option<String>,
    ) {
        let snapshot = self.runtime_state_store.snapshot();
        self.runtime_state_store
            .append(RuntimeDomainEvent::ToolInvocationUpserted {
                invocation: ToolInvocationRuntimeRecord {
                    invocation_id: invocation.invocation_id.clone(),
                    run_id: runtime_run_id(&inbound_msg.id),
                    turn_id: snapshot
                        .runs
                        .get(&runtime_run_id(&inbound_msg.id))
                        .and_then(|run| run.current_turn_id.clone()),
                    task_id: Some(runtime_task_id(
                        &inbound_msg.id,
                        RuntimeTaskPhase::ToolPermission,
                    )),
                    tool_name: invocation.tool_name.clone(),
                    tool_use_id: invocation.tool_use_id.clone(),
                    phase: serde_json::to_string(&invocation.phase)
                        .unwrap_or_else(|_| "\"unknown\"".into())
                        .trim_matches('"')
                        .to_string(),
                    permission_state: serde_json::to_string(&invocation.permission_state)
                        .unwrap_or_else(|_| "\"unknown\"".into())
                        .trim_matches('"')
                        .to_string(),
                    input_preview,
                    result_summary: invocation.result_summary.clone(),
                    error_summary: invocation.error_summary.clone(),
                    started_at_ms: invocation.started_at_ms,
                    finished_at_ms: invocation.finished_at_ms,
                },
            });
    }
}
