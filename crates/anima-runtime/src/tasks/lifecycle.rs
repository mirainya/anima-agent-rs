use super::model::{TaskRecord, TaskStatus};

pub fn mark_task_running(task: &mut TaskRecord, now_ms: u64) {
    task.status = TaskStatus::Running;
    task.updated_at_ms = now_ms;
    if task.started_at_ms.is_none() {
        task.started_at_ms = Some(now_ms);
    }
}

pub fn mark_task_suspended(task: &mut TaskRecord, now_ms: u64) {
    task.status = TaskStatus::Suspended;
    task.updated_at_ms = now_ms;
    if task.started_at_ms.is_none() {
        task.started_at_ms = Some(now_ms);
    }
}

pub fn mark_task_completed(task: &mut TaskRecord, now_ms: u64, result_kind: Option<String>) {
    task.status = TaskStatus::Completed;
    task.updated_at_ms = now_ms;
    task.completed_at_ms = Some(now_ms);
    if task.started_at_ms.is_none() {
        task.started_at_ms = Some(now_ms);
    }
    task.result_kind = result_kind;
}

pub fn mark_task_failed(task: &mut TaskRecord, now_ms: u64, error: Option<String>) {
    task.status = TaskStatus::Failed;
    task.updated_at_ms = now_ms;
    task.completed_at_ms = Some(now_ms);
    if task.started_at_ms.is_none() {
        task.started_at_ms = Some(now_ms);
    }
    task.error = error;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn blank_task() -> TaskRecord {
        TaskRecord {
            task_id: "t1".into(),
            run_id: "r1".into(),
            turn_id: None,
            parent_task_id: None,
            trace_id: "tr".into(),
            job_id: "j".into(),
            parent_job_id: None,
            plan_id: None,
            kind: crate::tasks::model::TaskKind::Main,
            name: "n".into(),
            task_type: "main".into(),
            description: "d".into(),
            status: TaskStatus::Pending,
            execution_mode: None,
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: None,
            updated_at_ms: 0,
            completed_at_ms: None,
            error: None,
        }
    }

    #[test]
    fn running_sets_started_at_only_once() {
        let mut task = blank_task();
        mark_task_running(&mut task, 100);
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.started_at_ms, Some(100));
        mark_task_running(&mut task, 200);
        assert_eq!(task.started_at_ms, Some(100));
        assert_eq!(task.updated_at_ms, 200);
    }

    #[test]
    fn completed_sets_result_kind_and_completed_at() {
        let mut task = blank_task();
        mark_task_completed(&mut task, 300, Some("success".into()));
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.completed_at_ms, Some(300));
        assert_eq!(task.result_kind, Some("success".into()));
        assert_eq!(task.started_at_ms, Some(300));
    }

    #[test]
    fn failed_sets_error() {
        let mut task = blank_task();
        mark_task_running(&mut task, 100);
        mark_task_failed(&mut task, 200, Some("boom".into()));
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error, Some("boom".into()));
        assert_eq!(task.completed_at_ms, Some(200));
        assert_eq!(task.started_at_ms, Some(100));
    }

    #[test]
    fn suspended_preserves_started_at() {
        let mut task = blank_task();
        mark_task_running(&mut task, 50);
        mark_task_suspended(&mut task, 100);
        assert_eq!(task.status, TaskStatus::Suspended);
        assert_eq!(task.started_at_ms, Some(50));
    }
}
