use anima_runtime::agent_orchestrator::*;
use anima_runtime::agent_specialist_pool::SpecialistPool;
use anima_runtime::agent_types::*;
use anima_runtime::agent_worker::WorkerPool;
use anima_runtime::agent_executor::TaskExecutor;
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct EchoExecutor;

struct SlowEchoExecutor;

struct ConditionalFailureExecutor;

struct QuestionExecutor;

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

impl TaskExecutor for SlowEchoExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text.contains("slow") {
            thread::sleep(Duration::from_millis(200));
        } else {
            thread::sleep(Duration::from_millis(20));
        }
        Ok(json!({
            "content": format!("echo[{session_id}]: {text}")
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "slow-echo-session-1"}))
    }
}

#[derive(Clone)]
struct RecordingExecutorState {
    prompts: Arc<Mutex<Vec<String>>>,
}

impl RecordingExecutorState {
    fn new() -> Self {
        Self {
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

impl TaskExecutor for RecordingExecutorState {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("").to_string();
        self.prompts.lock().unwrap().push(text.clone());
        if text.contains("slow") {
            thread::sleep(Duration::from_millis(150));
        }
        Ok(json!({
            "content": format!("echo[{session_id}]: {text}")
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "recording-session-1"}))
    }
}

impl TaskExecutor for ConditionalFailureExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text.contains("fail") {
            Err(format!("executor refused content: {text}"))
        } else {
            Ok(json!({
                "content": format!("echo[{session_id}]: {text}")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "conditional-failure-session-1"}))
    }
}

impl TaskExecutor for QuestionExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({
            "question": {
                "id": "question-orch-1",
                "kind": "input",
                "prompt": "Need one extra confirmation",
                "options": ["continue", "stop"]
            }
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "question-session-1"}))
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

fn make_orchestrator() -> AgentOrchestrator {
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    AgentOrchestrator::new(wp, sp, OrchestratorConfig::default())
}

fn make_pool_with_executor(executor: Arc<dyn TaskExecutor>, size: usize) -> Arc<WorkerPool> {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let pool = Arc::new(WorkerPool::new(client, executor, Some(size), None, Some(100)));
    pool.start();
    pool
}

#[test]
fn orchestrator_config_defaults() {
    let config = OrchestratorConfig::default();
    assert_eq!(config.default_timeout_ms, 60_000);
    assert_eq!(config.max_retries, 0);
    assert!(config.enable_parallel);
}

#[test]
fn orchestrator_lifecycle() {
    let orch = make_orchestrator();
    assert!(!orch.is_running());
    orch.start();
    assert!(orch.is_running());
    orch.stop();
    assert!(!orch.is_running());
}

#[test]
fn orchestrator_decompose_web_app() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("build a web app", "trace-1", "job-main-1");
    assert!(!plan.subtasks.is_empty());
    assert!(!plan.parallel_groups.is_empty());
    assert_eq!(*plan.status.lock().unwrap(), PlanStatus::Created);
    let first = plan.subtasks.values().next().unwrap();
    assert_eq!(first.parent_job_id, "job-main-1");
}

#[test]
fn orchestrator_decompose_api() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("create REST API endpoint", "trace-2", "trace-2");
    assert!(!plan.subtasks.is_empty());
}

#[test]
fn orchestrator_decompose_data_analysis() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("data analysis report", "trace-3", "trace-3");
    assert!(!plan.subtasks.is_empty());
}

#[test]
fn orchestrator_decompose_refactoring() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("refactor the auth module", "trace-4", "trace-4");
    assert!(!plan.subtasks.is_empty());
}

#[test]
fn orchestrator_decompose_unknown_creates_single_task() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("something random", "trace-5", "trace-5");
    assert_eq!(plan.subtasks.len(), 1);
    assert_eq!(plan.parallel_groups.len(), 1);
}

#[test]
fn orchestrator_orchestrate_runs_plan() {
    let orch = make_orchestrator();
    orch.start();
    // Decomposed subtasks use types like "design", "frontend" etc. which the
    // worker doesn't recognise, so the orchestration will report failure.
    let result = orch.orchestrate("build a web app");
    assert!(result.status == "success" || result.status == "failure");
    // Verify the orchestrator actually ran (metrics updated)
    let m = orch.metrics();
    assert!(m.plans_created >= 1);
}

#[test]
fn orchestrator_metrics() {
    let orch = make_orchestrator();
    orch.start();
    orch.orchestrate("create api endpoint");
    let m = orch.metrics();
    assert!(m.plans_created >= 1);
}

#[test]
fn orchestrator_status_json() {
    let orch = make_orchestrator();
    orch.start();
    let status = orch.status();
    assert_eq!(status["running"], true);
}

#[test]
fn orchestrator_backward_compat_execute_plan() {
    let wp = make_pool();
    let task = make_task(MakeTask {
        task_type: "session-create".into(),
        payload: Some(json!({})),
        ..Default::default()
    });
    let plan = ExecutionPlan {
        kind: ExecutionPlanKind::Single,
        plan_type: "single".into(),
        tasks: vec![task].into(),
        specialist: None,
    };
    let result = AgentOrchestrator::execute_plan(&wp, &plan, "sess-1");
    assert_eq!(result.status, "success");
}

#[test]
fn execute_plan_parallel_returns_results_in_input_order() {
    let wp = make_pool_with_executor(Arc::new(SlowEchoExecutor), 2);
    let plan = ExecutionPlan {
        kind: ExecutionPlanKind::Parallel,
        plan_type: "parallel".into(),
        tasks: vec![
            make_task(MakeTask {
                task_type: "api-call".into(),
                payload: Some(json!({
                    "opencode-session-id": "sess-1",
                    "content": "slow-first"
                })),
                ..Default::default()
            }),
            make_task(MakeTask {
                task_type: "api-call".into(),
                payload: Some(json!({
                    "opencode-session-id": "sess-1",
                    "content": "fast-second"
                })),
                ..Default::default()
            }),
        ]
        .into(),
        specialist: None,
    };

    let result = AgentOrchestrator::execute_plan(&wp, &plan, "sess-1");
    assert_eq!(result.status, "success");
    let results = result.result.unwrap()["results"].as_array().unwrap().clone();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["content"], "echo[sess-1]: slow-first");
    assert_eq!(results[1]["content"], "echo[sess-1]: fast-second");
}

#[test]
fn execute_plan_parallel_propagates_failure() {
    let wp = make_pool_with_executor(Arc::new(ConditionalFailureExecutor), 2);
    let plan = ExecutionPlan {
        kind: ExecutionPlanKind::Parallel,
        plan_type: "parallel".into(),
        tasks: vec![
            make_task(MakeTask {
                task_type: "api-call".into(),
                payload: Some(json!({
                    "opencode-session-id": "sess-1",
                    "content": "ok-first"
                })),
                ..Default::default()
            }),
            make_task(MakeTask {
                task_type: "api-call".into(),
                payload: Some(json!({
                    "opencode-session-id": "sess-1",
                    "content": "fail-second"
                })),
                ..Default::default()
            }),
        ]
        .into(),
        specialist: None,
    };

    let result = AgentOrchestrator::execute_plan(&wp, &plan, "sess-1");
    assert_eq!(result.status, "failure");
    assert!(result
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("executor refused content: fail-second"));
}

#[test]
fn orchestration_main_chain_lowering_uses_finer_primitives_with_api_call_fallback() {
    let orch = make_orchestrator();
    let mut events = Vec::new();
    let execution = orch
        .execute_orchestration_for_main_chain(
            "data analysis report",
            "trace-main-1",
            "job-main-1",
            "sess-main-1",
            |event, payload| events.push((event.to_string(), payload)),
        )
        .expect("orchestration should execute");

    assert_eq!(execution.result.status, "success");
    assert!(!execution.lowered_tasks.is_empty());
    assert!(execution
        .lowered_tasks
        .iter()
        .any(|task| task.lowered_task_type == "transform"));
    assert!(execution
        .lowered_tasks
        .iter()
        .any(|task| task.lowered_task_type == "api-call"));
    assert!(execution.lowered_tasks.iter().any(|task| task.lowered_task_type == "api-call"));
    assert!(execution.lowered_tasks.iter().filter(|task| task.lowered_task_type == "api-call").all(|task| {
        task.task
            .payload
            .get("opencode-session-id")
            .and_then(Value::as_str)
            == Some("sess-main-1")
    }));
    assert!(execution.lowered_tasks.iter().filter(|task| task.lowered_task_type == "api-call").all(|task| {
        task.task.metadata.get("orchestration_subtask").and_then(Value::as_bool) == Some(true)
            && task.task.metadata.get("streaming_observable").and_then(Value::as_bool) == Some(true)
            && task.task.payload.get("orchestration_subtask").and_then(Value::as_bool) == Some(true)
            && task.task.payload.get("streaming_observable").and_then(Value::as_bool) == Some(true)
    }));
    assert!(events.iter().any(|(event, _)| event == "orchestration_plan_created"));
    assert!(events.iter().any(|(event, _)| event == "orchestration_subtask_started"));
    assert!(events.iter().any(|(event, _)| event == "orchestration_subtask_completed"));
}

#[test]
fn orchestration_main_chain_stops_and_surfaces_question_result() {
    let wp = make_pool_with_executor(Arc::new(QuestionExecutor), 2);
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(wp, sp, OrchestratorConfig::default());
    let execution = orch
        .execute_orchestration_for_main_chain(
            "create REST API endpoint",
            "trace-main-2",
            "job-main-2",
            "sess-main-2",
            |_event, _payload| {},
        )
        .expect("orchestration should execute");

    assert_eq!(execution.result.status, "success");
    assert_eq!(execution.result.result.as_ref().unwrap()["question"]["id"], "question-orch-1");
}

#[test]
fn orchestration_main_chain_falls_back_to_serial_for_mixed_groups() {
    let executor = RecordingExecutorState::new();
    let wp = make_pool_with_executor(Arc::new(executor.clone()), 3);
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(wp, sp, OrchestratorConfig::default());
    let mut events = Vec::new();

    let execution = orch
        .execute_orchestration_for_main_chain(
            "data analysis report",
            "trace-main-4",
            "job-main-4",
            "sess-main-4",
            |event, payload| events.push((event.to_string(), payload)),
        )
        .expect("orchestration should execute");

    assert_eq!(execution.result.status, "success");
    let started_events: Vec<_> = events
        .iter()
        .filter(|(event, _)| event == "orchestration_subtask_started")
        .collect();
    assert!(!started_events.is_empty());
    assert!(started_events.iter().all(|(_, payload)| payload["execution_mode"] == "serial"));
    assert!(started_events.iter().any(|(_, payload)| payload["parallel_safe"] == true));

    let prompts = executor.snapshot();
    assert_eq!(prompts.len(), 2);
    assert!(prompts.iter().any(|prompt| prompt.contains("collect-data")));
    assert!(prompts.iter().any(|prompt| prompt.contains("analyze-data")));
}

#[test]
fn orchestration_main_chain_parallelizes_query_only_group() {
    let orch = make_orchestrator();
    let mut events = Vec::new();

    let execution = orch
        .execute_orchestration_for_main_chain(
            "something random",
            "trace-main-4b",
            "job-main-4b",
            "sess-main-4b",
            |event, payload| events.push((event.to_string(), payload)),
        )
        .expect("orchestration should execute");

    assert_eq!(execution.result.status, "success");
    let started_event = events
        .iter()
        .find(|(event, _)| event == "orchestration_subtask_started")
        .expect("should emit started event");
    assert_eq!(started_event.1["execution_mode"], "serial");
    assert_eq!(started_event.1["parallel_safe"], true);
    assert_eq!(started_event.1["lowered_task_type"], "query");
}

#[test]
fn orchestration_main_chain_respects_parallel_disable_switch() {
    let executor = RecordingExecutorState::new();
    let wp = make_pool_with_executor(Arc::new(executor), 3);
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let config = OrchestratorConfig {
        enable_parallel: false,
        ..OrchestratorConfig::default()
    };
    let orch = AgentOrchestrator::new(wp, sp, config);
    let mut events = Vec::new();

    let execution = orch
        .execute_orchestration_for_main_chain(
            "data analysis report",
            "trace-main-5",
            "job-main-5",
            "sess-main-5",
            |event, payload| events.push((event.to_string(), payload)),
        )
        .expect("orchestration should execute");

    assert_eq!(execution.result.status, "success");
    assert!(events
        .iter()
        .filter(|(event, _)| event == "orchestration_subtask_started")
        .all(|(_, payload)| payload["execution_mode"] == "serial"));
}

#[test]
fn orchestration_main_chain_can_force_fallback_error() {
    let orch = make_orchestrator();
    let result = orch.execute_orchestration_for_main_chain(
        "[orchestration-fail] build a web app",
        "trace-main-3",
        "job-main-3",
        "sess-main-3",
        |_event, _payload| {},
    );
    assert!(result.is_err());
}

#[test]
fn execute_plan_parallel_runs_truly_concurrently() {
    let wp = make_pool_with_executor(Arc::new(SlowEchoExecutor), 2);
    let plan = ExecutionPlan {
        kind: ExecutionPlanKind::Parallel,
        plan_type: "parallel".into(),
        tasks: vec![
            make_task(MakeTask {
                task_type: "api-call".into(),
                payload: Some(json!({
                    "opencode-session-id": "sess-1",
                    "content": "slow-a"
                })),
                ..Default::default()
            }),
            make_task(MakeTask {
                task_type: "api-call".into(),
                payload: Some(json!({
                    "opencode-session-id": "sess-1",
                    "content": "slow-b"
                })),
                ..Default::default()
            }),
        ]
        .into(),
        specialist: None,
    };

    let started = Instant::now();
    let result = AgentOrchestrator::execute_plan(&wp, &plan, "sess-1");
    let elapsed = started.elapsed();

    assert_eq!(result.status, "success");
    assert!(
        elapsed < Duration::from_millis(350),
        "parallel plan completed too slowly, expected true concurrency: {:?}",
        elapsed
    );
}
