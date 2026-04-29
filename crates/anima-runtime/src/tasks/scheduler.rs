use super::model::{TaskRecord, TaskStatus};

pub fn ready_task_ids(tasks: impl IntoIterator<Item = TaskRecord>) -> Vec<String> {
    let all = tasks.into_iter().collect::<Vec<_>>();
    all.iter()
        .filter(|task| task.status == TaskStatus::Pending)
        .filter(|task| {
            task.dependencies.iter().all(|dependency| {
                all.iter()
                    .find(|candidate| candidate.task_id == *dependency)
                    .is_none_or(|candidate| candidate.status == TaskStatus::Completed)
            })
        })
        .map(|task| task.task_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(id: &str, status: TaskStatus, deps: Vec<&str>) -> TaskRecord {
        TaskRecord {
            task_id: id.into(),
            run_id: "r".into(),
            turn_id: None,
            parent_task_id: None,
            trace_id: "t".into(),
            job_id: "j".into(),
            parent_job_id: None,
            plan_id: None,
            kind: crate::tasks::model::TaskKind::Main,
            name: "n".into(),
            task_type: "main".into(),
            description: "d".into(),
            status,
            execution_mode: None,
            result_kind: None,
            specialist_type: None,
            dependencies: deps.into_iter().map(Into::into).collect(),
            metadata: json!({}),
            started_at_ms: None,
            updated_at_ms: 0,
            completed_at_ms: None,
            error: None,
        }
    }

    #[test]
    fn no_deps_all_ready() {
        let ids = ready_task_ids(vec![
            task("a", TaskStatus::Pending, vec![]),
            task("b", TaskStatus::Pending, vec![]),
        ]);
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn dep_completed_is_ready() {
        let ids = ready_task_ids(vec![
            task("a", TaskStatus::Completed, vec![]),
            task("b", TaskStatus::Pending, vec!["a"]),
        ]);
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn dep_not_completed_blocks() {
        let ids = ready_task_ids(vec![
            task("a", TaskStatus::Running, vec![]),
            task("b", TaskStatus::Pending, vec!["a"]),
        ]);
        assert!(ids.is_empty());
    }

    #[test]
    fn missing_dep_treated_as_satisfied() {
        let ids = ready_task_ids(vec![task("b", TaskStatus::Pending, vec!["nonexistent"])]);
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn non_pending_tasks_excluded() {
        let ids = ready_task_ids(vec![
            task("a", TaskStatus::Running, vec![]),
            task("b", TaskStatus::Completed, vec![]),
        ]);
        assert!(ids.is_empty());
    }
}
