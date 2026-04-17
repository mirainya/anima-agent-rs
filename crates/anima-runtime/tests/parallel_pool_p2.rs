use anima_runtime::agent_executor::TaskExecutor;
use anima_runtime::agent_parallel_pool::*;
use anima_runtime::agent_types::*;
use anima_runtime::agent_worker::WorkerPool;
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::Arc;

struct EchoExecutor;
impl TaskExecutor for EchoExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({
            "content": format!("echo[{session_id}]: {}", content)
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "echo-session-1"}))
    }
}

struct FailExecutor;
impl TaskExecutor for FailExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Err("intentional failure".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Err("intentional failure".into())
    }
}

fn make_pool(executor: Arc<dyn TaskExecutor>) -> Arc<WorkerPool> {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let pool = Arc::new(WorkerPool::new(client, executor, Some(2), None, Some(100)));
    pool.start();
    pool
}

fn make_test_task(task_type: &str) -> Task {
    make_task(MakeTask {
        task_type: "transform".into(),
        payload: Some(json!({"data": {"type": task_type}})),
        ..Default::default()
    })
}

#[test]
fn parallel_pool_config_defaults() {
    let config = ParallelPoolConfig::default();
    assert_eq!(config.max_concurrent, 10);
    assert_eq!(config.default_timeout_ms, 60_000);
    assert!(!config.fail_fast);
    assert!((config.min_success_ratio - 0.5).abs() < f64::EPSILON);
}

#[test]
fn parallel_pool_lifecycle() {
    let pool = ParallelPool::new(make_pool(Arc::new(EchoExecutor)));
    assert!(!pool.is_running());
    pool.start();
    assert!(pool.is_running());
    pool.stop();
    assert!(!pool.is_running());
}

#[test]
fn parallel_pool_execute_batch_success() {
    let pool = ParallelPool::new(make_pool(Arc::new(EchoExecutor)));
    let tasks = vec![
        make_test_task("a"),
        make_test_task("b"),
        make_test_task("c"),
    ];
    let result = pool.execute_batch(tasks, None);
    assert_eq!(result.successful, 3);
    assert_eq!(result.failed, 0);
    assert!(result.success_ratio > 0.99);
}

#[test]
fn parallel_pool_execute_batch_with_failures() {
    let pool = ParallelPool::new(make_pool(Arc::new(FailExecutor)));
    let tasks: Vec<Task> = (0..2)
        .map(|_| {
            make_task(MakeTask {
                task_type: "session-create".into(),
                payload: Some(json!({})),
                ..Default::default()
            })
        })
        .collect();
    let result = pool.execute_batch(tasks, None);
    assert_eq!(result.failed, 2);
    assert_eq!(result.successful, 0);
    assert!(result.success_ratio < 0.01);
}

#[test]
fn parallel_pool_metrics_accumulate() {
    let pool = ParallelPool::new(make_pool(Arc::new(EchoExecutor)));
    let tasks = vec![make_test_task("a"), make_test_task("b")];
    pool.execute_batch(tasks, None);

    let m = pool.metrics();
    assert_eq!(m.batches_executed, 1);
    assert_eq!(m.total_tasks, 2);
    assert_eq!(m.successful_tasks, 2);
}

#[test]
fn parallel_pool_status_returns_json() {
    let pool = ParallelPool::new(make_pool(Arc::new(EchoExecutor)));
    pool.start();
    let status = pool.status();
    assert_eq!(status["running"], true);
    assert!(status["config"]["max_concurrent"].is_number());
}

#[test]
fn parallel_pool_backward_compat_execute() {
    let pool = ParallelPool::new(make_pool(Arc::new(EchoExecutor)));
    let tasks = vec![make_test_task("x")];
    let result = pool.execute(tasks);
    assert_eq!(result.status, "success");
}
