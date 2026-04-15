pub use crate::jobs_store::JobStore;
pub use crate::jobs_types::{
    AcceptedJob, JobEventView, JobKind, JobReviewInput, JobReviewView, JobStatus, JobView,
    OrchestrationView, QuestionView, ToolStateView, UserVerdict, WorkerTaskView,
};
use crate::jobs_derive::{
    collect_recent_job_events, derive_job_hierarchy, derive_job_status,
    derive_orchestration_view, derive_pending_question, derive_tool_state, failure_to_value,
    job_status_from_label, match_worker, question_decision_mode_label, question_kind_label,
    question_risk_level_label, runtime_failure_from_run, runtime_status_with_worker,
    summary_to_value,
};
use anima_runtime::agent::{
    ExecutionSummary, RuntimeFailureSnapshot, RuntimeTimelineEvent, WorkerStatus,
};
use anima_runtime::runtime::{build_projection, RuntimeProjectionView, RuntimeStateSnapshot};
use anima_runtime::support::now_ms;
use anima_runtime::tasks::{run_by_job_id, RunRecord};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MAX_JOB_VIEWS: usize = 100;
const FAILED_CURRENT_STEP: &str = "执行失败，等待处理";

struct JobTimelineIndex {
    ordered_ids: Vec<String>,
    grouped: HashMap<String, Vec<RuntimeTimelineEvent>>,
}

struct JobDerivedState {
    pending_question: Option<QuestionView>,
    tool_state: Option<ToolStateView>,
    execution_summary: Option<Value>,
    failure_value: Option<Value>,
    status: JobStatus,
    status_label: String,
    current_step: String,
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
    let JobTimelineIndex {
        ordered_ids,
        mut grouped,
    } = build_job_timeline_index(timeline, store, runtime_snapshot);

    let mut jobs = ordered_ids
        .into_iter()
        .map(|job_id| {
            let events = grouped.remove(&job_id).unwrap_or_default();
            build_job_view(
                &job_id,
                &events,
                summaries,
                failures,
                workers,
                store,
                runtime_snapshot,
                runtime_projection,
                now,
            )
        })
        .collect::<Vec<_>>();

    attach_orchestration_views(&mut jobs, runtime_snapshot, runtime_projection);
    jobs.sort_by_key(|job| std::cmp::Reverse(job.started_at_ms));
    if jobs.len() > MAX_JOB_VIEWS {
        jobs.drain(..jobs.len() - MAX_JOB_VIEWS);
    }
    jobs
}

fn build_job_timeline_index(
    timeline: &[RuntimeTimelineEvent],
    store: &JobStore,
    runtime_snapshot: &RuntimeStateSnapshot,
) -> JobTimelineIndex {
    let mut ordered_ids = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut grouped: HashMap<String, Vec<RuntimeTimelineEvent>> = HashMap::new();

    for job in store.accepted_jobs() {
        if seen_ids.insert(job.job_id.clone()) {
            ordered_ids.push(job.job_id.clone());
        }
    }

    for run in runtime_snapshot.runs.values() {
        if seen_ids.insert(run.job_id.clone()) {
            ordered_ids.push(run.job_id.clone());
        }
    }

    for event in timeline {
        let job_id = event.message_id.clone();
        if seen_ids.insert(job_id.clone()) {
            ordered_ids.push(job_id.clone());
        }
        grouped.entry(job_id).or_default().push(event.clone());
    }

    JobTimelineIndex { ordered_ids, grouped }
}

#[allow(clippy::too_many_arguments)]
fn build_job_view(
    job_id: &str,
    events: &[RuntimeTimelineEvent],
    summaries: &[ExecutionSummary],
    failures: &[RuntimeFailureSnapshot],
    workers: &[WorkerStatus],
    store: &JobStore,
    runtime_snapshot: &RuntimeStateSnapshot,
    runtime_projection: &RuntimeProjectionView,
    now: u64,
) -> JobView {
    let accepted_job = store.accepted_job(job_id);
    let first = events.first().cloned();
    let last = events.last().cloned().or(first.clone());
    let recent_events = collect_recent_job_events(events);
    let runtime_run = run_by_job_id(runtime_snapshot, job_id);
    let summary = summaries
        .iter()
        .find(|summary| summary.message_id == job_id)
        .cloned();
    let failure = failures
        .iter()
        .find(|failure| failure.message_id == job_id)
        .cloned();
    let review = store.review_for(job_id);
    let worker = match_worker(job_id, workers);
    let started_at_ms = derive_started_at_ms(now, runtime_run, first.as_ref(), accepted_job.as_ref());
    let updated_at_ms = derive_updated_at_ms(now, runtime_run, last.as_ref(), worker.as_ref(), started_at_ms);
    let derived = derive_job_state(
        job_id,
        events,
        summary.as_ref(),
        failure.as_ref(),
        review.as_ref(),
        worker.as_ref(),
        runtime_run,
        runtime_projection,
        now.saturating_sub(updated_at_ms),
    );
    let trace_id = runtime_run
        .map(|run| run.trace_id.clone())
        .or_else(|| first.as_ref().map(|event| event.trace_id.clone()))
        .or_else(|| accepted_job.as_ref().map(|job| job.trace_id.clone()))
        .unwrap_or_else(|| job_id.to_string());
    let (kind, parent_job_id) = derive_job_hierarchy(
        accepted_job.as_ref(),
        events,
        &trace_id,
        store,
        runtime_snapshot,
        job_id,
    );
    let message_id = first
        .as_ref()
        .map(|event| event.message_id.clone())
        .or_else(|| accepted_job.as_ref().map(|job| job.message_id.clone()))
        .unwrap_or_else(|| job_id.to_string());
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

    JobView {
        job_id: job_id.to_string(),
        trace_id,
        message_id,
        kind,
        parent_job_id,
        channel,
        chat_id,
        sender_id,
        user_content,
        status: derived.status,
        status_label: derived.status_label,
        accepted: accepted_job.is_some() || !events.is_empty(),
        started_at_ms,
        updated_at_ms,
        elapsed_ms: now.saturating_sub(started_at_ms),
        current_step: derived.current_step,
        pending_question: derived.pending_question,
        recent_events,
        worker,
        tool_state: derived.tool_state,
        execution_summary: derived.execution_summary,
        failure: derived.failure_value,
        review,
        orchestration: None,
    }
}

fn derive_started_at_ms(
    now: u64,
    runtime_run: Option<&RunRecord>,
    first: Option<&RuntimeTimelineEvent>,
    accepted_job: Option<&AcceptedJob>,
) -> u64 {
    runtime_run
        .map(|run| run.created_at_ms)
        .or_else(|| first.map(|event| event.recorded_at_ms))
        .or_else(|| accepted_job.map(|job| job.accepted_at_ms))
        .unwrap_or(now)
}

fn derive_updated_at_ms(
    now: u64,
    runtime_run: Option<&RunRecord>,
    last: Option<&RuntimeTimelineEvent>,
    worker: Option<&WorkerTaskView>,
    started_at_ms: u64,
) -> u64 {
    worker
        .map(|worker| now.saturating_sub(worker.elapsed_ms))
        .or_else(|| runtime_run.map(|run| run.updated_at_ms))
        .or_else(|| last.map(|event| event.recorded_at_ms))
        .unwrap_or(started_at_ms)
}

#[allow(clippy::too_many_arguments)]
fn derive_job_state(
    job_id: &str,
    events: &[RuntimeTimelineEvent],
    summary: Option<&ExecutionSummary>,
    failure: Option<&RuntimeFailureSnapshot>,
    review: Option<&JobReviewView>,
    worker: Option<&WorkerTaskView>,
    runtime_run: Option<&RunRecord>,
    runtime_projection: &RuntimeProjectionView,
    idle_since_update_ms: u64,
) -> JobDerivedState {
    let runtime_backed = runtime_run.is_some();
    let runtime_pending_question = runtime_projection
        .pending_questions
        .get(job_id)
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
            derive_pending_question(events)
        }
    });
    let runtime_tool_state = runtime_projection
        .tool_states
        .get(job_id)
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
            derive_tool_state(events, pending_question.as_ref())
        }
    });
    let runtime_execution_summary = runtime_projection
        .execution_summaries
        .get(job_id)
        .and_then(|summary| serde_json::to_value(summary).ok());
    let execution_summary = runtime_execution_summary.or_else(|| {
        if runtime_backed {
            None
        } else {
            summary.map(summary_to_value)
        }
    });
    let runtime_failure = runtime_projection
        .failures
        .get(job_id)
        .and_then(|failure| serde_json::to_value(failure).ok())
        .or_else(|| runtime_run.and_then(|run| runtime_failure_from_run(run, job_id)));
    let failure_value = runtime_failure.or_else(|| failure.map(failure_to_value));
    let runtime_status = derive_runtime_status(job_id, &execution_summary, &failure_value, failure, worker, runtime_projection);
    let (status, status_label, current_step) = runtime_status.unwrap_or_else(|| {
        derive_job_status(
            events,
            pending_question.as_ref(),
            tool_state.as_ref(),
            summary,
            failure,
            review,
            worker,
            idle_since_update_ms,
        )
    });

    JobDerivedState {
        pending_question,
        tool_state,
        execution_summary,
        failure_value,
        status,
        status_label,
        current_step,
    }
}

fn derive_runtime_status(
    job_id: &str,
    execution_summary: &Option<Value>,
    failure_value: &Option<Value>,
    failure: Option<&RuntimeFailureSnapshot>,
    worker: Option<&WorkerTaskView>,
    runtime_projection: &RuntimeProjectionView,
) -> Option<(JobStatus, String, String)> {
    let runtime_failure_status = failure_value.as_ref().map(|_| {
        (
            JobStatus::Failed,
            "failed".to_string(),
            FAILED_CURRENT_STEP.to_string(),
        )
    });
    let execution_summary_failure_status = execution_summary.as_ref().and_then(|summary| {
        let status = summary.get("status").and_then(Value::as_str)?;
        if matches!(status, "failure" | "followup_exhausted") {
            Some((
                JobStatus::Failed,
                "failed".to_string(),
                FAILED_CURRENT_STEP.to_string(),
            ))
        } else {
            None
        }
    });

    runtime_failure_status
        .or(execution_summary_failure_status)
        .or_else(|| {
            if failure.is_some() {
                None
            } else {
                runtime_projection.job_statuses.get(job_id).map(|summary| {
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
        })
}

fn attach_orchestration_views(
    jobs: &mut [JobView],
    runtime_snapshot: &RuntimeStateSnapshot,
    runtime_projection: &RuntimeProjectionView,
) {
    let child_ids_by_parent = jobs
        .iter()
        .fold(HashMap::<String, Vec<String>>::new(), |mut acc, job| {
            if let Some(parent_job_id) = &job.parent_job_id {
                acc.entry(parent_job_id.clone())
                    .or_default()
                    .push(job.job_id.clone());
            }
            acc
        });

    for job in jobs {
        if job.kind != JobKind::Main {
            continue;
        }
        let child_job_ids = child_ids_by_parent
            .get(&job.job_id)
            .cloned()
            .unwrap_or_default();
        job.orchestration = derive_orchestration_view(
            job,
            &child_job_ids,
            &child_ids_by_parent,
            runtime_snapshot,
            runtime_projection,
        );
    }
}
