use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::runtime_error::{RuntimeError, RuntimeErrorKind, RuntimeErrorStage};
use crate::agent::types::{make_task, MakeTask, Task, TaskResult};
use crate::worker::WorkerPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKind {
    Initial,
    QuestionContinuation,
    Followup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiCallExecutionRequest {
    pub trace_id: String,
    pub session_id: String,
    pub content: String,
    pub kind: ExecutionKind,
    pub metadata: Option<Value>,
}

pub fn build_api_call_task(request: &ApiCallExecutionRequest) -> Task {
    make_task(MakeTask {
        trace_id: Some(request.trace_id.clone()),
        task_type: "api-call".into(),
        payload: Some(json!({
            "opencode-session-id": request.session_id,
            "content": request.content,
        })),
        metadata: request.metadata.clone(),
        ..Default::default()
    })
}

pub fn execute_api_call(
    worker_pool: &Arc<WorkerPool>,
    request: ApiCallExecutionRequest,
) -> Result<TaskResult, RuntimeError> {
    let task = build_api_call_task(&request);
    worker_pool
        .submit_task(task)
        .recv()
        .map_err(|error| {
            RuntimeError::new(
                RuntimeErrorKind::TaskExecutionFailed,
                RuntimeErrorStage::PlanExecute,
                format!("Failed to receive {:?} result: {error}", request.kind),
            )
        })
}
