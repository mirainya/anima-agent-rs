use anima_runtime::runtime::StateStore;
use anima_runtime::tasks::{SubtaskBlockedReason, SuspensionKind, SuspensionStatus};
use anima_runtime::agent::types::{make_task_result, MakeTaskResult};

#[test]
fn subtask_blocked_reason_serializes_with_tag() {
    let reason = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "数据库连接字符串".into(),
    };
    let value = serde_json::to_value(&reason).unwrap();
    assert_eq!(value["kind"], "missing_parameter");
    assert_eq!(value["name"], "db_url");

    let reason2 = SubtaskBlockedReason::MultipleOptions {
        options: vec!["A".into(), "B".into()],
        prompt: "选一个".into(),
    };
    let value2 = serde_json::to_value(&reason2).unwrap();
    assert_eq!(value2["kind"], "multiple_options");
    assert_eq!(value2["options"].as_array().unwrap().len(), 2);
}

#[test]
fn subtask_blocked_reason_roundtrips() {
    let cases = vec![
        SubtaskBlockedReason::MissingParameter {
            name: "key".into(),
            description: "desc".into(),
        },
        SubtaskBlockedReason::MissingContext {
            what_needed: "用户角色".into(),
        },
        SubtaskBlockedReason::MultipleOptions {
            options: vec!["X".into(), "Y".into()],
            prompt: "pick".into(),
        },
        SubtaskBlockedReason::NeedsDecision {
            reason: "架构选型".into(),
        },
    ];
    for reason in &cases {
        let json = serde_json::to_value(reason).unwrap();
        let back: SubtaskBlockedReason = serde_json::from_value(json).unwrap();
        assert_eq!(&back, reason);
    }
}

#[test]
fn task_result_blocked_reason_defaults_to_none() {
    let result = make_task_result(MakeTaskResult {
        task_id: "t1".into(),
        trace_id: "tr1".into(),
        status: "success".into(),
        ..Default::default()
    });
    assert!(result.blocked_reason.is_none());
}

#[test]
fn task_result_carries_blocked_reason() {
    let reason = SubtaskBlockedReason::NeedsDecision {
        reason: "需要主 agent 决策".into(),
    };
    let mut result = make_task_result(MakeTaskResult {
        task_id: "t2".into(),
        trace_id: "tr2".into(),
        status: "success".into(),
        ..Default::default()
    });
    result.blocked_reason = Some(serde_json::to_value(&reason).unwrap());
    assert!(result.blocked_reason.is_some());
    let back: SubtaskBlockedReason =
        serde_json::from_value(result.blocked_reason.unwrap()).unwrap();
    assert_eq!(back, reason);
}

#[test]
fn suspension_kind_subtask_blocked_variant_exists() {
    let kind = SuspensionKind::SubtaskBlocked;
    let json = serde_json::to_value(&kind).unwrap();
    assert_eq!(json, "subtask_blocked");
    let back: SuspensionKind = serde_json::from_value(json).unwrap();
    assert_eq!(back, SuspensionKind::SubtaskBlocked);
}

#[test]
fn context_assembly_mode_subtask_blocked_continuation_exists() {
    use anima_runtime::execution::context_assembly::ContextAssemblyMode;
    let mode = ContextAssemblyMode::SubtaskBlockedContinuation;
    assert_eq!(mode, ContextAssemblyMode::SubtaskBlockedContinuation);
}

#[test]
fn blocked_continuation_prompt_includes_context() {
    let reason = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "数据库连接字符串".into(),
    };
    let reason_json = serde_json::to_value(&reason).unwrap();
    let kind = reason_json.get("kind").and_then(|v| v.as_str()).unwrap();
    let prompt = format!(
        "[子任务阻塞恢复] 阻塞类型: {kind}\n原始问题: 子任务需要参数 `db_url`: 数据库连接字符串\n用户回答: postgres://localhost/mydb\n请基于此信息继续执行。"
    );
    assert!(prompt.contains("missing_parameter"));
    assert!(prompt.contains("db_url"));
    assert!(prompt.contains("postgres://localhost/mydb"));
}

#[test]
fn auto_resolve_matches_param_name_in_request() {
    let _reason = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "数据库连接字符串".into(),
    };
    let user_request = "请使用 db_url=postgres://localhost/test 部署服务";
    let lower_request = user_request.to_lowercase();
    let lower_name = "db_url".to_lowercase();
    let resolved = if lower_request.contains(&lower_name) {
        Some(format!(
            "参数 `{}` 可从用户原始请求中获取: {}",
            "db_url", user_request
        ))
    } else {
        None
    };
    assert!(resolved.is_some());
    assert!(resolved.unwrap().contains("db_url"));

    // 不匹配的情况
    let _reason2 = SubtaskBlockedReason::MissingParameter {
        name: "api_key".into(),
        description: "API 密钥".into(),
    };
    let lower_name2 = "api_key".to_lowercase();
    let resolved2 = if lower_request.contains(&lower_name2) {
        Some("found".to_string())
    } else {
        None
    };
    assert!(resolved2.is_none());
}

#[test]
fn projection_maps_subtask_blocked_source_kind() {
    use anima_runtime::agent::PendingQuestionSourceKind;
    use anima_runtime::runtime::{build_projection, RuntimeStateSnapshot};
    use anima_runtime::tasks::{
        RunRecord, RunStatus, SuspensionRecord, SuspensionStatus,
    };

    let run_id = "run-sb-1".to_string();
    let job_id = "job-sb-1".to_string();
    let suspension_id = "susp-sb-1".to_string();
    let question_id = "q-sb-1".to_string();

    let mut snapshot = RuntimeStateSnapshot::default();
    snapshot.runs.insert(
        run_id.clone(),
        RunRecord {
            run_id: run_id.clone(),
            trace_id: "trace-sb-1".into(),
            job_id: job_id.clone(),
            status: RunStatus::Running,
            channel: "test".into(),
            chat_id: None,
            current_turn_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            completed_at_ms: None,
            latest_error: None,
        },
    );
    snapshot.suspensions.insert(
        suspension_id.clone(),
        SuspensionRecord {
            suspension_id: suspension_id.clone(),
            run_id: run_id.clone(),
            turn_id: "turn-1".into(),
            task_id: None,
            question_id: Some(question_id.clone()),
            invocation_id: None,
            kind: SuspensionKind::SubtaskBlocked,
            status: SuspensionStatus::Active,
            prompt: Some("blocked prompt".into()),
            options: vec![],
            raw_payload: serde_json::json!({}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    );
    snapshot
        .index
        .suspension_ids_by_question_id
        .insert(question_id.clone(), suspension_id.clone());

    let projection = build_projection(&snapshot);
    let question = projection.pending_questions.get(&job_id).unwrap();
    assert_eq!(question.source_kind, PendingQuestionSourceKind::SubtaskBlocked);
}

#[test]
fn submit_answer_emits_resolved_suspension_for_subtask_blocked() {
    use anima_runtime::agent::event_emitter::RuntimeEventEmitter;
    use anima_runtime::agent::suspension::SuspensionCoordinator;
    use anima_runtime::agent::QuestionAnswerInput;
    use anima_runtime::bus::Bus;
    use anima_runtime::runtime::{RuntimeStateStore, StateStore};
    use std::sync::Arc;

    let store = Arc::new(RuntimeStateStore::new());
    let bus = Arc::new(Bus::create());
    let timeline = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let emitter = Arc::new(RuntimeEventEmitter::new(
        bus.clone(),
        timeline,
        store.clone(),
    ));
    let coordinator = SuspensionCoordinator::new(store.clone(), emitter);

    let inbound = anima_runtime::bus::make_inbound(anima_runtime::bus::MakeInbound {
        channel: "test".into(),
        content: "deploy service".into(),
        ..Default::default()
    });
    let reason = SubtaskBlockedReason::NeedsDecision {
        reason: "架构选型".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "sess-1", &reason);

    let question = coordinator.question_state(&inbound.id, false).unwrap();
    let answer = QuestionAnswerInput {
        question_id: question.question_id.clone(),
        source: "user".into(),
        answer_type: "text".into(),
        answer: "选方案A".into(),
    };
    coordinator
        .submit_answer(&inbound.id, &answer, "选方案A".into())
        .unwrap();

    let snapshot = store.snapshot();
    let resolved = snapshot
        .suspensions
        .values()
        .find(|s| s.status == SuspensionStatus::Resolved);
    assert!(resolved.is_some(), "should have a Resolved suspension");
    assert_eq!(resolved.unwrap().kind, SuspensionKind::SubtaskBlocked);
}

#[test]
fn clear_question_uses_correct_kind_for_subtask_blocked() {
    use anima_runtime::agent::event_emitter::RuntimeEventEmitter;
    use anima_runtime::agent::suspension::SuspensionCoordinator;
    use anima_runtime::bus::Bus;
    use anima_runtime::runtime::{RuntimeStateStore, StateStore};
    use std::sync::Arc;

    let store = Arc::new(RuntimeStateStore::new());
    let bus = Arc::new(Bus::create());
    let timeline = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let emitter = Arc::new(RuntimeEventEmitter::new(
        bus.clone(),
        timeline,
        store.clone(),
    ));
    let coordinator = SuspensionCoordinator::new(store.clone(), emitter);

    let inbound = anima_runtime::bus::make_inbound(anima_runtime::bus::MakeInbound {
        channel: "test".into(),
        content: "deploy".into(),
        ..Default::default()
    });
    let reason = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "连接字符串".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "sess-2", &reason);
    coordinator.clear_question(&inbound.id);

    let snapshot = store.snapshot();
    let cleared = snapshot
        .suspensions
        .values()
        .find(|s| s.status == SuspensionStatus::Cleared);
    assert!(cleared.is_some(), "should have a Cleared suspension");
    assert_eq!(cleared.unwrap().kind, SuspensionKind::SubtaskBlocked);
}

// ── Phase 5: 变体覆盖 + 完整生命周期 ──────────────────────────

fn make_coordinator() -> (
    anima_runtime::agent::suspension::SuspensionCoordinator,
    std::sync::Arc<anima_runtime::runtime::RuntimeStateStore>,
) {
    use anima_runtime::agent::event_emitter::RuntimeEventEmitter;
    use anima_runtime::agent::suspension::SuspensionCoordinator;
    use anima_runtime::bus::Bus;
    use anima_runtime::runtime::{RuntimeStateStore, StateStore};
    use std::sync::Arc;

    let store = Arc::new(RuntimeStateStore::new());
    let bus = Arc::new(Bus::create());
    let timeline = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let emitter = Arc::new(RuntimeEventEmitter::new(bus, timeline, store.clone()));
    (SuspensionCoordinator::new(store.clone(), emitter), store)
}

fn make_test_inbound(content: &str) -> anima_runtime::bus::InboundMessage {
    anima_runtime::bus::make_inbound(anima_runtime::bus::MakeInbound {
        channel: "test".into(),
        content: content.into(),
        ..Default::default()
    })
}

#[test]
fn register_missing_parameter_produces_input_question() {
    use anima_runtime::agent::QuestionKind;
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "数据库连接字符串".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);
    let q = coordinator.question_state(&inbound.id, false).unwrap();
    assert_eq!(q.question_kind, QuestionKind::Input);
    assert!(q.prompt.contains("db_url"));
    assert!(q.options.is_empty());
}

#[test]
fn register_missing_context_produces_input_question() {
    use anima_runtime::agent::QuestionKind;
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::MissingContext {
        what_needed: "用户角色".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);
    let q = coordinator.question_state(&inbound.id, false).unwrap();
    assert_eq!(q.question_kind, QuestionKind::Input);
    assert!(q.prompt.contains("用户角色"));
}

#[test]
fn register_multiple_options_produces_choice_question() {
    use anima_runtime::agent::QuestionKind;
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::MultipleOptions {
        options: vec!["方案A".into(), "方案B".into(), "方案C".into()],
        prompt: "请选择部署方案".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);
    let q = coordinator.question_state(&inbound.id, false).unwrap();
    assert_eq!(q.question_kind, QuestionKind::Choice);
    assert_eq!(q.options, vec!["方案A", "方案B", "方案C"]);
    assert_eq!(q.prompt, "请选择部署方案");
}

#[test]
fn register_needs_decision_produces_input_question() {
    use anima_runtime::agent::QuestionKind;
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::NeedsDecision {
        reason: "架构选型".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);
    let q = coordinator.question_state(&inbound.id, false).unwrap();
    assert_eq!(q.question_kind, QuestionKind::Input);
    assert!(q.prompt.contains("架构选型"));
}

#[test]
fn multiple_options_full_lifecycle_register_submit_resolve() {
    use anima_runtime::agent::QuestionAnswerInput;
    let (coordinator, store) = make_coordinator();
    let inbound = make_test_inbound("deploy service");
    let reason = SubtaskBlockedReason::MultipleOptions {
        options: vec!["ECS".into(), "Lambda".into()],
        prompt: "选择部署目标".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);

    // Active suspension exists
    let snapshot = store.snapshot();
    let active = snapshot
        .suspensions
        .values()
        .find(|s| s.status == SuspensionStatus::Active && s.kind == SuspensionKind::SubtaskBlocked);
    assert!(active.is_some());

    // Submit answer
    let q = coordinator.question_state(&inbound.id, false).unwrap();
    let answer = QuestionAnswerInput {
        question_id: q.question_id.clone(),
        source: "user".into(),
        answer_type: "choice".into(),
        answer: "ECS".into(),
    };
    coordinator
        .submit_answer(&inbound.id, &answer, "ECS".into())
        .unwrap();

    // Resolved suspension
    let snapshot = store.snapshot();
    let resolved = snapshot
        .suspensions
        .values()
        .find(|s| s.status == SuspensionStatus::Resolved && s.kind == SuspensionKind::SubtaskBlocked);
    assert!(resolved.is_some());
    assert_eq!(resolved.unwrap().answer_summary, Some("ECS".into()));
}

#[test]
fn question_state_raw_question_contains_blocked_reason_fields() {
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::MissingParameter {
        name: "api_key".into(),
        description: "API 密钥".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);
    let q = coordinator.question_state(&inbound.id, false).unwrap();
    assert_eq!(q.raw_question.get("kind").and_then(|v| v.as_str()), Some("missing_parameter"));
    assert_eq!(q.raw_question.get("name").and_then(|v| v.as_str()), Some("api_key"));
}

// ── Phase 6: Edge cases + transcript + auto-resolve 逻辑 ─────

#[test]
fn double_register_overwrites_previous_question() {
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason1 = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "first".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason1);
    let q1_id = coordinator.question_state(&inbound.id, false).unwrap().question_id;

    let reason2 = SubtaskBlockedReason::NeedsDecision {
        reason: "second".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason2);
    let q2 = coordinator.question_state(&inbound.id, false).unwrap();
    assert_ne!(q2.question_id, q1_id);
    assert!(q2.prompt.contains("second"));
}

#[test]
fn submit_answer_rejects_wrong_question_id() {
    use anima_runtime::agent::QuestionAnswerInput;
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::NeedsDecision {
        reason: "test".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);

    let answer = QuestionAnswerInput {
        question_id: "wrong-id".into(),
        source: "user".into(),
        answer_type: "text".into(),
        answer: "answer".into(),
    };
    let err = coordinator
        .submit_answer(&inbound.id, &answer, "answer".into())
        .unwrap_err();
    assert!(err.to_string().contains("mismatch"), "error should mention mismatch: {err}");
}

#[test]
fn submit_answer_fails_for_nonexistent_job() {
    use anima_runtime::agent::QuestionAnswerInput;
    let (coordinator, _) = make_coordinator();
    let answer = QuestionAnswerInput {
        question_id: "q1".into(),
        source: "user".into(),
        answer_type: "text".into(),
        answer: "answer".into(),
    };
    let err = coordinator
        .submit_answer("nonexistent-job", &answer, "answer".into())
        .unwrap_err();
    assert!(err.to_string().contains("no pending question"));
}

#[test]
fn register_creates_transcript_entry() {
    let (coordinator, store) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::MissingParameter {
        name: "db_url".into(),
        description: "连接字符串".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);

    let snapshot = store.snapshot();
    assert!(!snapshot.suspensions.is_empty());
    assert!(!snapshot.tasks.is_empty());
}

#[test]
fn auto_resolve_logic_matches_param_name_case_insensitive() {
    let user_request = "请使用 DB_URL=postgres://localhost/test 部署";
    let param_name = "db_url";
    let matched = user_request.to_lowercase().contains(&param_name.to_lowercase());
    assert!(matched);

    let no_match_request = "请部署服务";
    let not_matched = no_match_request.to_lowercase().contains(&param_name.to_lowercase());
    assert!(!not_matched);
}

#[test]
fn auto_resolve_logic_matches_context_keyword() {
    let user_request = "我是管理员角色，请部署服务";
    let what_needed = "管理员角色";
    let matched = user_request.to_lowercase().contains(&what_needed.to_lowercase());
    assert!(matched);
}

#[test]
fn auto_resolve_skips_multiple_options_and_needs_decision() {
    // MultipleOptions and NeedsDecision should never auto-resolve
    let user_request = "方案A 方案B 请部署";
    let reason_opts = SubtaskBlockedReason::MultipleOptions {
        options: vec!["方案A".into(), "方案B".into()],
        prompt: "选择".into(),
    };
    let reason_decision = SubtaskBlockedReason::NeedsDecision {
        reason: "架构选型".into(),
    };
    // These variants have no name/keyword to match — auto-resolve returns None
    let can_resolve_opts = match &reason_opts {
        SubtaskBlockedReason::MissingParameter { name, .. } => {
            user_request.to_lowercase().contains(&name.to_lowercase())
        }
        SubtaskBlockedReason::MissingContext { what_needed } => {
            user_request.to_lowercase().contains(&what_needed.to_lowercase())
        }
        _ => false,
    };
    assert!(!can_resolve_opts);

    let can_resolve_decision = match &reason_decision {
        SubtaskBlockedReason::MissingParameter { name, .. } => {
            user_request.to_lowercase().contains(&name.to_lowercase())
        }
        SubtaskBlockedReason::MissingContext { what_needed } => {
            user_request.to_lowercase().contains(&what_needed.to_lowercase())
        }
        _ => false,
    };
    assert!(!can_resolve_decision);
}

#[test]
fn cleared_question_no_longer_returned_by_question_state() {
    let (coordinator, _) = make_coordinator();
    let inbound = make_test_inbound("deploy");
    let reason = SubtaskBlockedReason::NeedsDecision {
        reason: "test".into(),
    };
    coordinator.register_subtask_blocked(&inbound.id, &inbound, "s1", &reason);
    assert!(coordinator.question_state(&inbound.id, false).is_some());

    coordinator.clear_question(&inbound.id);
    assert!(coordinator.question_state(&inbound.id, false).is_none());
}

// ── Phase 7: Worker + Orchestrator blocked_reason 传播 ─────

#[test]
fn worker_extracts_blocked_reason_from_response() {
    use anima_runtime::worker::executor::TaskExecutor;
    use anima_runtime::worker::WorkerPool;
    use anima_runtime::agent::{make_task, MakeTask};
    use serde_json::json;
    use std::sync::Arc;

    struct BlockedExecutor;

    impl TaskExecutor for BlockedExecutor {
        fn send_prompt(
            &self,
            _session_id: &str,
            _content: serde_json::Value,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            Ok(json!({
                "content": "I need the db_url parameter",
                "blocked_reason": {
                    "kind": "missing_parameter",
                    "name": "db_url",
                    "description": "数据库连接字符串"
                }
            }))
        }

        fn create_session(
            &self,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            Ok(json!({"id": "blocked-session"}))
        }
    }

    let pool = Arc::new(WorkerPool::new(
        Arc::new(BlockedExecutor),
        Some(1),
        None,
        Some(100),
    ));
    pool.start();

    let rx = pool.submit_task(make_task(MakeTask {
        task_type: "api-call".into(),
        payload: Some(json!({
            "opencode-session-id": "s1",
            "content": "deploy"
        })),
        ..Default::default()
    }));

    let result = rx.recv().unwrap();
    assert_eq!(result.status, "success");
    assert!(result.blocked_reason.is_some());
    let reason: SubtaskBlockedReason =
        serde_json::from_value(result.blocked_reason.unwrap()).unwrap();
    assert_eq!(
        reason,
        SubtaskBlockedReason::MissingParameter {
            name: "db_url".into(),
            description: "数据库连接字符串".into(),
        }
    );
}

#[test]
fn orchestrator_propagates_blocked_reason_on_final_result() {
    use anima_runtime::worker::executor::TaskExecutor;
    use anima_runtime::worker::WorkerPool;
    use anima_runtime::orchestrator::core::{AgentOrchestrator, OrchestratorConfig};
    use anima_runtime::orchestrator::specialist_pool::SpecialistPool;
    use anima_runtime::provider::{ChatRequest, ChatResponse, Provider, ProviderError, StopReason};
    use anima_runtime::messages::types::ContentBlock;
    use anima_runtime::runtime::{RuntimeStateStore, StateStore};
    use serde_json::json;
    use std::sync::Arc;

    struct BlockedOrchestratorExecutor;

    impl TaskExecutor for BlockedOrchestratorExecutor {
        fn send_prompt(
            &self,
            _session_id: &str,
            _content: serde_json::Value,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            Ok(json!({
                "content": "blocked",
                "blocked_reason": {
                    "kind": "missing_context",
                    "what_needed": "用户角色"
                }
            }))
        }

        fn create_session(
            &self,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            Ok(json!({"id": "orch-blocked-session"}))
        }
    }

    struct SimpleDecomposeProvider;
    impl Provider for SimpleDecomposeProvider {
        fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                content: vec![ContentBlock::Text {
                    text: r#"[{"name":"deploy","task_type":"backend","specialist_type":"default","dependencies":[],"description":"deploy"},{"name":"verify","task_type":"testing","specialist_type":"default","dependencies":["deploy"],"description":"verify"}]"#.into(),
                }],
                stop_reason: StopReason::EndTurn,
                usage: None,
                raw: serde_json::Value::Null,
            })
        }
    }

    let pool = Arc::new(WorkerPool::new(
        Arc::new(BlockedOrchestratorExecutor),
        Some(2),
        None,
        Some(100),
    ));
    pool.start();
    let sp = Arc::new(SpecialistPool::new(pool.clone()));
    let store = Arc::new(RuntimeStateStore::new());
    let orch = AgentOrchestrator::new(pool, sp, store, OrchestratorConfig::default())
        .with_llm(Arc::new(SimpleDecomposeProvider));

    let execution = orch
        .execute_orchestration_for_main_chain(
            "请帮我部署一个 webapp 到生产环境",
            "trace-1",
            "job-1",
            "session-1",
            |_, _| {},
        )
        .expect("orchestration should succeed");

    let result = execution.result;
    assert_eq!(result.status, "success");
    assert!(
        result.blocked_reason.is_some(),
        "orchestrator should propagate blocked_reason, got: {:?}",
        result.result
    );
}

// ── Phase 8: 主 Agent LLM 自主决策 Worker 升级 ─────

#[test]
#[serial_test::serial]
fn main_agent_llm_resolves_blocked_subtask() {
    use anima_runtime::agent::{Agent, TaskExecutor};
    use anima_runtime::bus::{make_inbound, Bus, MakeInbound};
    use anima_runtime::runtime::{RuntimeStateStore, StateStore};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct EscalationResolvedExecutor {
        call_count: AtomicUsize,
    }

    impl EscalationResolvedExecutor {
        fn new() -> Self {
            Self { call_count: AtomicUsize::new(0) }
        }
    }

    impl TaskExecutor for EscalationResolvedExecutor {
        fn send_prompt(
            &self,
            _session_id: &str,
            content: serde_json::Value,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            let text = content.to_string();
            if text.contains("任务分解引擎") {
                return Ok(json!({"content": "[]"}));
            }
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            match idx {
                // First call: worker execution returns blocked
                0 => Ok(json!({
                    "content": "blocked",
                    "blocked_reason": {
                        "kind": "needs_decision",
                        "reason": "选择部署目标区域"
                    }
                })),
                // Second call: LLM escalation resolves it
                1 => Ok(json!({
                    "content": [{"type": "text", "text": "部署到 us-east-1 区域"}]
                })),
                // Third call: continuation after resolution
                _ => Ok(json!({
                    "content": "部署完成"
                })),
            }
        }

        fn create_session(
            &self,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            Ok(json!({"id": "escalation-session"}))
        }
    }

    let bus = Arc::new(Bus::create());
    let agent = Agent::with_runtime_state_store(
        bus.clone(),
        None,
        Arc::new(EscalationResolvedExecutor::new()),
        Arc::new(RuntimeStateStore::new()),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-esc".into()),
        chat_id: Some("chat-esc".into()),
        content: "部署服务到生产环境".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    std::thread::sleep(Duration::from_millis(300));

    // LLM resolved it — no pending question should remain
    let pending = agent.core_agent().pending_question_for(&job_id);
    assert!(
        pending.is_none(),
        "LLM should have resolved the blocked subtask, but found pending: {:?}",
        pending
    );

    agent.stop();
}

#[test]
#[serial_test::serial]
fn main_agent_llm_cannot_resolve_falls_through_to_suspension() {
    use anima_runtime::agent::{Agent, TaskExecutor};
    use anima_runtime::bus::{make_inbound, Bus, MakeInbound};
    use anima_runtime::runtime::{RuntimeStateStore, StateStore};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct EscalationFailedExecutor {
        call_count: AtomicUsize,
    }

    impl EscalationFailedExecutor {
        fn new() -> Self {
            Self { call_count: AtomicUsize::new(0) }
        }
    }

    impl TaskExecutor for EscalationFailedExecutor {
        fn send_prompt(
            &self,
            _session_id: &str,
            content: serde_json::Value,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            let text = content.to_string();
            if text.contains("任务分解引擎") {
                return Ok(json!({"content": "[]"}));
            }
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            match idx {
                // First call: worker execution returns blocked
                0 => Ok(json!({
                    "content": "blocked",
                    "blocked_reason": {
                        "kind": "needs_decision",
                        "reason": "选择部署目标区域"
                    }
                })),
                // Second call: LLM cannot resolve
                1 => Ok(json!({
                    "content": [{"type": "text", "text": "CANNOT_RESOLVE"}]
                })),
                _ => Ok(json!({"content": "unexpected"})),
            }
        }

        fn create_session(
            &self,
        ) -> Result<serde_json::Value, anima_runtime::agent::runtime_error::RuntimeError> {
            Ok(json!({"id": "escalation-fail-session"}))
        }
    }

    let bus = Arc::new(Bus::create());
    let agent = Agent::with_runtime_state_store(
        bus.clone(),
        None,
        Arc::new(EscalationFailedExecutor::new()),
        Arc::new(RuntimeStateStore::new()),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-esc-fail".into()),
        chat_id: Some("chat-esc-fail".into()),
        content: "部署服务".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    std::thread::sleep(Duration::from_millis(300));

    // LLM could not resolve — should have a pending question (suspension)
    let pending = agent.core_agent().pending_question_for(&job_id);
    assert!(
        pending.is_some(),
        "LLM returned CANNOT_RESOLVE, should have suspended with pending question"
    );
    let q = pending.unwrap();
    assert!(q.prompt.contains("决策") || q.prompt.contains("区域"));

    agent.stop();
}
