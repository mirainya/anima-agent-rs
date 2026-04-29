use anima_runtime::agent::types::ExecutionPlanKind;
use anima_runtime::agent::{TaskExecutor, WorkerPool};
use anima_runtime::bus::{make_inbound, MakeInbound};
use anima_runtime::classifier::rule::{AgentClassifier, ClassificationKind};
use anima_runtime::orchestrator::core::AgentOrchestrator;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Default)]
struct MockExecutor;

impl TaskExecutor for MockExecutor {
    fn send_prompt(
        &self,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({
            "content": format!("reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "mock-session-1"}))
    }
}

#[test]
fn agent_classifier_preserves_direct_single_and_specialist_decisions() {
    let direct = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "/status".into(),
        metadata: Some(json!({"system-command": true})),
        ..Default::default()
    });
    assert_eq!(
        AgentClassifier::classify(&direct).kind,
        ClassificationKind::Direct
    );
    assert_eq!(AgentClassifier::build_plan(&direct).plan_type, "direct");
    assert_eq!(
        AgentClassifier::build_plan(&direct).kind,
        ExecutionPlanKind::Direct
    );

    // multi-step and parallel metadata no longer affect classification
    let multi_step = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "do multi".into(),
        metadata: Some(json!({"multi-step": true})),
        ..Default::default()
    });
    assert_eq!(
        AgentClassifier::classify(&multi_step).kind,
        ClassificationKind::Single
    );

    let parallel = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "fan out".into(),
        metadata: Some(json!({"parallel": true})),
        ..Default::default()
    });
    assert_eq!(
        AgentClassifier::classify(&parallel).kind,
        ClassificationKind::Single
    );

    let single = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "hello".into(),
        ..Default::default()
    });
    assert_eq!(
        AgentClassifier::classify(&single).kind,
        ClassificationKind::Single
    );
    assert_eq!(AgentClassifier::build_plan(&single).plan_type, "single");
    assert_eq!(
        AgentClassifier::build_plan(&single).kind,
        ExecutionPlanKind::Single
    );
    assert_eq!(AgentClassifier::build_plan(&single).tasks.len(), 1);
}

#[test]
fn agent_classifier_builds_specialist_route_skeleton() {
    let specialist = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "specialize".into(),
        metadata: Some(json!({"specialist": "math"})),
        ..Default::default()
    });
    let specialist_plan = AgentClassifier::build_plan(&specialist);
    assert_eq!(
        AgentClassifier::classify(&specialist).kind,
        ClassificationKind::SpecialistRoute
    );
    assert_eq!(specialist_plan.kind, ExecutionPlanKind::SpecialistRoute);
    assert_eq!(specialist_plan.specialist.as_deref(), Some("math"));
}

#[test]
fn agent_orchestrator_executes_single_plan() {
    let worker_pool = Arc::new(WorkerPool::new(
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(100),
    ));
    worker_pool.start();

    let single = AgentClassifier::build_plan(&make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "hello".into(),
        ..Default::default()
    }));
    let single_result = AgentOrchestrator::execute_plan(&worker_pool, &single, "session-1");
    assert_eq!(single_result.status, "success");
    assert_eq!(
        single_result.result.unwrap()["content"],
        "reply[session-1]: hello"
    );

    worker_pool.stop();
}

#[test]
fn agent_orchestrator_executes_specialist_route_skeleton() {
    let worker_pool = Arc::new(WorkerPool::new(
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(100),
    ));
    worker_pool.start();

    let specialist = AgentClassifier::build_plan(&make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "specialize".into(),
        metadata: Some(json!({"specialist": "math"})),
        ..Default::default()
    }));
    let specialist_result = AgentOrchestrator::execute_plan(&worker_pool, &specialist, "session-4");
    assert_eq!(specialist_result.status, "success");
    assert_eq!(specialist_result.result.unwrap()["specialist"], "math");

    worker_pool.stop();
}
