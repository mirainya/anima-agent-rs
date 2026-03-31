use anima_runtime::agent::{CurrentTaskInfo, ExecutionSummary, RuntimeFailureSnapshot, RuntimeTimelineEvent, WorkerStatus};
use anima_runtime::support::now_ms;
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
    WaitingUpstreamInput,
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
pub struct WorkerTaskView {
    pub worker_id: String,
    pub status: String,
    pub task_id: String,
    pub trace_id: String,
    pub task_type: String,
    pub elapsed_ms: u64,
    pub content_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobView {
    pub job_id: String,
    pub trace_id: String,
    pub message_id: String,
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
    pub recent_events: Vec<JobEventView>,
    pub worker: Option<WorkerTaskView>,
    pub execution_summary: Option<Value>,
    pub failure: Option<Value>,
    pub review: Option<JobReviewView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedJob {
    pub job_id: String,
    pub trace_id: String,
    pub message_id: String,
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
    let now = now_ms();
    let mut ordered_ids = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut grouped: HashMap<String, Vec<RuntimeTimelineEvent>> = HashMap::new();

    for job in store.accepted.values() {
        if seen_ids.insert(job.job_id.clone()) {
            ordered_ids.push(job.job_id.clone());
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
        let recent_events = events
            .iter()
            .rev()
            .take(MAX_JOB_EVENTS)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|event| JobEventView {
                event: event.event,
                recorded_at_ms: event.recorded_at_ms,
                payload: event.payload,
            })
            .collect::<Vec<_>>();
        let summary = summaries.iter().find(|summary| summary.message_id == job_id).cloned();
        let failure = failures.iter().find(|failure| failure.message_id == job_id).cloned();
        let review = store.review_for(&job_id);
        let worker = match_worker(job_id.as_str(), workers);
        let started_at_ms = first
            .as_ref()
            .map(|event| event.recorded_at_ms)
            .or_else(|| accepted_job.as_ref().map(|job| job.accepted_at_ms))
            .unwrap_or(now);
        let updated_at_ms = worker
            .as_ref()
            .map(|worker| now.saturating_sub(worker.elapsed_ms))
            .or_else(|| last.as_ref().map(|event| event.recorded_at_ms))
            .unwrap_or(started_at_ms);
        let (status, status_label, current_step) = derive_job_status(
            &events,
            summary.as_ref(),
            failure.as_ref(),
            review.as_ref(),
            worker.as_ref(),
            now.saturating_sub(updated_at_ms),
        );
        let execution_summary = summary.as_ref().map(summary_to_value);
        let failure_value = failure.as_ref().map(failure_to_value);
        let trace_id = first
            .as_ref()
            .map(|event| event.trace_id.clone())
            .or_else(|| accepted_job.as_ref().map(|job| job.trace_id.clone()))
            .unwrap_or_else(|| job_id.clone());
        let message_id = first
            .as_ref()
            .map(|event| event.message_id.clone())
            .or_else(|| accepted_job.as_ref().map(|job| job.message_id.clone()))
            .unwrap_or_else(|| job_id.clone());
        let channel = first
            .as_ref()
            .map(|event| event.channel.clone())
            .or_else(|| accepted_job.as_ref().map(|job| job.channel.clone()))
            .unwrap_or_else(|| "web".into());
        let chat_id = first
            .as_ref()
            .and_then(|event| event.chat_id.clone())
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
            recent_events,
            worker,
            execution_summary,
            failure: failure_value,
            review,
        });
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
            })
        } else {
            None
        }
    })
}

fn current_task_matches(job_id: &str, task: &CurrentTaskInfo) -> bool {
    task.trace_id == job_id || task.task_id == job_id
}

fn derive_job_status(
    events: &[RuntimeTimelineEvent],
    summary: Option<&ExecutionSummary>,
    failure: Option<&RuntimeFailureSnapshot>,
    _review: Option<&JobReviewView>,
    worker: Option<&WorkerTaskView>,
    idle_since_update_ms: u64,
) -> (JobStatus, String, String) {
    let has_message_received = has_event(events, "message_received");
    let has_session_ready = has_event(events, "session_ready");
    let has_plan_built = has_event(events, "plan_built");
    let has_message_completed = has_event(events, "message_completed");
    let has_message_failed = has_event(events, "message_failed");
    let has_session_create_failed = has_event(events, "session_create_failed");
    let last_event_name = events.last().map(|event| event.event.as_str());

    if failure.is_some() || has_message_failed || has_session_create_failed {
        return (
            JobStatus::Failed,
            "failed".into(),
            "执行失败".into(),
        );
    }

    if has_message_completed {
        return (
            JobStatus::Completed,
            "completed".into(),
            "执行完成，结果已返回".into(),
        );
    }

    if let Some(worker) = worker {
        if worker.task_type == "session-create" {
            return (
                JobStatus::CreatingSession,
                "creating_session".into(),
                "正在创建上游会话".into(),
            );
        }
        return (
            JobStatus::Executing,
            "executing".into(),
            format!("worker 正在执行 {}", worker.task_type),
        );
    }

    if matches!(last_event_name, Some("cache_hit" | "cache_miss")) {
        let plan_type = summary
            .map(|item| item.plan_type.clone())
            .or_else(|| last_event_payload_value(events, "plan_built", "plan_type"))
            .unwrap_or_else(|| "single".into());
        return (
            JobStatus::Executing,
            "executing".into(),
            format!("已进入执行阶段，正在处理 {plan_type}"),
        );
    }

    if has_plan_built {
        let plan_type = summary
            .map(|item| item.plan_type.clone())
            .or_else(|| last_event_payload_value(events, "plan_built", "plan_type"))
            .unwrap_or_else(|| "single".into());

        if matches!(last_event_name, Some("plan_built")) && idle_since_update_ms >= UPSTREAM_INPUT_STALL_MS {
            return (
                JobStatus::WaitingUpstreamInput,
                "waiting_upstream_input".into(),
                "规划已完成，但长时间无新进展，可能正在等待上游交互输入".into(),
            );
        }

        return (
            JobStatus::Planning,
            "planning".into(),
            format!("已完成规划，准备执行 {plan_type}"),
        );
    }

    if has_session_ready {
        return (
            JobStatus::Planning,
            "planning".into(),
            "会话已就绪，正在构建计划".into(),
        );
    }

    if has_message_received {
        return (
            JobStatus::PreparingContext,
            "preparing_context".into(),
            "正在准备上下文".into(),
        );
    }

    (
        JobStatus::Queued,
        "queued".into(),
        "已进入队列".into(),
    )
}

fn has_event(events: &[RuntimeTimelineEvent], name: &str) -> bool {
    events.iter().any(|event| event.event == name)
}

fn last_event_payload_value(events: &[RuntimeTimelineEvent], event_name: &str, key: &str) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event.event == event_name)
        .and_then(|event| event.payload.get(key))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
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
