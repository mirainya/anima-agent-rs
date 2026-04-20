use std::sync::{Arc, Mutex};

use anima_runtime::agent::TaskExecutor;
use anima_runtime::agent::worker::WorkerPool;
use anima_runtime::execution::driver::{execute_api_call, ApiCallExecutionRequest, ExecutionKind};
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};

#[derive(Debug, Default)]
struct RecordingExecutor {
    payloads: Mutex<Vec<Value>>,
}

impl TaskExecutor for RecordingExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        self.payloads.lock().unwrap().push(content.clone());
        Ok(json!({
            "session_id": session_id,
            "echo": content,
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "unused"}))
    }
}

#[test]
fn execute_api_call_sends_expected_payload() {
    let executor = Arc::new(RecordingExecutor::default());
    let worker_pool = Arc::new(WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        executor.clone(),
        Some(1),
        None,
        None,
    ));
    worker_pool.start();

    let result = execute_api_call(
        &worker_pool,
        ApiCallExecutionRequest {
            trace_id: "trace-1".into(),
            session_id: "session-123".into(),
            content: "hello execution driver".into(),
            kind: ExecutionKind::QuestionContinuation,
            metadata: Some(json!({})),
        },
    )
    .expect("execution should succeed");

    assert_eq!(result.status, "success");
    assert_eq!(result.result.as_ref().unwrap()["session_id"], "session-123");
    assert_eq!(
        result.result.as_ref().unwrap()["echo"],
        "hello execution driver"
    );

    let recorded = executor.payloads.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0], Value::String("hello execution driver".into()));

    worker_pool.stop();
}
