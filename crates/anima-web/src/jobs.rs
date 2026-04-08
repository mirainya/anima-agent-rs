use anima_runtime::agent::{
    CurrentTaskInfo, ExecutionSummary, RuntimeFailureSnapshot, RuntimeTimelineEvent, WorkerStatus,
};
use anima_runtime::runtime::{
    build_projection, completed_current_step, creating_session_current_step,
    executing_plan_current_step, failed_current_step, followup_current_step,
    planning_ready_current_step, planning_stalled_current_step, preparing_context_current_step,
    queued_current_step, session_ready_current_step, tool_execution_failed_current_step,
    tool_execution_finished_current_step, tool_permission_resolved_current_step,
    tool_phase_current_step, tool_result_recorded_current_step, waiting_user_input_current_step,
    worker_executing_current_step, RuntimeProjectionView, RuntimeStateSnapshot,
};
use anima_runtime::support::now_ms;
use anima_runtime::tasks::{run_by_job_id, tasks_for_job};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_JOB_EVENTS: usize = 8;
const MAX_JOB_VIEWS: usize = 100;
const UPSTREAM_INPUT_STALL_MS: u64 = 15_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserVerdict {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    PreparingContext,
    CreatingSession,
    Planning,
    Executing,
    WaitingUserInput,
    Stalled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobEventView {
    pub event: String,
    pub recorded_at_ms: u64,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobReviewView {
    pub verdict: UserVerdict,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub reviewed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionView {
    pub question_id: String,
    pub question_kind: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub raw_question: Value,
    pub decision_mode: String,
    pub risk_level: String,
    pub requires_user_confirmation: bool,
    pub opencode_session_id: Option<String>,
    pub answer_summary: Option<String>,
    pub resolution_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerTaskView {
    pub worker_id: String,
    pub status: String,
    pub task_id: String,
    pub trace_id: String,
    pub task_type: String,
    pub elapsed_ms: u64,
    pub content_preview: String,
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStateView {
    pub invocation_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub phase: String,
    pub permission_state: Option<String>,
    pub input_preview: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub awaits_user_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Main,
    Subtask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestrationView {
    pub plan_id: Option<String>,
    pub active_subtask_name: Option<String>,
    pub active_subtask_type: Option<String>,
    pub active_subtask_id: Option<String>,
    pub total_subtasks: usize,
    pub active_subtasks: usize,
    pub completed_subtasks: usize,
    pub failed_subtasks: usize,
    pub child_job_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobView {
    pub job_id: String,
    pub trace_id: String,
    pub message_id: String,
    pub kind: JobKind,
    pub parent_job_id: Option<String>,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: String,
    pub user_content: Option<String>,
    pub status: JobStatus,
    pub status_label: String,
    pub accepted: bool,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub elapsed_ms: u64,
    pub current_step: String,
    pub pending_question: Option<QuestionView>,
    pub recent_events: Vec<JobEventView>,
    pub worker: Option<WorkerTaskView>,
    pub tool_state: Option<ToolStateView>,
    pub execution_summary: Option<Value>,
    pub failure: Option<Value>,
    pub review: Option<JobReviewView>,
    pub orchestration: Option<OrchestrationView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedJob {
    pub job_id: String,
    pub trace_id: String,
    pub message_id: String,
    pub kind: JobKind,
    pub parent_job_id: Option<String>,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: String,
    pub user_content: String,
    pub accepted_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct JobStore {
    accepted: HashMap<String, AcceptedJob>,
    reviews: HashMap<String, JobReviewView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobReviewInput {
    pub user_verdict: UserVerdict,
    pub reason: Option<String>,
    pub note: Option<String>,
}

impl JobStore {
    pub fn register_accepted_job(&mut self, job: AcceptedJob) {
        self.accepted.insert(job.job_id.clone(), job);
    }

    pub fn accepted_job(&self, job_id: &str) -> Option<AcceptedJob> {
        self.accepted.get(job_id).cloned()
    }

    pub fn parent_job_id_by_trace(&self, trace_id: &str) -> Option<String> {
        self.accepted
            .values()
            .find(|job| job.trace_id == trace_id && job.parent_job_id.is_some())
            .and_then(|job| job.parent_job_id.clone())
    }

    pub fn record_review(&mut self, job_id: String, input: JobReviewInput) -> JobReviewView {
        let review = JobReviewView {
            verdict: input.user_verdict,
            reason: input.reason.filter(|value| !value.trim().is_empty()),
            note: input.note.filter(|value| !value.trim().is_empty()),
            reviewed_at_ms: now_ms(),
        };
        self.reviews.insert(job_id, review.clone());
        review
    }

    pub fn review_for(&self, job_id: &str) -> Option<JobReviewView> {
        self.reviews.get(job_id).cloned()
    }
}

pub fn build_job_views(
    timeline: &[RuntimeTimelineEvent],
    summaries: &[ExecutionSummary],
    failures: &[RuntimeFailureSnapshot],
    workers: &[WorkerStatus],
    store: &JobStore,
) -> Vec<JobView> {
    let runtime_snapshot = RuntimeStateSnapshot::default();
    let runtime_projection = build_projection(&runtime_snapshot);
    build_job_views_with_projection(
        timeline,
        summaries,
        failures,
        workers,
        store,
        &runtime_snapshot,
        &runtime_projection,
    )
}

pub fn build_job_views_with_runtime(
    timeline: &[RuntimeTimelineEvent],
    summaries: &[ExecutionSummary],
    failures: &[RuntimeFailureSnapshot],
    workers: &[WorkerStatus],
    store: &JobStore,
    runtime_snapshot: &RuntimeStateSnapshot,
) -> Vec<JobView> {
    let runtime_projection = build_projection(runtime_snapshot);
    build_job_views_with_projection(
        timeline,
        summaries,
        failures,
        workers,
        store,
        runtime_snapshot,
        &runtime_projection,
    )
}

pub fn build_job_views_with_projection(
    timeline: &[RuntimeTimelineEvent],
    summaries: &[ExecutionSummary],
    failures: &[RuntimeFailureSnapshot],
    workers: &[WorkerStatus],
    store: &JobStore,
    runtime_snapshot: &RuntimeStateSnapshot,
    runtime_projection: &RuntimeProjectionView,
) -> Vec<JobView> {
    let now = now_ms();
    let mut ordered_ids = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut grouped: HashMap<String, Vec<RuntimeTimelineEvent>> = HashMap::new();

    for job in store.accepted.values() {
        if seen_ids.insert(job.job_id.clone()) {
            ordered_ids.push(job.job_id.clone());
        }
    }

    for run in runtime_snapshot.runs.values() {
        if seen_ids.insert(run.job_id.clone()) {
            ordered_ids.push(run.job_id.clone());
        }
    }

    for event in timeline.iter() {
        let job_id = event.message_id.clone();
        if seen_ids.insert(job_id.clone()) {
            ordered_ids.push(job_id.clone());
        }
        grouped.entry(job_id).or_default().push(event.clone());
    }

    let mut jobs = Vec::new();
    for job_id in ordered_ids {
        let events = grouped.remove(&job_id).unwrap_or_default();
        let accepted_job = store.accepted_job(&job_id);
        let first = events.first().cloned();
        let last = events.last().cloned().or(first.clone());
        let recent_events = collect_recent_job_events(&events);
        let runtime_run = run_by_job_id(runtime_snapshot, &job_id);
        let runtime_backed = runtime_run.is_some();
        let summary = summaries
            .iter()
            .find(|summary| summary.message_id == job_id)
            .cloned();
        let runtime_execution_summary = runtime_projection
            .execution_summaries
            .get(&job_id)
            .and_then(|summary| serde_json::to_value(summary).ok());
        let failure = failures
            .iter()
            .find(|failure| failure.message_id == job_id)
            .cloned();
        let review = store.review_for(&job_id);
        let worker = match_worker(job_id.as_str(), workers);
        let runtime_pending_question = runtime_projection
            .pending_questions
            .get(&job_id)
            .cloned()
            .map(|question| QuestionView {
                question_id: question.question_id,
                question_kind: question_kind_label(&question.question_kind),
                prompt: question.prompt,
                options: question.options,
                raw_question: question.raw_question,
                decision_mode: question_decision_mode_label(&question.decision_mode),
                risk_level: question_risk_level_label(&question.risk_level),
                requires_user_confirmation: question.requires_user_confirmation,
                opencode_session_id: Some(question.opencode_session_id),
                answer_summary: question.answer_summary,
                resolution_source: question.resolution_source,
            });
        let pending_question = runtime_pending_question.clone().or_else(|| {
            if runtime_backed {
                None
            } else {
                derive_pending_question(&events)
            }
        });
        let runtime_tool_state =
            runtime_projection
                .tool_states
                .get(&job_id)
                .cloned()
                .map(|tool_state| ToolStateView {
                    invocation_id: tool_state.invocation_id,
                    tool_name: tool_state.tool_name,
                    tool_use_id: tool_state.tool_use_id,
                    phase: tool_state.phase,
                    permission_state: tool_state.permission_state,
                    input_preview: tool_state.input_preview,
                    result_preview: tool_state.result_preview,
                    error: tool_state.error,
                    awaits_user_confirmation: tool_state.awaits_user_confirmation,
                });
        let tool_state = runtime_tool_state.or_else(|| {
            if runtime_backed {
                None
            } else {
                derive_tool_state(&events, pending_question.as_ref())
            }
        });
        let started_at_ms = runtime_run
            .map(|run| run.created_at_ms)
            .or_else(|| first.as_ref().map(|event| event.recorded_at_ms))
            .or_else(|| accepted_job.as_ref().map(|job| job.accepted_at_ms))
            .unwrap_or(now);
        let updated_at_ms = worker
            .as_ref()
            .map(|worker| now.saturating_sub(worker.elapsed_ms))
            .or_else(|| runtime_run.map(|run| run.updated_at_ms))
            .or_else(|| last.as_ref().map(|event| event.recorded_at_ms))
            .unwrap_or(started_at_ms);
        let execution_summary = runtime_execution_summary.or_else(|| {
            if runtime_backed {
                None
            } else {
                summary.as_ref().map(summary_to_value)
            }
        });
        let runtime_failure = runtime_projection
            .failures
            .get(&job_id)
            .and_then(|failure| serde_json::to_value(failure).ok())
            .or_else(|| runtime_run.and_then(|run| runtime_failure_from_run(run, &job_id)));
        let failure_value = runtime_failure.or_else(|| failure.as_ref().map(failure_to_value));
        let runtime_failure_status = failure_value.as_ref().map(|_| {
            (
                JobStatus::Failed,
                "failed".to_string(),
                failed_current_step(),
            )
        });
        let execution_summary_failure_status = execution_summary.as_ref().and_then(|summary| {
            let status = summary.get("status").and_then(Value::as_str)?;
            if matches!(status, "failure" | "followup_exhausted") {
                Some((
                    JobStatus::Failed,
                    "failed".to_string(),
                    failed_current_step(),
                ))
            } else {
                None
            }
        });
        let runtime_status = runtime_failure_status
            .or(execution_summary_failure_status)
            .or_else(|| {
                if failure.is_some() {
                    None
                } else {
                    runtime_projection.job_statuses.get(&job_id).map(|summary| {
                        let worker_override = worker
                            .as_ref()
                            .and_then(|worker| runtime_status_with_worker(summary, worker));
                        worker_override.unwrap_or_else(|| {
                            (
                                job_status_from_label(&summary.status_label),
                                summary.status_label.clone(),
                                summary.current_step.clone(),
                            )
                        })
                    })
                }
            });
        let (status, status_label, current_step) = runtime_status.unwrap_or_else(|| {
            derive_job_status(
                &events,
                pending_question.as_ref(),
                tool_state.as_ref(),
                summary.as_ref(),
                failure.as_ref(),
                review.as_ref(),
                worker.as_ref(),
                now.saturating_sub(updated_at_ms),
            )
        });
        let trace_id = runtime_run
            .map(|run| run.trace_id.clone())
            .or_else(|| first.as_ref().map(|event| event.trace_id.clone()))
            .or_else(|| accepted_job.as_ref().map(|job| job.trace_id.clone()))
            .unwrap_or_else(|| job_id.clone());
        let (kind, parent_job_id) = derive_job_hierarchy(
            accepted_job.as_ref(),
            &events,
            &trace_id,
            store,
            runtime_snapshot,
            &job_id,
        );
        let message_id = first
            .as_ref()
            .map(|event| event.message_id.clone())
            .or_else(|| accepted_job.as_ref().map(|job| job.message_id.clone()))
            .unwrap_or_else(|| job_id.clone());
        let channel = runtime_run
            .map(|run| run.channel.clone())
            .or_else(|| first.as_ref().map(|event| event.channel.clone()))
            .or_else(|| accepted_job.as_ref().map(|job| job.channel.clone()))
            .unwrap_or_else(|| "web".into());
        let chat_id = runtime_run
            .and_then(|run| run.chat_id.clone())
            .or_else(|| first.as_ref().and_then(|event| event.chat_id.clone()))
            .or_else(|| accepted_job.as_ref().and_then(|job| job.chat_id.clone()));
        let sender_id = first
            .as_ref()
            .map(|event| event.sender_id.clone())
            .or_else(|| accepted_job.as_ref().map(|job| job.sender_id.clone()))
            .unwrap_or_else(|| "web-user".into());
        let user_content = accepted_job.as_ref().map(|job| job.user_content.clone());

        jobs.push(JobView {
            job_id: job_id.clone(),
            trace_id,
            message_id,
            kind,
            parent_job_id,
            channel,
            chat_id,
            sender_id,
            user_content,
            status,
            status_label,
            accepted: accepted_job.is_some() || !events.is_empty(),
            started_at_ms,
            updated_at_ms,
            elapsed_ms: now.saturating_sub(started_at_ms),
            current_step,
            pending_question,
            recent_events,
            worker,
            tool_state,
            execution_summary,
            failure: failure_value,
            review,
            orchestration: None,
        });
    }

    let child_ids_by_parent =
        jobs.iter()
            .fold(HashMap::<String, Vec<String>>::new(), |mut acc, job| {
                if let Some(parent_job_id) = &job.parent_job_id {
                    acc.entry(parent_job_id.clone())
                        .or_default()
                        .push(job.job_id.clone());
                }
                acc
            });

    for job in &mut jobs {
        if job.kind != JobKind::Main {
            continue;
        }
        let child_job_ids = child_ids_by_parent
            .get(&job.job_id)
            .cloned()
            .unwrap_or_default();
        let orchestration = derive_orchestration_view(
            job,
            &child_job_ids,
            &child_ids_by_parent,
            runtime_snapshot,
            runtime_projection,
        );
        job.orchestration = orchestration;
    }

    jobs.sort_by_key(|job| std::cmp::Reverse(job.started_at_ms));
    if jobs.len() > MAX_JOB_VIEWS {
        jobs.drain(..jobs.len() - MAX_JOB_VIEWS);
    }
    jobs
}

fn match_worker(job_id: &str, workers: &[WorkerStatus]) -> Option<WorkerTaskView> {
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

fn job_status_from_label(label: &str) -> JobStatus {
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

fn question_kind_label(kind: &anima_runtime::agent::QuestionKind) -> String {
    match kind {
        anima_runtime::agent::QuestionKind::Confirm => "confirm".into(),
        anima_runtime::agent::QuestionKind::Choice => "choice".into(),
        anima_runtime::agent::QuestionKind::Input => "input".into(),
    }
}

fn question_decision_mode_label(mode: &anima_runtime::agent::QuestionDecisionMode) -> String {
    match mode {
        anima_runtime::agent::QuestionDecisionMode::AutoAllowed => "auto_allowed".into(),
        anima_runtime::agent::QuestionDecisionMode::UserRequired => "user_required".into(),
    }
}

fn question_risk_level_label(level: &anima_runtime::agent::QuestionRiskLevel) -> String {
    match level {
        anima_runtime::agent::QuestionRiskLevel::Low => "low".into(),
        anima_runtime::agent::QuestionRiskLevel::High => "high".into(),
    }
}

fn runtime_status_with_worker(
    summary: &anima_runtime::runtime::ProjectionJobStatusSummary,
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
        let current_step =
            worker_executing_current_step(&worker.task_type, worker.phase.as_deref());
        return Some((JobStatus::Executing, "executing".into(), current_step));
    }

    None
}

fn derive_job_hierarchy(
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

fn derive_orchestration_view(
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

fn collect_recent_job_events(events: &[RuntimeTimelineEvent]) -> Vec<JobEventView> {
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

fn latest_payload_string(
    events: &[JobEventView],
    event_names: &[&str],
    key: &str,
) -> Option<String> {
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

fn derive_pending_question(events: &[RuntimeTimelineEvent]) -> Option<QuestionView> {
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
            .or_else(|| {
                normalized_raw_question
                    .get("prompt")
                    .and_then(Value::as_str)
            })
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
fn derive_job_status(
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
        let current_step =
            worker_executing_current_step(&worker.task_type, worker.phase.as_deref());
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

fn derive_tool_state(
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
        .or_else(|| {
            details
                .and_then(|value| value.get("error"))
                .and_then(Value::as_str)
        })
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

fn summary_to_value(summary: &ExecutionSummary) -> Value {
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

fn failure_to_value(failure: &RuntimeFailureSnapshot) -> Value {
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

fn runtime_failure_from_run(run: &anima_runtime::tasks::RunRecord, job_id: &str) -> Option<Value> {
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
