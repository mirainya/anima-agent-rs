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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::model::*;
    use serde_json::json;

    fn make_snapshot() -> RuntimeStateSnapshot {
        RuntimeStateSnapshot::default()
    }

    fn run(run_id: &str, job_id: &str) -> RunRecord {
        RunRecord {
            run_id: run_id.into(),
            trace_id: "t".into(),
            job_id: job_id.into(),
            chat_id: None,
            channel: "ch".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            completed_at_ms: None,
        }
    }

    fn task(
        id: &str,
        run_id: &str,
        plan_id: Option<&str>,
        parent: Option<&str>,
        started: Option<u64>,
        updated: u64,
    ) -> TaskRecord {
        TaskRecord {
            task_id: id.into(),
            run_id: run_id.into(),
            turn_id: None,
            parent_task_id: parent.map(Into::into),
            trace_id: "t".into(),
            job_id: "j".into(),
            parent_job_id: None,
            plan_id: plan_id.map(Into::into),
            kind: TaskKind::Main,
            name: "n".into(),
            task_type: "main".into(),
            description: "d".into(),
            status: TaskStatus::Running,
            execution_mode: None,
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: started,
            updated_at_ms: updated,
            completed_at_ms: None,
            error: None,
        }
    }

    #[test]
    fn tasks_for_run_sorted_by_started_at() {
        let mut snap = make_snapshot();
        snap.tasks
            .insert("t1".into(), task("t1", "r1", None, None, Some(300), 300));
        snap.tasks
            .insert("t2".into(), task("t2", "r1", None, None, Some(100), 100));
        snap.tasks
            .insert("t3".into(), task("t3", "r1", None, None, Some(200), 200));
        let result = tasks_for_run(&snap, "r1");
        let ids: Vec<&str> = result.iter().map(|t| t.task_id.as_str()).collect();
        assert_eq!(ids, vec!["t2", "t3", "t1"]);
    }

    #[test]
    fn tasks_for_run_fallback_to_updated_at() {
        let mut snap = make_snapshot();
        snap.tasks
            .insert("t1".into(), task("t1", "r1", None, None, None, 50));
        snap.tasks
            .insert("t2".into(), task("t2", "r1", None, None, None, 100));
        let result = tasks_for_run(&snap, "r1");
        let ids: Vec<&str> = result.iter().map(|t| t.task_id.as_str()).collect();
        assert_eq!(ids, vec!["t1", "t2"]);
    }

    #[test]
    fn run_by_job_id_uses_index() {
        let mut snap = make_snapshot();
        snap.runs.insert("r1".into(), run("r1", "j1"));
        snap.index
            .run_ids_by_job_id
            .insert("j1".into(), "r1".into());
        assert_eq!(run_by_job_id(&snap, "j1").unwrap().run_id, "r1");
        assert!(run_by_job_id(&snap, "missing").is_none());
    }

    #[test]
    fn plan_task_vs_subtasks() {
        let mut snap = make_snapshot();
        let root = task("root", "r1", Some("p1"), None, Some(10), 10);
        let sub1 = task("sub1", "r1", Some("p1"), Some("root"), Some(20), 20);
        let sub2 = task("sub2", "r1", Some("p1"), Some("root"), Some(30), 30);
        snap.tasks.insert("root".into(), root);
        snap.tasks.insert("sub1".into(), sub1);
        snap.tasks.insert("sub2".into(), sub2);
        snap.index.task_ids_by_plan_id.insert(
            "p1".into(),
            vec!["root".into(), "sub1".into(), "sub2".into()],
        );

        assert_eq!(plan_task(&snap, "p1").unwrap().task_id, "root");
        let subs: Vec<&str> = subtasks_for_plan(&snap, "p1")
            .iter()
            .map(|t| t.task_id.as_str())
            .collect();
        assert_eq!(subs, vec!["sub1", "sub2"]);
    }

    #[test]
    fn active_suspension_only_returns_active() {
        let mut snap = make_snapshot();
        snap.suspensions.insert(
            "s1".into(),
            SuspensionRecord {
                suspension_id: "s1".into(),
                run_id: "r1".into(),
                turn_id: "t1".into(),
                task_id: None,
                question_id: None,
                invocation_id: None,
                kind: SuspensionKind::Question,
                status: SuspensionStatus::Resolved,
                prompt: None,
                options: vec![],
                raw_payload: json!({}),
                resolution_source: None,
                answer_summary: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                resolved_at_ms: None,
                cleared_at_ms: None,
            },
        );
        assert!(active_suspension(&snap, "r1").is_none());

        snap.suspensions.insert(
            "s2".into(),
            SuspensionRecord {
                suspension_id: "s2".into(),
                run_id: "r1".into(),
                turn_id: "t1".into(),
                task_id: None,
                question_id: None,
                invocation_id: None,
                kind: SuspensionKind::Question,
                status: SuspensionStatus::Active,
                prompt: None,
                options: vec![],
                raw_payload: json!({}),
                resolution_source: None,
                answer_summary: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                resolved_at_ms: None,
                cleared_at_ms: None,
            },
        );
        assert_eq!(active_suspension(&snap, "r1").unwrap().suspension_id, "s2");
    }

    #[test]
    fn latest_tool_invocation_picks_newest() {
        let mut snap = make_snapshot();
        let inv = |id: &str, started: u64| ToolInvocationRuntimeRecord {
            invocation_id: id.into(),
            run_id: "r1".into(),
            turn_id: None,
            task_id: None,
            tool_name: "bash".into(),
            tool_use_id: None,
            phase: "done".into(),
            permission_state: "allowed".into(),
            input_preview: None,
            result_summary: None,
            error_summary: None,
            started_at_ms: started,
            finished_at_ms: None,
        };
        snap.tool_invocations.insert("i1".into(), inv("i1", 100));
        snap.tool_invocations.insert("i2".into(), inv("i2", 300));
        snap.tool_invocations.insert("i3".into(), inv("i3", 200));
        assert_eq!(
            latest_tool_invocation(&snap, "r1").unwrap().invocation_id,
            "i2"
        );
    }
}
