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
