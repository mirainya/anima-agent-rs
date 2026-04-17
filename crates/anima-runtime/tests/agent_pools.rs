use anima_runtime::agent::{TaskExecutor, WorkerPool};
use anima_runtime::agent_parallel_pool::ParallelPool;
use anima_runtime::agent_specialist_pool::SpecialistPool;
use anima_runtime::agent_types::{make_task, MakeTask};
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Default)]
struct MockExecutor;

impl TaskExecutor for MockExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({
            "content": format!("reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "mock-session-1"}))
    }
}

#[test]
fn parallel_pool_executes_and_aggregates_results() {
    let worker_pool = Arc::new(WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(100),
    ));
    worker_pool.start();

    let parallel_pool = ParallelPool::new(worker_pool.clone());
    let result = parallel_pool.execute(vec![
        make_task(MakeTask {
            task_type: "transform".into(),
            payload: Some(json!({"data": {"branch": 1}})),
            ..Default::default()
        }),
        make_task(MakeTask {
            task_type: "transform".into(),
            payload: Some(json!({"data": {"branch": 2}})),
            ..Default::default()
        }),
    ]);

    assert_eq!(result.status, "success");
    assert_eq!(
        result.result.unwrap()["results"].as_array().unwrap().len(),
        2
    );
    worker_pool.stop();
}

#[test]
fn specialist_pool_routes_and_wraps_result() {
    let worker_pool = Arc::new(WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        Some(1),
        None,
        Some(100),
    ));
    worker_pool.start();

    let specialist_pool = SpecialistPool::new(worker_pool.clone());
    let result = specialist_pool.execute(
        "math",
        make_task(MakeTask {
            task_type: "transform".into(),
            payload: Some(json!({"data": {"value": 42}})),
            ..Default::default()
        }),
    );

    assert_eq!(result.status, "success");
    assert_eq!(result.result.unwrap()["specialist"], "math");
    worker_pool.stop();
}
