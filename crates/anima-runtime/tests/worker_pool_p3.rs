use anima_runtime::worker::executor::TaskExecutor;
use anima_runtime::agent::types::{make_task, MakeTask};
use anima_runtime::worker::WorkerPool;
use serde_json::{json, Value};
use std::sync::Arc;

struct TestExecutor;

impl TaskExecutor for TestExecutor {
    fn send_prompt(
        &self,        session_id: &str,
        _content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"content": format!("reply[{session_id}]")}))
    }

    fn create_session(&self) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "test-session-1"}))
    }
}

fn make_pool(size: usize) -> WorkerPool {
    let executor: Arc<dyn TaskExecutor> = Arc::new(TestExecutor);
    WorkerPool::new(executor, Some(size), None, Some(100))
}

fn make_pool_with_bounds(size: usize, min: usize, max: usize) -> WorkerPool {
    let executor: Arc<dyn TaskExecutor> = Arc::new(TestExecutor);
    WorkerPool::new(executor, Some(size), None, Some(100)).with_bounds(min, max)
}

fn test_task() -> anima_runtime::agent::types::Task {
    make_task(MakeTask {
        task_type: "session-create".into(),
        ..Default::default()
    })
}

// ── Basic pool lifecycle ────────────────────────────────────────────

#[test]
fn pool_creates_with_initial_size() {
    let pool = make_pool(3);
    assert_eq!(pool.size(), 3);
    assert!(!pool.is_running());
}

#[test]
fn pool_start_and_stop() {
    let pool = make_pool(2);
    pool.start();
    assert!(pool.is_running());
    pool.stop();
    assert!(!pool.is_running());
}

#[test]
fn pool_status_reports_correctly() {
    let pool = make_pool(2);
    let status = pool.status();
    assert_eq!(status.status, "stopped");
    assert_eq!(status.size, 2);
    assert_eq!(status.workers.len(), 2);

    pool.start();
    let status = pool.status();
    assert_eq!(status.status, "running");
}

// ── Scale to ────────────────────────────────────────────────────────

#[test]
fn scale_up_adds_workers() {
    let pool = make_pool_with_bounds(2, 1, 8);
    assert_eq!(pool.size(), 2);
    let new_size = pool.scale_to(5);
    assert_eq!(new_size, 5);
    assert_eq!(pool.size(), 5);
}

#[test]
fn scale_down_removes_workers() {
    let pool = make_pool_with_bounds(4, 1, 8);
    pool.start();
    let new_size = pool.scale_to(2);
    assert_eq!(new_size, 2);
    assert_eq!(pool.size(), 2);
}

#[test]
fn scale_to_respects_min_bound() {
    let pool = make_pool_with_bounds(3, 2, 8);
    let new_size = pool.scale_to(1); // below min
    assert_eq!(new_size, 2); // clamped to min
}

#[test]
fn scale_to_respects_max_bound() {
    let pool = make_pool_with_bounds(3, 1, 5);
    let new_size = pool.scale_to(10); // above max
    assert_eq!(new_size, 5); // clamped to max
}

#[test]
fn scale_to_same_size_is_noop() {
    let pool = make_pool_with_bounds(3, 1, 8);
    let new_size = pool.scale_to(3);
    assert_eq!(new_size, 3);
}

#[test]
fn scaled_up_workers_start_if_pool_running() {
    let pool = make_pool_with_bounds(2, 1, 8);
    pool.start();
    pool.scale_to(4);

    let status = pool.status();
    assert_eq!(status.workers.len(), 4);
    for w in &status.workers {
        assert_eq!(w.status, "idle");
    }
}

// ── Task submission ─────────────────────────────────────────────────

#[test]
fn submit_task_to_running_pool() {
    let pool = make_pool(2);
    pool.start();
    let rx = pool.submit_task(test_task());
    let result = rx.recv().unwrap();
    assert_eq!(result.status, "success");
}

#[test]
fn submit_task_to_stopped_pool_fails() {
    let pool = make_pool(2);
    let rx = pool.submit_task(test_task());
    let result = rx.recv().unwrap();
    assert_eq!(result.status, "failure");
    assert!(result.error.unwrap().contains("not running"));
}
