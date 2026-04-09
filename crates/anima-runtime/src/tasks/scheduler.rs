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
