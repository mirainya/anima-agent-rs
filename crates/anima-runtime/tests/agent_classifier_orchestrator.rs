use anima_runtime::agent::{TaskExecutor, WorkerPool};
use anima_runtime::agent_classifier::{AgentClassifier, ClassificationKind};
use anima_runtime::agent_orchestrator::AgentOrchestrator;
use anima_runtime::agent_types::ExecutionPlanKind;
use anima_runtime::bus::{make_inbound, MakeInbound};
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
fn agent_classifier_preserves_direct_single_and_sequential_decisions() {
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

    let sequential = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "do multi".into(),
        metadata: Some(json!({"multi-step": true})),
        ..Default::default()
    });
    assert_eq!(
        AgentClassifier::classify(&sequential).kind,
        ClassificationKind::Sequential
    );
    assert_eq!(
        AgentClassifier::build_plan(&sequential).plan_type,
        "sequential"
    );
    assert_eq!(
        AgentClassifier::build_plan(&sequential).kind,
        ExecutionPlanKind::Sequential
    );
    assert_eq!(AgentClassifier::build_plan(&sequential).tasks.len(), 2);

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
fn agent_classifier_builds_parallel_and_specialist_route_skeletons() {
    let parallel = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "fan out".into(),
        metadata: Some(json!({"parallel": true})),
        ..Default::default()
    });
    let parallel_plan = AgentClassifier::build_plan(&parallel);
    assert_eq!(
        AgentClassifier::classify(&parallel).kind,
        ClassificationKind::Parallel
    );
    assert_eq!(parallel_plan.kind, ExecutionPlanKind::Parallel);
    assert_eq!(parallel_plan.tasks.len(), 2);

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
fn agent_orchestrator_executes_single_and_sequential_plans() {
    let worker_pool = Arc::new(WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
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

    let sequential = AgentClassifier::build_plan(&make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "multi".into(),
        metadata: Some(json!({"multi-step": true})),
        ..Default::default()
    }));
    let sequential_result = AgentOrchestrator::execute_plan(&worker_pool, &sequential, "session-2");
    assert_eq!(sequential_result.status, "success");

    worker_pool.stop();
}

#[test]
fn agent_classifier_detects_web_orchestration_v1_upgrade_conservatively() {
    let web_complex = make_inbound(MakeInbound {
        channel: "web".into(),
        content: "build a web app with frontend and backend".into(),
        ..Default::default()
    });
    assert!(AgentClassifier::should_upgrade_to_orchestration_v1(
        &web_complex
    ));

    let cli_same_text = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "build a web app with frontend and backend".into(),
        ..Default::default()
    });
    assert!(!AgentClassifier::should_upgrade_to_orchestration_v1(
        &cli_same_text
    ));

    let forced = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "plain request".into(),
        metadata: Some(json!({"orchestration-v1": true})),
        ..Default::default()
    });
    assert!(AgentClassifier::should_upgrade_to_orchestration_v1(&forced));
}

#[test]
fn agent_orchestrator_executes_parallel_and_specialist_route_skeletons() {
    let worker_pool = Arc::new(WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(100),
    ));
    worker_pool.start();

    let parallel = AgentClassifier::build_plan(&make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "fan out".into(),
        metadata: Some(json!({"parallel": true})),
        ..Default::default()
    }));
    let parallel_result = AgentOrchestrator::execute_plan(&worker_pool, &parallel, "session-3");
    assert_eq!(parallel_result.status, "success");
    assert_eq!(
        parallel_result.result.as_ref().unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

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
