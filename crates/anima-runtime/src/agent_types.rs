use serde_json::{json, Value};
use std::collections::VecDeque;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub trace_id: String,
    pub task_type: String,
    pub payload: Value,
    pub priority: u8,
    pub timeout_ms: u64,
    pub metadata: Value,
}

pub fn make_task(input: MakeTask) -> Task {
    Task {
        id: Uuid::new_v4().to_string(),
        trace_id: input.trace_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        task_type: input.task_type,
        payload: input.payload.unwrap_or_else(|| json!({})),
        priority: input.priority.unwrap_or(5),
        timeout_ms: input.timeout_ms.unwrap_or(60_000),
        metadata: input.metadata.unwrap_or_else(|| json!({})),
    }
}

#[derive(Debug, Default)]
pub struct MakeTask {
    pub trace_id: Option<String>,
    pub task_type: String,
    pub payload: Option<Value>,
    pub priority: Option<u8>,
    pub timeout_ms: Option<u64>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskResult {
    pub task_id: String,
    pub trace_id: String,
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub worker_id: Option<String>,
}

pub fn make_task_result(input: MakeTaskResult) -> TaskResult {
    TaskResult {
        task_id: input.task_id,
        trace_id: input.trace_id,
        status: input.status,
        result: input.result,
        error: input.error,
        duration_ms: input.duration_ms,
        worker_id: input.worker_id,
    }
}

#[derive(Debug, Default)]
pub struct MakeTaskResult {
    pub task_id: String,
    pub trace_id: String,
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionPlanKind {
    Direct,
    Single,
    Sequential,
    Parallel,
    SpecialistRoute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    pub kind: ExecutionPlanKind,
    pub plan_type: String,
    pub tasks: VecDeque<Task>,
    pub specialist: Option<String>,
}
