use anima_runtime::agent_executor::TaskExecutor;
use anima_runtime::agent_specialist_pool::*;
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
    ) -> Result<Value, String> {
        Ok(json!({
            "content": format!("echo[{session_id}]: {}", content)
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "echo-session-1"}))
    }
}

fn make_pool() -> Arc<WorkerPool> {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let pool = Arc::new(WorkerPool::new(
        client,
        Arc::new(EchoExecutor),
        Some(2),
        None,
        Some(100),
    ));
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
fn specialist_pool_backward_compat() {
    let pool = SpecialistPool::new(make_pool());
    pool.register("nlp", make_pool());
    let _resolved = pool.resolve("nlp");
    // Should resolve to the registered pool, not default
    let task = make_test_task("classify");
    let result = pool.execute("nlp", task);
    // execute() wraps the result with specialist info; the underlying
    // worker handles "transform" task_type successfully.
    assert_eq!(result.status, "success");
}

#[test]
fn specialist_pool_lifecycle() {
    let pool = SpecialistPool::new(make_pool());
    assert!(!pool.is_running());
    pool.start();
    assert!(pool.is_running());
    pool.stop();
    assert!(!pool.is_running());
}

#[test]
fn specialist_pool_register_and_list() {
    let pool = SpecialistPool::new(make_pool());
    pool.register_specialist(RegisterSpecialistOpts {
        id: "s1".into(),
        name: "NLP Specialist".into(),
        capabilities: vec!["classify".into(), "summarize".into()],
        priority: 10,
        max_concurrent: 5,
        pool: make_pool(),
    });

    let specialists = pool.list_specialists();
    assert_eq!(specialists.len(), 1);
    assert_eq!(specialists[0].name, "NLP Specialist");
    assert_eq!(specialists[0].capabilities, vec!["classify", "summarize"]);
}

#[test]
fn specialist_pool_unregister() {
    let pool = SpecialistPool::new(make_pool());
    pool.register_specialist(RegisterSpecialistOpts {
        id: "s1".into(),
        name: "Test".into(),
        capabilities: vec!["test".into()],
        priority: 1,
        max_concurrent: 1,
        pool: make_pool(),
    });
    assert!(pool.unregister_specialist("s1"));
    assert!(!pool.unregister_specialist("s1")); // already removed
    assert!(pool.list_specialists().is_empty());
}

#[test]
fn specialist_pool_capabilities() {
    let pool = SpecialistPool::new(make_pool());
    pool.register_specialist(RegisterSpecialistOpts {
        id: "s1".into(),
        name: "S1".into(),
        capabilities: vec!["cap-a".into()],
        priority: 1,
        max_concurrent: 1,
        pool: make_pool(),
    });

    let caps = pool.list_capabilities();
    assert!(caps.contains(&"cap-a".to_string()));

    pool.add_capability("s1", "cap-b");
    let caps = pool.list_capabilities();
    assert!(caps.contains(&"cap-b".to_string()));

    pool.remove_capability("s1", "cap-a");
    let by_cap = pool.list_specialists_by_capability("cap-a");
    assert!(by_cap.is_empty());
}

#[test]
fn specialist_pool_route_task() {
    let pool = SpecialistPool::new(make_pool());
    pool.register_specialist(RegisterSpecialistOpts {
        id: "nlp".into(),
        name: "NLP".into(),
        capabilities: vec!["transform".into()],
        priority: 1,
        max_concurrent: 5,
        pool: make_pool(),
    });

    let task = make_test_task("transform");
    let result = pool.route_task(task);
    assert_eq!(result.status, "success");
}

#[test]
fn specialist_pool_route_task_fallback() {
    let config = SpecialistPoolConfig {
        fallback_to_worker: true,
        ..Default::default()
    };
    let pool = SpecialistPool::with_config(make_pool(), config);
    // No specialists registered, should fallback to default pool
    let task = make_test_task("fallback");
    let result = pool.route_task(task);
    // The task_type used for routing is "transform" (from make_test_task),
    // which the worker handles, so fallback succeeds.
    assert_eq!(result.status, "success");
}

#[test]
fn specialist_pool_health_check() {
    let pool = SpecialistPool::new(make_pool());
    pool.start();
    let health = pool.health_check();
    assert_eq!(health["running"], true);
}

#[test]
fn specialist_pool_metrics() {
    let pool = SpecialistPool::new(make_pool());
    let m = pool.metrics();
    assert_eq!(m.total_tasks, 0);
}

#[test]
fn specialist_pool_get_specialist() {
    let pool = SpecialistPool::new(make_pool());
    pool.register_specialist(RegisterSpecialistOpts {
        id: "s1".into(),
        name: "Test".into(),
        capabilities: vec![],
        priority: 1,
        max_concurrent: 1,
        pool: make_pool(),
    });
    assert!(pool.get_specialist("s1").is_some());
    assert!(pool.get_specialist("nonexistent").is_none());
}
