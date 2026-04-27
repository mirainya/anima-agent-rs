pub use crate::jobs_store::JobStore;
pub use crate::jobs_types::{
    AcceptedJob, JobEventView, JobKind, JobReviewInput, JobReviewView, JobStatus, JobView,
    OrchestrationView, PlanProposalView, PlanTaskView, QuestionView, ToolStateView, UserVerdict,
    WorkerTaskView,
};
use crate::jobs_derive::{
    collect_recent_job_events, derive_job_hierarchy, derive_job_status,
    derive_orchestration_view, derive_pending_question, derive_tool_state, failure_to_value,
    job_status_from_label, match_worker, runtime_failure_from_run,
    runtime_pending_question_view, runtime_status_with_worker, runtime_tool_state_view,
    summary_to_value,
};
use anima_runtime::agent::{
    ExecutionSummary, PendingQuestionSourceKind, RuntimeFailureSnapshot, RuntimeTimelineEvent,
    WorkerStatus,
};
use anima_runtime::runtime::{
    build_projection, queued_current_step, RuntimeProjectionView, RuntimeStateSnapshot,
};
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

struct LegacyStatusContext<'a> {
    events: &'a [RuntimeTimelineEvent],
    pending_question: Option<&'a QuestionView>,
    tool_state: Option<&'a ToolStateView>,
    summary: Option<&'a ExecutionSummary>,
    failure: Option<&'a RuntimeFailureSnapshot>,
    review: Option<&'a JobReviewView>,
    worker: Option<&'a WorkerTaskView>,
    idle_since_update_ms: u64,
    job_id: &'a str,
    execution_summary: &'a Option<Value>,
    runtime_failure_value: &'a Option<Value>,
    runtime_projection: &'a RuntimeProjectionView,
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

struct JobObservabilityState {
    pending_question: Option<QuestionView>,
    tool_state: Option<ToolStateView>,
    execution_summary: Option<Value>,
    runtime_failure_value: Option<Value>,
    failure_value: Option<Value>,
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
    let user_content = accepted_job
        .as_ref()
        .map(|job| job.user_content.clone())
        .or_else(|| {
            events.iter().find(|e| e.event == "message_received")
                .and_then(|e| e.payload.get("content_preview"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            let run = runtime_run?;
            runtime_snapshot.transcript.iter()
                .find(|m| m.run_id == run.run_id && m.role == anima_runtime::messages::MessageRole::User)
                .and_then(|m| m.blocks.first())
                .and_then(|b| match b {
                    anima_runtime::messages::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
        });

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
        pending_plan: derive_pending_plan(job_id, runtime_projection),
        orchestration: None,
    }
}

fn derive_pending_plan(job_id: &str, projection: &RuntimeProjectionView) -> Option<PlanProposalView> {
    let question = projection.pending_questions.get(job_id)?;
    if question.source_kind != PendingQuestionSourceKind::PlanApproval {
        return None;
    }
    let raw = &question.raw_question;
    let proposal_id = raw.get("proposal_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let summary = raw.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let tasks = raw.get("plan")
        .and_then(|p| p.get("tasks"))
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().map(|t| PlanTaskView {
            id: t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            task_type: t.get("task_type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            payload: t.get("payload").cloned().unwrap_or_default(),
        }).collect())
        .unwrap_or_default();
    Some(PlanProposalView {
        proposal_id,
        summary,
        tasks,
        proposed_at_ms: question.asked_at_ms,
    })
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
    let observability = derive_job_observability(
        job_id,
        events,
        summary,
        failure,
        runtime_run,
        runtime_projection,
    );
    let (status, status_label, current_step) = if runtime_backed {
        derive_runtime_backed_status(
            job_id,
            &observability.execution_summary,
            &observability.runtime_failure_value,
            worker,
            runtime_projection,
        )
    } else {
        let legacy_context = LegacyStatusContext {
            events,
            pending_question: observability.pending_question.as_ref(),
            tool_state: observability.tool_state.as_ref(),
            summary,
            failure,
            review,
            worker,
            idle_since_update_ms,
            job_id,
            execution_summary: &observability.execution_summary,
            runtime_failure_value: &observability.runtime_failure_value,
            runtime_projection,
        };
        derive_legacy_status(legacy_context)
    };

    JobDerivedState {
        pending_question: observability.pending_question,
        tool_state: observability.tool_state,
        execution_summary: observability.execution_summary,
        failure_value: observability.failure_value,
        status,
        status_label,
        current_step,
    }
}

fn derive_runtime_observability(
    job_id: &str,
    runtime_run: Option<&RunRecord>,
    runtime_projection: &RuntimeProjectionView,
) -> JobObservabilityState {
    let pending_question = runtime_projection
        .pending_questions
        .get(job_id)
        .map(runtime_pending_question_view);
    let tool_state = runtime_projection
        .tool_states
        .get(job_id)
        .map(runtime_tool_state_view);
    let execution_summary = runtime_projection
        .execution_summaries
        .get(job_id)
        .and_then(|summary| serde_json::to_value(summary).ok());
    let runtime_failure_value = runtime_projection
        .failures
        .get(job_id)
        .and_then(|failure| serde_json::to_value(failure).ok())
        .or_else(|| runtime_run.and_then(|run| runtime_failure_from_run(run, job_id)));

    JobObservabilityState {
        pending_question,
        tool_state,
        execution_summary,
        failure_value: runtime_failure_value.clone(),
        runtime_failure_value,
    }
}

fn apply_legacy_observability_fallback(
    base: JobObservabilityState,
    events: &[RuntimeTimelineEvent],
    summary: Option<&ExecutionSummary>,
    failure: Option<&RuntimeFailureSnapshot>,
) -> JobObservabilityState {
    let pending_question = base
        .pending_question
        .or_else(|| derive_pending_question(events));
    let tool_state = base
        .tool_state
        .or_else(|| derive_tool_state(events, pending_question.as_ref()));
    let execution_summary = base
        .execution_summary
        .or_else(|| summary.map(summary_to_value));
    let failure_value = base
        .failure_value
        .or_else(|| failure.map(failure_to_value));

    JobObservabilityState {
        pending_question,
        tool_state,
        execution_summary,
        runtime_failure_value: base.runtime_failure_value,
        failure_value,
    }
}

fn derive_job_observability(
    job_id: &str,
    events: &[RuntimeTimelineEvent],
    summary: Option<&ExecutionSummary>,
    failure: Option<&RuntimeFailureSnapshot>,
    runtime_run: Option<&RunRecord>,
    runtime_projection: &RuntimeProjectionView,
) -> JobObservabilityState {
    let mut base = derive_runtime_observability(job_id, runtime_run, runtime_projection);
    if base.failure_value.is_none() {
        base.failure_value = failure.map(failure_to_value);
    }
    if runtime_run.is_some() {
        base
    } else {
        apply_legacy_observability_fallback(base, events, summary, failure)
    }
}

fn derive_runtime_backed_status(
    job_id: &str,
    execution_summary: &Option<Value>,
    runtime_failure_value: &Option<Value>,
    worker: Option<&WorkerTaskView>,
    runtime_projection: &RuntimeProjectionView,
) -> (JobStatus, String, String) {
    derive_runtime_status(
        job_id,
        execution_summary,
        runtime_failure_value,
        worker,
        runtime_projection,
    )
    .unwrap_or_else(|| (JobStatus::Queued, "queued".to_string(), queued_current_step()))
}

fn derive_legacy_status(context: LegacyStatusContext<'_>) -> (JobStatus, String, String) {
    derive_runtime_status(
        context.job_id,
        context.execution_summary,
        context.runtime_failure_value,
        context.worker,
        context.runtime_projection,
    )
    .or_else(|| {
        if context.failure.is_none() {
            None
        } else {
            derive_runtime_failure_status(context.execution_summary, context.runtime_failure_value)
        }
    })
    .unwrap_or_else(|| {
        derive_job_status(
            context.events,
            context.pending_question,
            context.tool_state,
            context.summary,
            context.failure,
            context.review,
            context.worker,
            context.idle_since_update_ms,
        )
    })
}

fn derive_runtime_failure_status(
    execution_summary: &Option<Value>,
    runtime_failure_value: &Option<Value>,
) -> Option<(JobStatus, String, String)> {
    let runtime_failure_status = runtime_failure_value.as_ref().map(|_| {
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

    runtime_failure_status.or(execution_summary_failure_status)
}

fn derive_runtime_status(
    job_id: &str,
    execution_summary: &Option<Value>,
    runtime_failure_value: &Option<Value>,
    worker: Option<&WorkerTaskView>,
    runtime_projection: &RuntimeProjectionView,
) -> Option<(JobStatus, String, String)> {
    derive_runtime_failure_status(execution_summary, runtime_failure_value).or_else(|| {
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

#[cfg(test)]
mod tests {
    use super::build_job_views_with_projection;
    use crate::jobs_store::JobStore;
    use crate::jobs_types::JobStatus;
    use anima_runtime::agent::RuntimeFailureSnapshot;
    use anima_runtime::runtime::RuntimeStateSnapshot;
    use anima_runtime::tasks::{RunRecord, RunStatus, RuntimeTaskIndex};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn runtime_backed_job_status_ignores_legacy_failure_snapshot() {
        let runtime_snapshot = RuntimeStateSnapshot {
            runs: HashMap::from([(
                "run-1".into(),
                RunRecord {
                    run_id: "run-1".into(),
                    trace_id: "trace-1".into(),
                    job_id: "job-1".into(),
                    chat_id: Some("session-1".into()),
                    channel: "web".into(),
                    status: RunStatus::Running,
                    current_turn_id: None,
                    latest_error: None,
                    created_at_ms: 10,
                    updated_at_ms: 20,
                    completed_at_ms: None,
                },
            )]),
            index: RuntimeTaskIndex {
                run_ids_by_job_id: HashMap::from([("job-1".into(), "run-1".into())]),
                ..RuntimeTaskIndex::default()
            },
            ..RuntimeStateSnapshot::default()
        };
        let runtime_projection = anima_runtime::runtime::build_projection(&runtime_snapshot);
        let legacy_failures = vec![RuntimeFailureSnapshot {
            error_code: "legacy_failure".into(),
            error_stage: "runtime".into(),
            message_id: "job-1".into(),
            channel: "web".into(),
            chat_id: Some("session-1".into()),
            occurred_at_ms: 30,
            internal_message: "legacy only".into(),
        }];
        let jobs = build_job_views_with_projection(
            &[],
            &[],
            &legacy_failures,
            &[],
            &JobStore::default(),
            &runtime_snapshot,
            &runtime_projection,
        );

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "job-1");
        assert_eq!(jobs[0].status, JobStatus::Executing);
        assert_eq!(jobs[0].status_label, "executing");
        assert_eq!(
            jobs[0]
                .failure
                .as_ref()
                .and_then(|value| value.get("error_code"))
                .and_then(|value| value.as_str()),
            Some("legacy_failure")
        );
        assert_eq!(jobs[0].execution_summary, Some(json!({
            "plan_type": "single",
            "status": "running",
            "cache_hit": false,
            "worker_id": null,
            "error_code": null,
            "error_stage": null,
            "task_duration_ms": 10,
            "stages": {}
        })));
    }
}
