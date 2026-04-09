use super::model::{
    RequirementRecord, RunRecord, SuspensionRecord, TaskRecord, ToolInvocationRuntimeRecord,
    TurnRecord,
};
use crate::runtime::snapshot::RuntimeStateSnapshot;
use std::cmp::Reverse;

pub fn run_by_job_id<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    job_id: &str,
) -> Option<&'a RunRecord> {
    snapshot
        .index
        .run_ids_by_job_id
        .get(job_id)
        .and_then(|run_id| snapshot.runs.get(run_id))
}

pub fn active_turn<'a>(snapshot: &'a RuntimeStateSnapshot, run_id: &str) -> Option<&'a TurnRecord> {
    snapshot
        .runs
        .get(run_id)
        .and_then(|run| run.current_turn_id.as_ref())
        .and_then(|turn_id| snapshot.turns.get(turn_id))
}

pub fn tasks_for_run<'a>(snapshot: &'a RuntimeStateSnapshot, run_id: &str) -> Vec<&'a TaskRecord> {
    let mut tasks = snapshot
        .tasks
        .values()
        .filter(|task| task.run_id == run_id)
        .collect::<Vec<_>>();
    tasks.sort_by_key(|task| {
        (
            task.started_at_ms.unwrap_or(task.updated_at_ms),
            Reverse(task.updated_at_ms),
            task.task_id.as_str(),
        )
    });
    tasks
}

pub fn tasks_for_job<'a>(snapshot: &'a RuntimeStateSnapshot, job_id: &str) -> Vec<&'a TaskRecord> {
    let Some(run) = run_by_job_id(snapshot, job_id) else {
        return Vec::new();
    };
    tasks_for_run(snapshot, &run.run_id)
}

pub fn tasks_for_plan<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    plan_id: &str,
) -> Vec<&'a TaskRecord> {
    let mut tasks = snapshot
        .index
        .task_ids_by_plan_id
        .get(plan_id)
        .into_iter()
        .flatten()
        .filter_map(|task_id| snapshot.tasks.get(task_id))
        .collect::<Vec<_>>();
    tasks.sort_by_key(|task| {
        (
            task.started_at_ms.unwrap_or(task.updated_at_ms),
            Reverse(task.updated_at_ms),
            task.task_id.as_str(),
        )
    });
    tasks
}

pub fn plan_task<'a>(snapshot: &'a RuntimeStateSnapshot, plan_id: &str) -> Option<&'a TaskRecord> {
    tasks_for_plan(snapshot, plan_id)
        .into_iter()
        .find(|task| task.parent_task_id.is_none())
}

pub fn subtasks_for_plan<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    plan_id: &str,
) -> Vec<&'a TaskRecord> {
    let mut tasks = tasks_for_plan(snapshot, plan_id)
        .into_iter()
        .filter(|task| task.parent_task_id.is_some())
        .collect::<Vec<_>>();
    tasks.sort_by_key(|task| {
        (
            task.started_at_ms.unwrap_or(task.updated_at_ms),
            Reverse(task.updated_at_ms),
            task.task_id.as_str(),
        )
    });
    tasks
}

pub fn active_suspension<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    run_id: &str,
) -> Option<&'a SuspensionRecord> {
    snapshot.suspensions.values().find(|record| {
        record.run_id == run_id && matches!(record.status, super::model::SuspensionStatus::Active)
    })
}

pub fn suspension_by_question_id<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    question_id: &str,
) -> Option<&'a SuspensionRecord> {
    snapshot
        .index
        .suspension_ids_by_question_id
        .get(question_id)
        .and_then(|suspension_id| snapshot.suspensions.get(suspension_id))
}

pub fn active_requirement<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    run_id: &str,
) -> Option<&'a RequirementRecord> {
    snapshot
        .requirements
        .values()
        .find(|record| record.run_id == run_id)
}

pub fn latest_tool_invocation<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    run_id: &str,
) -> Option<&'a ToolInvocationRuntimeRecord> {
    snapshot
        .tool_invocations
        .values()
        .filter(|record| record.run_id == run_id)
        .max_by_key(|record| record.started_at_ms)
}

pub fn invocation_by_question_id<'a>(
    snapshot: &'a RuntimeStateSnapshot,
    question_id: &str,
) -> Option<&'a ToolInvocationRuntimeRecord> {
    snapshot
        .index
        .invocation_ids_by_question_id
        .get(question_id)
        .and_then(|invocation_id| snapshot.tool_invocations.get(invocation_id))
}
