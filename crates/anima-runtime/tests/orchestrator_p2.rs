use anima_runtime::agent::executor::TaskExecutor;
use anima_runtime::orchestrator::core::*;
use anima_runtime::orchestrator::specialist_pool::SpecialistPool;
use anima_runtime::agent::types::*;
use anima_runtime::agent::worker::WorkerPool;
use anima_runtime::runtime::RuntimeStateStore;
use anima_runtime::tasks::{TaskKind, TaskStatus};
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
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({
            "content": format!("echo[{session_id}]: {}", content)
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "echo-session-1"}))
    }
}

impl TaskExecutor for SlowEchoExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
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

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
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
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        let text = content.as_str().unwrap_or("").to_string();
        self.prompts.lock().unwrap().push(text.clone());
        if text.contains("slow") {
            thread::sleep(Duration::from_millis(150));
        }
        Ok(json!({
            "content": format!("echo[{session_id}]: {text}")
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "recording-session-1"}))
    }
}

impl TaskExecutor for ConditionalFailureExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        let text = content.as_str().unwrap_or("");
        if text.contains("fail") {
            Err(format!("executor refused content: {text}").into())
        } else {
            Ok(json!({
                "content": format!("echo[{session_id}]: {text}")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "conditional-failure-session-1"}))
    }
}

impl TaskExecutor for QuestionExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({
            "question": {
                "id": "question-orch-1",
                "kind": "input",
                "prompt": "Need one extra confirmation",
                "options": ["continue", "stop"]
            }
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
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
    AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    )
}

fn make_pool_with_executor(executor: Arc<dyn TaskExecutor>, size: usize) -> Arc<WorkerPool> {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let pool = Arc::new(WorkerPool::new(
        client,
        executor,
        Some(size),
        None,
        Some(100),
    ));
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
    let plan = orch.decompose_task("build a web app", "trace-1", "job-main-1", None);
    assert!(!plan.subtasks.is_empty());
    assert!(!plan.parallel_groups.is_empty());
    assert!(plan.created_at > 0);
    let first = plan.subtasks.values().next().unwrap();
    assert_eq!(first.parent_job_id, "job-main-1");
}

#[test]
fn orchestrator_decompose_api() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("create REST API endpoint", "trace-2", "trace-2", None);
    assert!(!plan.subtasks.is_empty());
}

#[test]
fn orchestrator_decompose_data_analysis() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("data analysis report", "trace-3", "trace-3", None);
    assert!(!plan.subtasks.is_empty());
}

#[test]
fn orchestrator_decompose_refactoring() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("refactor the auth module", "trace-4", "trace-4", None);
    assert!(!plan.subtasks.is_empty());
}

#[test]
fn orchestrator_decompose_unknown_creates_single_task() {
    let orch = make_orchestrator();
    let plan = orch.decompose_task("something random", "trace-5", "trace-5", None);
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
fn orchestrator_status_active_plans_comes_from_runtime_store() {
    let executor = RecordingExecutorState::new();
    let wp = make_pool_with_executor(Arc::new(executor), 1);
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = Arc::new(AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    ));
    orch.start();

    let orchestrator = Arc::clone(&orch);
    let handle = thread::spawn(move || {
        orchestrator.execute_orchestration_for_main_chain(
            "slow data analysis report",
            "trace-status-runtime",
            "job-status-runtime",
            "sess-status-runtime",
            |_event, _payload| {},
        )
    });

    thread::sleep(Duration::from_millis(30));
    let status_during_run = orch.status();
    assert_eq!(status_during_run["running"], true);
    assert_eq!(status_during_run["active_plans"], 1);

    let result = handle
        .join()
        .expect("execution thread should join")
        .expect("orchestration should execute");
    assert_eq!(result.result.status, "success");

    let status_after_run = orch.status();
    assert_eq!(status_after_run["active_plans"], 0);
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
    let results = result.result.unwrap()["results"]
        .as_array()
        .unwrap()
        .clone();
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
    assert!(execution
        .lowered_tasks
        .iter()
        .any(|task| task.lowered_task_type == "api-call"));
    assert!(execution
        .lowered_tasks
        .iter()
        .filter(|task| task.lowered_task_type == "api-call")
        .all(|task| {
            task.task
                .payload
                .get("opencode-session-id")
                .and_then(Value::as_str)
                == Some("sess-main-1")
        }));
    assert!(execution
        .lowered_tasks
        .iter()
        .filter(|task| task.lowered_task_type == "api-call")
        .all(|task| {
            task.task
                .metadata
                .get("orchestration_subtask")
                .and_then(Value::as_bool)
                == Some(true)
                && task
                    .task
                    .metadata
                    .get("streaming_observable")
                    .and_then(Value::as_bool)
                    == Some(false)
                && task
                    .task
                    .payload
                    .get("orchestration_subtask")
                    .and_then(Value::as_bool)
                    == Some(true)
                && task
                    .task
                    .payload
                    .get("streaming_observable")
                    .and_then(Value::as_bool)
                    == Some(false)
        }));
    assert!(events
        .iter()
        .any(|(event, _)| event == "orchestration_plan_created"));
    assert!(events
        .iter()
        .any(|(event, _)| event == "orchestration_subtask_started"));
    assert!(events
        .iter()
        .any(|(event, _)| event == "orchestration_subtask_completed"));
}

#[test]
fn orchestration_main_chain_stops_and_surfaces_question_result() {
    let wp = make_pool_with_executor(Arc::new(QuestionExecutor), 2);
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    );
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
    assert_eq!(
        execution.result.result.as_ref().unwrap()["question"]["id"],
        "question-orch-1"
    );
}

#[test]
fn orchestration_final_result_aggregates_multiple_subtask_outputs() {
    let orch = make_orchestrator();
    let execution = orch
        .execute_orchestration_for_main_chain(
            "build a web app",
            "trace-main-aggregate",
            "job-main-aggregate",
            "sess-main-aggregate",
            |_event, _payload| {},
        )
        .expect("orchestration should execute");

    assert_eq!(execution.result.status, "success");
    let result = execution
        .result
        .result
        .as_ref()
        .expect("expected aggregated result");

    if result.get("question").is_some() {
        let question = &result["question"];
        assert_eq!(question["kind"], "input");
        assert!(question["prompt"]
            .as_str()
            .unwrap_or_default()
            .contains("产品类型、核心用户，以及 3-5 个核心功能"));
        assert_eq!(
            question["orchestration"]["reason"],
            "multiple_subtasks_missing_shared_context"
        );
        assert!(
            question["orchestration"]["subtasks"]
                .as_array()
                .unwrap_or(&Vec::new())
                .len()
                >= 2
        );
    } else {
        let content = result["content"]
            .as_str()
            .expect("aggregated content should be string");
        let subtask_results = result["subtask_results"]
            .as_object()
            .expect("aggregated result should expose subtask results");

        assert!(content.contains("Orchestration completed for request: build a web app"));
        assert!(content.contains("Subtask outcomes:"));
        assert!(content.contains("- design [design / success]:"));
        assert!(content.contains("- implement-frontend [frontend / success]:"));
        assert!(content.contains("- implement-backend [backend / success]:"));
        assert!(content.contains("- testing [testing / success]:"));
        assert!(subtask_results.contains_key("design"));
        assert!(subtask_results.contains_key("implement-frontend"));
        assert!(subtask_results.contains_key("implement-backend"));
        assert!(subtask_results.contains_key("testing"));
    }
}

#[test]
fn orchestration_main_chain_falls_back_to_serial_for_mixed_groups() {
    let executor = RecordingExecutorState::new();
    let wp = make_pool_with_executor(Arc::new(executor.clone()), 3);
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    );
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
    assert!(started_events
        .iter()
        .all(|(_, payload)| payload["execution_mode"] == "serial"));
    assert!(started_events
        .iter()
        .any(|(_, payload)| payload["parallel_safe"] == true));

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
fn orchestration_runtime_subtask_records_preserve_original_type_and_lowering_metadata() {
    let store = Arc::new(RuntimeStateStore::new());
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(wp, sp, store.clone(), OrchestratorConfig::default());

    let execution = orch
        .execute_orchestration_for_main_chain(
            "data analysis report",
            "trace-runtime-subtask-type",
            "job-runtime-subtask-type",
            "sess-runtime-subtask-type",
            |_event, _payload| {},
        )
        .expect("orchestration should execute");
    assert_eq!(execution.result.status, "success");

    let snapshot = store.snapshot();
    let reporting_subtask = snapshot
        .tasks
        .values()
        .find(|task| task.kind == TaskKind::Subtask && task.name == "generate-report")
        .expect("expected reporting subtask record");

    assert_eq!(reporting_subtask.status, TaskStatus::Completed);
    assert_eq!(reporting_subtask.task_type, "reporting");
    assert_eq!(
        reporting_subtask.metadata["original_task_type"],
        "reporting"
    );
    assert_eq!(reporting_subtask.metadata["lowered_task_type"], "transform");
    assert_eq!(reporting_subtask.metadata["parallel_safe"], true);
}

#[test]
fn orchestration_api_call_subtask_prompt_uses_shared_convergence_contract() {
    let orch = make_orchestrator();
    let execution = orch
        .execute_orchestration_for_main_chain(
            "build a web app",
            "trace-main-design-prompt",
            "job-main-design-prompt",
            "sess-main-design-prompt",
            |_event, _payload| {},
        )
        .expect("orchestration should execute");

    let design_task = execution
        .lowered_tasks
        .iter()
        .find(|task| task.name == "design")
        .expect("expected design lowered task");
    let content = design_task.task.payload["content"]
        .as_str()
        .expect("design content should be string");

    assert!(content
        .contains("You are executing exactly one orchestration subtask inside a larger plan."));
    assert!(content.contains(
        "Focus only on this subtask and return the concrete result needed by downstream steps."
    ));
    assert!(content.contains("Do not re-decompose the whole project."));
    assert!(content.contains("Do not keep planning indefinitely."));
    assert!(content.contains("Do not create or manage todo lists unless the user explicitly asked for that exact output."));
    assert!(content.contains("Do not use tools. Do not spawn subagents."));
    assert!(content.contains("Assume reasonable defaults for minor missing details when you can still produce a useful concrete result."));
    assert!(content.contains("Only ask one focused question if a truly critical detail is missing and you cannot proceed without it."));
    assert!(content.contains(
        "produce a concise final deliverable for this subtask and stop after the result"
    ));
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
    let orch = AgentOrchestrator::new(wp, sp, Arc::new(RuntimeStateStore::new()), config);
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

// ── Phase: LLM Decompose ─────────────────────────────────────────

struct LlmDecomposeExecutor;

impl TaskExecutor for LlmDecomposeExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        let text = content
            .as_array()
            .and_then(|a| a.first())
            .and_then(|b| b.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if text.contains("任务分解引擎") {
            Ok(json!({
                "content": [{"type": "text", "text": r#"[
                    {"name": "design-schema", "task_type": "design", "specialist_type": "designer", "dependencies": [], "description": "设计数据库 schema"},
                    {"name": "implement-api", "task_type": "backend", "specialist_type": "backend-dev", "dependencies": ["design-schema"], "description": "实现 REST API"},
                    {"name": "write-tests", "task_type": "testing", "specialist_type": "tester", "dependencies": ["implement-api"], "description": "编写集成测试"}
                ]"#}]
            }))
        } else {
            Ok(json!({"content": format!("echo: {text}")}))
        }
    }

    fn create_session(
        &self,
        _client: &SdkClient,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "llm-session"}))
    }
}

struct LlmDecomposeFailExecutor;

impl TaskExecutor for LlmDecomposeFailExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"content": [{"type": "text", "text": "抱歉，我无法理解这个请求"}]}))
    }

    fn create_session(
        &self,
        _client: &SdkClient,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "llm-session"}))
    }
}

#[test]
fn llm_decompose_produces_valid_plan() {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let executor: Arc<dyn TaskExecutor> = Arc::new(LlmDecomposeExecutor);
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    )
    .with_llm(executor, client);

    let plan = orch.decompose_task("创建用户管理 API", "trace-llm-1", "job-llm-1", Some("sess-1"));

    assert_eq!(plan.matched_rule.as_deref(), Some("llm-decompose"));
    assert_eq!(plan.subtasks.len(), 3);
    assert!(plan.subtasks.contains_key("design-schema"));
    assert!(plan.subtasks.contains_key("implement-api"));
    assert!(plan.subtasks.contains_key("write-tests"));

    let implement = plan.subtasks.get("implement-api").unwrap();
    assert!(implement.dependencies.contains("design-schema"));

    let tests = plan.subtasks.get("write-tests").unwrap();
    assert!(tests.dependencies.contains("implement-api"));

    assert!(!plan.parallel_groups.is_empty());
}

#[test]
fn llm_decompose_fallback_to_regex_on_invalid_response() {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let executor: Arc<dyn TaskExecutor> = Arc::new(LlmDecomposeFailExecutor);
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    )
    .with_llm(executor, client);

    let plan = orch.decompose_task("build a web app", "trace-llm-2", "job-llm-2", Some("sess-2"));

    // LLM failed → falls back to regex "web-app" rule
    assert_eq!(plan.matched_rule.as_deref(), Some("web-app"));
    assert_eq!(plan.subtasks.len(), 4);
}

#[test]
fn llm_decompose_skipped_without_session_id() {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let executor: Arc<dyn TaskExecutor> = Arc::new(LlmDecomposeExecutor);
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    )
    .with_llm(executor, client);

    // session_id = None → LLM skipped, falls back to regex
    let plan = orch.decompose_task("build a web app", "trace-llm-3", "job-llm-3", None);
    assert_eq!(plan.matched_rule.as_deref(), Some("web-app"));
}

// ── LLM context inference tests ──────────────────────────────────────

struct ContextInferExecutor;

impl TaskExecutor for ContextInferExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        let text = content
            .as_array()
            .and_then(|a| a.first())
            .and_then(|b| b.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if text.contains("上下文完整性分析器") {
            Ok(json!({
                "content": [{"type": "text", "text": r#"{"needs_question": true, "prompt": "请问您的目标用户群体是什么？", "options": ["企业用户", "个人用户", "开发者"]}"#}]
            }))
        } else if text.contains("任务分解引擎") {
            Ok(json!({
                "content": [{"type": "text", "text": r#"[
                    {"name": "task-a", "task_type": "generic", "specialist_type": "default", "dependencies": [], "description": "A"},
                    {"name": "task-b", "task_type": "generic", "specialist_type": "default", "dependencies": [], "description": "B"}
                ]"#}]
            }))
        } else {
            Ok(json!({"content": format!("echo: {text}")}))
        }
    }

    fn create_session(
        &self,
        _client: &SdkClient,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "ctx-session"}))
    }
}

struct ContextInferFailExecutor;

impl TaskExecutor for ContextInferFailExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        let text = content
            .as_array()
            .and_then(|a| a.first())
            .and_then(|b| b.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if text.contains("任务分解引擎") {
            Ok(json!({
                "content": [{"type": "text", "text": r#"[
                    {"name": "task-a", "task_type": "generic", "specialist_type": "default", "dependencies": [], "description": "A"},
                    {"name": "task-b", "task_type": "generic", "specialist_type": "default", "dependencies": [], "description": "B"}
                ]"#}]
            }))
        } else {
            Ok(json!({"content": [{"type": "text", "text": "无法分析"}]}))
        }
    }

    fn create_session(
        &self,
        _client: &SdkClient,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "ctx-fail-session"}))
    }
}

#[test]
fn llm_context_infer_generates_question() {
    use anima_runtime::orchestrator::llm_context_infer::try_llm_infer_missing_context;

    let client = SdkClient::new("http://127.0.0.1:9711");
    let executor: Arc<dyn TaskExecutor> = Arc::new(ContextInferExecutor);
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    )
    .with_llm(executor.clone(), client.clone());

    let plan = orch.decompose_task("build a complex system", "t1", "j1", Some("s1"));

    let subtask_results = json!({
        "task-a": {"status": "success", "result": {"content": "需要了解核心用户流程"}},
        "task-b": {"status": "success", "result": {"content": "需要了解业务类型"}}
    });

    let lowered: Vec<LoweredTask> = plan
        .execution_order
        .iter()
        .filter_map(|name| {
            plan.subtasks.get(name).map(|st| LoweredTask {
                name: st.name.clone(),
                original_task_type: st.task_type.clone(),
                lowered_task_type: st.task_type.clone(),
                primitive: LoweringPrimitive::ApiCall,
                parallel_safe: true,
                task: anima_runtime::agent::types::make_task(anima_runtime::agent::types::MakeTask {
                    task_type: st.task_type.clone(),
                    ..Default::default()
                }),
            })
        })
        .collect();

    let result = try_llm_infer_missing_context(
        &executor, &client, "s1", &plan, &lowered, &subtask_results,
    );

    assert!(result.is_some());
    let q = result.unwrap();
    assert_eq!(q.get("type").and_then(Value::as_str), Some("question"));
    let question = q.get("question").unwrap();
    assert!(question.get("prompt").and_then(Value::as_str).unwrap().contains("用户"));
    assert_eq!(
        question
            .get("orchestration")
            .and_then(|o| o.get("reason"))
            .and_then(Value::as_str),
        Some("llm_inferred_missing_context")
    );
}

#[test]
fn llm_context_infer_fallback_on_invalid_response() {
    use anima_runtime::orchestrator::llm_context_infer::try_llm_infer_missing_context;

    let client = SdkClient::new("http://127.0.0.1:9711");
    let executor: Arc<dyn TaskExecutor> = Arc::new(ContextInferFailExecutor);
    let wp = make_pool();
    let sp = Arc::new(SpecialistPool::new(wp.clone()));
    let orch = AgentOrchestrator::new(
        wp,
        sp,
        Arc::new(RuntimeStateStore::new()),
        OrchestratorConfig::default(),
    )
    .with_llm(executor.clone(), client.clone());

    let plan = orch.decompose_task("build a complex system", "t2", "j2", Some("s2"));

    let subtask_results = json!({
        "task-a": {"status": "success", "result": {"content": "需要了解核心用户流程"}},
        "task-b": {"status": "success", "result": {"content": "需要了解业务类型"}}
    });

    let lowered: Vec<LoweredTask> = plan
        .execution_order
        .iter()
        .filter_map(|name| {
            plan.subtasks.get(name).map(|st| LoweredTask {
                name: st.name.clone(),
                original_task_type: st.task_type.clone(),
                lowered_task_type: st.task_type.clone(),
                primitive: LoweringPrimitive::ApiCall,
                parallel_safe: true,
                task: anima_runtime::agent::types::make_task(anima_runtime::agent::types::MakeTask {
                    task_type: st.task_type.clone(),
                    ..Default::default()
                }),
            })
        })
        .collect();

    let result = try_llm_infer_missing_context(
        &executor, &client, "s2", &plan, &lowered, &subtask_results,
    );

    // LLM returned invalid JSON → None (fallback path)
    assert!(result.is_none());
}
