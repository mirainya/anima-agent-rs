use crate::jobs_store::JobStore;
use crate::jobs_types::{
    AcceptedJob, JobEventView, JobKind, JobReviewView, JobStatus, JobView, OrchestrationView,
    QuestionView, ToolStateView, WorkerTaskView,
};
use anima_runtime::agent::{
    CurrentTaskInfo, ExecutionSummary, RuntimeFailureSnapshot, RuntimeTimelineEvent, WorkerStatus,
};
use anima_runtime::runtime::{
    completed_current_step, creating_session_current_step, executing_plan_current_step,
    failed_current_step, followup_current_step, planning_ready_current_step,
    planning_stalled_current_step, preparing_context_current_step, queued_current_step,
    session_ready_current_step, tool_execution_failed_current_step,
    tool_execution_finished_current_step, tool_permission_resolved_current_step,
    tool_phase_current_step, tool_result_recorded_current_step, waiting_user_input_current_step,
    worker_executing_current_step, ProjectionJobStatusSummary, RuntimeProjectionView,
    RuntimeStateSnapshot,
};
use anima_runtime::support::now_ms;
use anima_runtime::tasks::{tasks_for_job, RunRecord};
use serde_json::Value;
use std::collections::HashMap;

const MAX_JOB_EVENTS: usize = 8;
const UPSTREAM_INPUT_STALL_MS: u64 = 15_000;

pub(crate) fn match_worker(job_id: &str, workers: &[WorkerStatus]) -> Option<WorkerTaskView> {
    let now = now_ms();
    workers.iter().find_map(|worker| {
        let task = worker.current_task.as_ref()?;
        if current_task_matches(job_id, task) {
            Some(WorkerTaskView {
                worker_id: worker.id.clone(),
                status: worker.status.clone(),
                task_id: task.task_id.clone(),
                trace_id: task.trace_id.clone(),
                task_type: task.task_type.clone(),
                elapsed_ms: now.saturating_sub(task.started_ms),
                content_preview: task.content_preview.clone(),
                phase: Some(task.phase.clone()),
            })
        } else {
            None
        }
    })
}

pub(crate) fn job_status_from_label(label: &str) -> JobStatus {
    match label {
        "queued" => JobStatus::Queued,
        "preparing_context" => JobStatus::PreparingContext,
        "creating_session" => JobStatus::CreatingSession,
        "planning" => JobStatus::Planning,
        "executing" => JobStatus::Executing,
        "waiting_user_input" => JobStatus::WaitingUserInput,
        "stalled" => JobStatus::Stalled,
        "completed" => JobStatus::Completed,
        "failed" => JobStatus::Failed,
        _ => JobStatus::Executing,
    }
}

pub(crate) fn question_kind_label(kind: &anima_runtime::agent::QuestionKind) -> String {
    match kind {
        anima_runtime::agent::QuestionKind::Confirm => "confirm".into(),
        anima_runtime::agent::QuestionKind::Choice => "choice".into(),
        anima_runtime::agent::QuestionKind::Input => "input".into(),
    }
}

pub(crate) fn question_decision_mode_label(
    mode: &anima_runtime::agent::QuestionDecisionMode,
) -> String {
    match mode {
        anima_runtime::agent::QuestionDecisionMode::AutoAllowed => "auto_allowed".into(),
        anima_runtime::agent::QuestionDecisionMode::UserRequired => "user_required".into(),
    }
}

pub(crate) fn question_risk_level_label(level: &anima_runtime::agent::QuestionRiskLevel) -> String {
    match level {
        anima_runtime::agent::QuestionRiskLevel::Low => "low".into(),
        anima_runtime::agent::QuestionRiskLevel::High => "high".into(),
    }
}

pub(crate) fn runtime_status_with_worker(
    summary: &ProjectionJobStatusSummary,
    worker: &WorkerTaskView,
) -> Option<(JobStatus, String, String)> {
    if matches!(
        summary.status_label.as_str(),
        "completed" | "failed" | "waiting_user_input"
    ) {
        return None;
    }

    if worker.task_type == "session-create" {
        return Some((
            JobStatus::CreatingSession,
            "creating_session".into(),
            creating_session_current_step(worker.phase.as_deref()),
        ));
    }

    if matches!(
        summary.status_label.as_str(),
        "executing" | "planning" | "preparing_context"
    ) {
        let current_step = worker_executing_current_step(&worker.task_type, worker.phase.as_deref());
        return Some((JobStatus::Executing, "executing".into(), current_step));
    }

    None
}

pub(crate) fn derive_job_hierarchy(
    accepted_job: Option<&AcceptedJob>,
    events: &[RuntimeTimelineEvent],
    trace_id: &str,
    store: &JobStore,
    runtime_snapshot: &RuntimeStateSnapshot,
    job_id: &str,
) -> (JobKind, Option<String>) {
    let self_job_id = events.first().map(|event| event.message_id.as_str());
    if let Some(parent_job_id) = runtime_parent_job_id(runtime_snapshot, job_id) {
        if Some(parent_job_id.as_str()) != self_job_id {
            return (JobKind::Subtask, Some(parent_job_id));
        }
    }

    if let Some(parent_job_id) = event_parent_job_id(events) {
        if Some(parent_job_id.as_str()) != self_job_id {
            return (JobKind::Subtask, Some(parent_job_id));
        }
    }

    if let Some(job) = accepted_job {
        return (job.kind.clone(), job.parent_job_id.clone());
    }

    if let Some(parent_job_id) = store.parent_job_id_by_trace(trace_id) {
        return (JobKind::Subtask, Some(parent_job_id));
    }

    (JobKind::Main, None)
}

fn event_parent_job_id(events: &[RuntimeTimelineEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        event
            .payload
            .get("parent_job_id")
            .and_then(|value| value.as_str())
            .map(ToString::to_string)
    })
}

fn current_task_matches(job_id: &str, task: &CurrentTaskInfo) -> bool {
    task.trace_id == job_id || task.task_id == job_id
}

pub(crate) fn derive_orchestration_view(
    job: &JobView,
    child_job_ids: &[String],
    child_ids_by_parent: &HashMap<String, Vec<String>>,
    runtime_snapshot: &RuntimeStateSnapshot,
    runtime_projection: &RuntimeProjectionView,
) -> Option<OrchestrationView> {
    if let Some(runtime_plan) = runtime_projection.orchestration.get(&job.job_id) {
        return Some(OrchestrationView {
            plan_id: runtime_plan.plan_id.clone(),
            active_subtask_name: runtime_plan.active_subtask_name.clone(),
            active_subtask_type: runtime_plan.active_subtask_type.clone(),
            active_subtask_id: runtime_plan.active_subtask_id.clone(),
            total_subtasks: runtime_plan.total_subtasks,
            active_subtasks: runtime_plan.active_subtasks,
            completed_subtasks: runtime_plan.completed_subtasks,
            failed_subtasks: runtime_plan.failed_subtasks,
            child_job_ids: child_job_ids.to_vec(),
        });
    }

    let plan_id = tasks_for_job(runtime_snapshot, &job.job_id)
        .into_iter()
        .find_map(|task| task.plan_id.clone())
        .or_else(|| {
            latest_payload_string(
                &job.recent_events,
                &["orchestration_plan_created"],
                "plan_id",
            )
        });
    let active_subtask_name = latest_payload_string(
        &job.recent_events,
        &["orchestration_subtask_started"],
        "subtask_name",
    );
    let active_subtask_id = latest_payload_string(
        &job.recent_events,
        &["orchestration_subtask_started"],
        "subtask_id",
    );
    let active_subtask_type = latest_payload_string(
        &job.recent_events,
        &["orchestration_subtask_started"],
        "original_task_type",
    );
    let total_subtasks = latest_payload_usize(
        &job.recent_events,
        &["orchestration_plan_created"],
        "subtask_count",
    )
    .unwrap_or(child_job_ids.len());
    let completed_subtasks = count_events(&job.recent_events, "orchestration_subtask_completed");
    let failed_subtasks = count_events(&job.recent_events, "orchestration_subtask_failed");
    let active_subtasks = usize::from(active_subtask_name.is_some() && failed_subtasks == 0);

    let has_orchestration_signal = plan_id.is_some()
        || child_ids_by_parent.contains_key(&job.job_id)
        || job
            .recent_events
            .iter()
            .any(|event| event.event.starts_with("orchestration_"));

    if !has_orchestration_signal {
        return None;
    }

    Some(OrchestrationView {
        plan_id,
        active_subtask_name,
        active_subtask_type,
        active_subtask_id,
        total_subtasks,
        active_subtasks,
        completed_subtasks,
        failed_subtasks,
        child_job_ids: child_job_ids.to_vec(),
    })
}

pub(crate) fn collect_recent_job_events(events: &[RuntimeTimelineEvent]) -> Vec<JobEventView> {
    let mut recent_events_source = events
        .iter()
        .rev()
        .take(MAX_JOB_EVENTS)
        .cloned()
        .collect::<Vec<_>>();
    let retained_plan_created = recent_events_source
        .iter()
        .any(|event| event.event == "orchestration_plan_created");
    let injected_plan_created = if !retained_plan_created {
        events
            .iter()
            .rev()
            .find(|event| event.event == "orchestration_plan_created")
            .cloned()
            .map(|event| {
                recent_events_source.push(event);
                true
            })
            .unwrap_or(false)
    } else {
        false
    };

    recent_events_source.sort_by_key(|event| event.recorded_at_ms);
    if recent_events_source.len() > MAX_JOB_EVENTS {
        if injected_plan_created {
            while recent_events_source.len() > MAX_JOB_EVENTS {
                if let Some(index) = recent_events_source
                    .iter()
                    .position(|event| event.event != "orchestration_plan_created")
                {
                    recent_events_source.remove(index);
                } else {
                    recent_events_source.drain(..recent_events_source.len() - MAX_JOB_EVENTS);
                }
            }
        } else {
            recent_events_source.drain(..recent_events_source.len() - MAX_JOB_EVENTS);
        }
    }

    recent_events_source
        .into_iter()
        .map(|event| JobEventView {
            event: event.event,
            recorded_at_ms: event.recorded_at_ms,
            payload: event.payload,
        })
        .collect()
}

fn runtime_parent_job_id(snapshot: &RuntimeStateSnapshot, job_id: &str) -> Option<String> {
    tasks_for_job(snapshot, job_id)
        .into_iter()
        .find_map(|task| task.parent_job_id.clone())
}

fn latest_payload_string(events: &[JobEventView], event_names: &[&str], key: &str) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event_names.contains(&event.event.as_str()))
        .and_then(|event| event.payload.get(key))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn count_events(events: &[JobEventView], event_name: &str) -> usize {
    events
        .iter()
        .filter(|event| event.event == event_name)
        .count()
}

pub(crate) fn derive_pending_question(events: &[RuntimeTimelineEvent]) -> Option<QuestionView> {
    let asked = events
        .iter()
        .rev()
        .find(|event| event.event == "question_asked")?;
    let question_id = asked
        .payload
        .get("question_id")
        .and_then(|value| value.as_str())?
        .to_string();

    let resolved = events.iter().rev().find(|event| {
        event.event == "question_resolved"
            && event
                .payload
                .get("question_id")
                .and_then(|value| value.as_str())
                == Some(question_id.as_str())
    });
    if resolved.is_some() {
        return None;
    }

    let submitted = events.iter().rev().find(|event| {
        event.event == "question_answer_submitted"
            && event
                .payload
                .get("question_id")
                .and_then(|value| value.as_str())
                == Some(question_id.as_str())
    });
    let permission_requested = events.iter().rev().find(|event| {
        event.event == "tool_permission_requested"
            && event
                .payload
                .get("question_id")
                .and_then(|value| value.as_str())
                == Some(question_id.as_str())
    });
    let raw_question = asked
        .payload
        .get("raw_question")
        .cloned()
        .unwrap_or(Value::Null);
    let normalized_raw_question =
        normalize_tool_permission_raw_question(raw_question, permission_requested);

    Some(QuestionView {
        question_id,
        question_kind: asked
            .payload
            .get("question_kind")
            .and_then(|value| value.as_str())
            .unwrap_or("input")
            .to_string(),
        prompt: asked
            .payload
            .get("prompt")
            .and_then(|value| value.as_str())
            .or_else(|| normalized_raw_question.get("prompt").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        options: asked
            .payload
            .get("options")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        raw_question: normalized_raw_question,
        decision_mode: asked
            .payload
            .get("decision_mode")
            .and_then(|value| value.as_str())
            .unwrap_or("user_required")
            .to_string(),
        risk_level: asked
            .payload
            .get("risk_level")
            .and_then(|value| value.as_str())
            .or_else(|| {
                permission_requested
                    .and_then(|event| event.payload.get("risk_level"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("high")
            .to_string(),
        requires_user_confirmation: asked
            .payload
            .get("requires_user_confirmation")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        opencode_session_id: asked
            .payload
            .get("opencode_session_id")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        answer_summary: submitted
            .and_then(|event| event.payload.get("answer_summary"))
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        resolution_source: submitted
            .and_then(|event| event.payload.get("resolution_source"))
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_job_status(
    events: &[RuntimeTimelineEvent],
    pending_question: Option<&QuestionView>,
    tool_state: Option<&ToolStateView>,
    summary: Option<&ExecutionSummary>,
    failure: Option<&RuntimeFailureSnapshot>,
    _review: Option<&JobReviewView>,
    worker: Option<&WorkerTaskView>,
    idle_since_update_ms: u64,
) -> (JobStatus, String, String) {
    let has_message_received = has_event(events, "message_received");
    let has_session_ready = has_event(events, "session_ready");
    let has_plan_built = has_event(events, "plan_built");
    let has_requirement_satisfied = has_event(events, "requirement_satisfied");
    let has_message_completed = has_event(events, "message_completed");
    let has_message_failed = has_event(events, "message_failed");
    let has_session_create_failed = has_event(events, "session_create_failed");
    let has_followup_scheduled = has_event(events, "requirement_followup_scheduled");
    let has_followup_exhausted = has_event(events, "requirement_followup_exhausted");
    let has_user_input_required = has_event(events, "user_input_required");
    let last_event_name = events.last().map(|event| event.event.as_str());
    let summary_status = summary.map(|item| item.status.as_str());

    if failure.is_some()
        || has_message_failed
        || has_session_create_failed
        || has_followup_exhausted
        || matches!(summary_status, Some("failure" | "followup_exhausted"))
    {
        return (JobStatus::Failed, "failed".into(), failed_current_step());
    }

    if has_message_completed && has_requirement_satisfied {
        return (
            JobStatus::Completed,
            "completed".into(),
            completed_current_step(),
        );
    }

    if let Some(question) = pending_question {
        let is_tool_permission =
            question.raw_question.get("type").and_then(Value::as_str) == Some("tool_permission");
        let current_step = if has_event(events, "question_answer_submitted") {
            if is_tool_permission {
                "已提交工具权限决定，等待主 agent 继续处理".to_string()
            } else {
                "已提交回答，等待主 agent 继续处理".to_string()
            }
        } else if is_tool_permission {
            "等待用户确认工具调用权限".to_string()
        } else {
            waiting_user_input_current_step()
        };
        return (
            JobStatus::WaitingUserInput,
            "waiting_user_input".into(),
            if has_user_input_required {
                current_step
            } else if question.decision_mode == "auto_allowed" {
                "存在待确认 question，但仍需用户介入".to_string()
            } else {
                current_step
            },
        );
    }

    if let Some(worker) = worker {
        if worker.task_type == "session-create" {
            return (
                JobStatus::CreatingSession,
                "creating_session".into(),
                worker
                    .phase
                    .as_ref()
                    .map(|phase| format!("正在创建上游会话（{phase}）"))
                    .unwrap_or_else(|| "正在创建上游会话".into()),
            );
        }
        let current_step = worker_executing_current_step(&worker.task_type, worker.phase.as_deref());
        return (JobStatus::Executing, "executing".into(), current_step);
    }

    if let Some(tool_state) = tool_state {
        let tool_label = tool_state.tool_name.as_deref().unwrap_or("工具");
        let current_step = match tool_state.phase.as_str() {
            "permission_resolved" => tool_permission_resolved_current_step(),
            "execution_started" | "executing" => format!("正在执行工具：{tool_label}"),
            "execution_finished" | "completed" => tool_execution_finished_current_step(),
            "execution_failed" | "failed" => tool_execution_failed_current_step(),
            "result_recorded" => tool_result_recorded_current_step(),
            other => tool_phase_current_step(other),
        };
        return (JobStatus::Executing, "executing".into(), current_step);
    }

    if has_followup_scheduled || matches!(summary_status, Some("followup_pending")) {
        return (
            JobStatus::Executing,
            "executing".into(),
            followup_current_step(),
        );
    }

    if matches!(
        last_event_name,
        Some(
            "cache_hit"
                | "cache_miss"
                | "upstream_response_observed"
                | "requirement_evaluation_started"
                | "requirement_unsatisfied"
        )
    ) {
        let plan_type = summary
            .map(|item| item.plan_type.clone())
            .or_else(|| last_event_payload_value(events, "plan_built", "plan_type"))
            .unwrap_or_else(|| "single".into());
        return (
            JobStatus::Executing,
            "executing".into(),
            executing_plan_current_step(&plan_type),
        );
    }

    if has_plan_built {
        let plan_type = summary
            .map(|item| item.plan_type.clone())
            .or_else(|| last_event_payload_value(events, "plan_built", "plan_type"))
            .unwrap_or_else(|| "single".into());

        if matches!(last_event_name, Some("plan_built"))
            && idle_since_update_ms >= UPSTREAM_INPUT_STALL_MS
        {
            return (
                JobStatus::Stalled,
                "stalled".into(),
                planning_stalled_current_step(),
            );
        }

        return (
            JobStatus::Planning,
            "planning".into(),
            planning_ready_current_step(&plan_type),
        );
    }

    if has_session_ready {
        return (
            JobStatus::Planning,
            "planning".into(),
            session_ready_current_step(),
        );
    }

    if has_message_received {
        return (
            JobStatus::PreparingContext,
            "preparing_context".into(),
            preparing_context_current_step(),
        );
    }

    (JobStatus::Queued, "queued".into(), queued_current_step())
}

pub(crate) fn derive_tool_state(
    events: &[RuntimeTimelineEvent],
    pending_question: Option<&QuestionView>,
) -> Option<ToolStateView> {
    let event = events.iter().rev().find(|event| {
        matches!(
            event.event.as_str(),
            "tool_result_recorded"
                | "tool_execution_failed"
                | "tool_execution_finished"
                | "tool_execution_started"
                | "tool_permission_resolved"
                | "tool_permission_requested"
                | "tool_invocation_detected"
        )
    })?;
    let tool_invocation = event.payload.get("tool_invocation");
    let details = event.payload.get("details");
    let raw_tool_name = event
        .payload
        .get("tool_name")
        .and_then(Value::as_str)
        .or_else(|| {
            tool_invocation
                .and_then(|value| value.get("tool_name"))
                .and_then(Value::as_str)
        })
        .or_else(|| pending_question.and_then(tool_permission_tool_name));
    let raw_tool_use_id = event
        .payload
        .get("tool_use_id")
        .and_then(Value::as_str)
        .or_else(|| {
            tool_invocation
                .and_then(|value| value.get("tool_use_id"))
                .and_then(Value::as_str)
        })
        .or_else(|| pending_question.and_then(tool_permission_tool_use_id));
    let raw_invocation_id = event
        .payload
        .get("invocation_id")
        .and_then(Value::as_str)
        .or_else(|| {
            tool_invocation
                .and_then(|value| value.get("invocation_id"))
                .and_then(Value::as_str)
        });
    let permission_state = event
        .payload
        .get("permission_state")
        .and_then(Value::as_str)
        .or_else(|| {
            tool_invocation
                .and_then(|value| value.get("permission_state"))
                .and_then(Value::as_str)
        });
    let result_preview = event
        .payload
        .get("result_summary")
        .and_then(Value::as_str)
        .or_else(|| {
            tool_invocation
                .and_then(|value| value.get("result_summary"))
                .and_then(Value::as_str)
        });
    let error = event
        .payload
        .get("error_summary")
        .and_then(Value::as_str)
        .or_else(|| details.and_then(|value| value.get("error")).and_then(Value::as_str))
        .or_else(|| {
            tool_invocation
                .and_then(|value| value.get("error_summary"))
                .and_then(Value::as_str)
        });
    let input_preview = details
        .and_then(|value| value.get("tool_input"))
        .map(json_preview)
        .or_else(|| event.payload.get("tool_input").map(json_preview))
        .or_else(|| pending_question.and_then(tool_permission_input_preview));
    let phase = event
        .payload
        .get("phase")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| map_tool_event_to_phase(event.event.as_str()).to_string());
    let awaits_user_confirmation = pending_question
        .map(|question| {
            question.requires_user_confirmation
                && question.raw_question.get("type").and_then(Value::as_str)
                    == Some("tool_permission")
        })
        .unwrap_or(false);

    Some(ToolStateView {
        invocation_id: raw_invocation_id.map(ToString::to_string),
        tool_name: raw_tool_name.map(ToString::to_string),
        tool_use_id: raw_tool_use_id.map(ToString::to_string),
        phase,
        permission_state: permission_state.map(ToString::to_string),
        input_preview,
        result_preview: result_preview.map(ToString::to_string),
        error: error.map(ToString::to_string),
        awaits_user_confirmation,
    })
}

fn normalize_tool_permission_raw_question(
    raw_question: Value,
    permission_requested: Option<&RuntimeTimelineEvent>,
) -> Value {
    let mut object = match raw_question {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => return other,
    };

    let is_tool_permission = object.get("type").and_then(Value::as_str) == Some("tool_permission")
        || permission_requested.is_some();
    if !is_tool_permission {
        return Value::Object(object);
    }

    object
        .entry("type")
        .or_insert_with(|| Value::String("tool_permission".into()));
    if let Some(event) = permission_requested {
        copy_string_field_if_missing(&mut object, &event.payload, "tool_name");
        copy_string_field_if_missing(&mut object, &event.payload, "tool_use_id");
        copy_value_field_if_missing(&mut object, &event.payload, "tool_input");
        copy_string_field_if_missing(&mut object, &event.payload, "prompt");
        if !object.contains_key("input_preview") {
            if let Some(value) = event.payload.get("tool_input") {
                object.insert("input_preview".into(), Value::String(json_preview(value)));
            }
        }
    }

    Value::Object(object)
}

fn copy_string_field_if_missing(
    target: &mut serde_json::Map<String, Value>,
    source: &Value,
    key: &str,
) {
    if target.contains_key(key) {
        return;
    }
    if let Some(value) = source.get(key).and_then(Value::as_str) {
        target.insert(key.into(), Value::String(value.to_string()));
    }
}

fn copy_value_field_if_missing(
    target: &mut serde_json::Map<String, Value>,
    source: &Value,
    key: &str,
) {
    if target.contains_key(key) {
        return;
    }
    if let Some(value) = source.get(key) {
        target.insert(key.into(), value.clone());
    }
}

fn tool_permission_tool_name(question: &QuestionView) -> Option<&str> {
    question
        .raw_question
        .get("tool_name")
        .and_then(Value::as_str)
}

fn tool_permission_tool_use_id(question: &QuestionView) -> Option<&str> {
    question
        .raw_question
        .get("tool_use_id")
        .and_then(Value::as_str)
}

fn tool_permission_input_preview(question: &QuestionView) -> Option<String> {
    question
        .raw_question
        .get("input_preview")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| question.raw_question.get("tool_input").map(json_preview))
}

fn json_preview(value: &Value) -> String {
    let raw = serde_json::to_string(value).unwrap_or_default();
    raw.chars().take(240).collect()
}

fn map_tool_event_to_phase(event: &str) -> &'static str {
    match event {
        "tool_invocation_detected" => "detected",
        "tool_permission_requested" => "permission_requested",
        "tool_permission_resolved" => "permission_resolved",
        "tool_execution_started" => "execution_started",
        "tool_execution_finished" => "execution_finished",
        "tool_execution_failed" => "execution_failed",
        "tool_result_recorded" => "result_recorded",
        _ => "unknown",
    }
}

fn has_event(events: &[RuntimeTimelineEvent], name: &str) -> bool {
    events.iter().any(|event| event.event == name)
}

fn last_event_payload_value(
    events: &[RuntimeTimelineEvent],
    event_name: &str,
    key: &str,
) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event.event == event_name)
        .and_then(|event| event.payload.get(key))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn latest_payload_usize(events: &[JobEventView], event_names: &[&str], key: &str) -> Option<usize> {
    events
        .iter()
        .rev()
        .find(|event| event_names.contains(&event.event.as_str()))
        .and_then(|event| event.payload.get(key))
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
}

pub(crate) fn summary_to_value(summary: &ExecutionSummary) -> Value {
    serde_json::json!({
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
    })
}

pub(crate) fn failure_to_value(failure: &RuntimeFailureSnapshot) -> Value {
    serde_json::json!({
        "error_code": failure.error_code,
        "error_stage": failure.error_stage,
        "message_id": failure.message_id,
        "channel": failure.channel,
        "chat_id": failure.chat_id,
        "occurred_at_ms": failure.occurred_at_ms,
        "internal_message": failure.internal_message,
    })
}

pub(crate) fn runtime_failure_from_run(run: &RunRecord, job_id: &str) -> Option<Value> {
    let latest_error = run.latest_error.as_ref()?;
    Some(serde_json::json!({
        "error_code": "runtime_run_failed",
        "error_stage": "runtime",
        "message_id": job_id,
        "channel": run.channel,
        "chat_id": run.chat_id,
        "occurred_at_ms": run.completed_at_ms.unwrap_or(run.updated_at_ms),
        "internal_message": latest_error,
    }))
}
