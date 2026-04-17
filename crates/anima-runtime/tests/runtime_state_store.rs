use anima_runtime::messages::types::MessageRole;
use anima_runtime::runtime::{build_projection, RuntimeDomainEvent, RuntimeStateStore};
use anima_runtime::support::now_ms;
use std::path::PathBuf;
use anima_runtime::tasks::{
    RequirementRecord, RequirementStatus, RunRecord, RunStatus, SuspensionKind, SuspensionRecord,
    SuspensionStatus, TaskKind, TaskRecord, TaskStatus, ToolInvocationRuntimeRecord, TurnRecord,
    TurnStatus,
};
use anima_runtime::transcript::{validate_pairing, ContentBlock, MessageRecord};
use serde_json::json;

#[test]
fn reducer_updates_unified_runtime_snapshot() {
    let store = RuntimeStateStore::new();
    let now = now_ms();

    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-1".into(),
            trace_id: "trace-1".into(),
            job_id: "job-1".into(),
            chat_id: Some("chat-1".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            source: "initial".into(),
            status: TurnStatus::Running,
            transcript_checkpoint: 0,
            requirement_id: Some("req-1".into()),
            suspension_id: Some("susp-1".into()),
            started_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "task-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            parent_task_id: None,
            trace_id: "trace-1".into(),
            job_id: "job-1".into(),
            parent_job_id: None,
            plan_id: Some("plan-1".into()),
            kind: TaskKind::Main,
            name: "main".into(),
            task_type: "api-call".into(),
            description: "main task".into(),
            status: TaskStatus::Running,
            execution_mode: Some("serial".into()),
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({"chat_id": "chat-1"}),
            started_at_ms: Some(now),
            updated_at_ms: now,
            completed_at_ms: None,
            error: None,
        },
    });

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-1".into()),
            question_id: Some("q-1".into()),
            invocation_id: Some("inv-1".into()),
            kind: SuspensionKind::ToolPermission,
            status: SuspensionStatus::Active,
            prompt: Some("allow?".into()),
            options: vec!["allow".into(), "deny".into()],
            raw_payload: json!({"type": "tool_permission"}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now,
            updated_at_ms: now,
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::ToolInvocationUpserted {
        invocation: ToolInvocationRuntimeRecord {
            invocation_id: "inv-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            task_id: Some("task-1".into()),
            tool_name: "bash_exec".into(),
            tool_use_id: Some("toolu_1".into()),
            phase: "permission_requested".into(),
            permission_state: "requested".into(),
            input_preview: Some("rm test.txt".into()),
            result_summary: None,
            error_summary: None,
            started_at_ms: now,
            finished_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::RequirementUpserted {
        requirement: RequirementRecord {
            requirement_id: "req-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            job_id: "job-1".into(),
            original_user_request: "need confirmation".into(),
            attempted_rounds: 1,
            max_rounds: 3,
            last_result_fingerprint: Some("fp-1".into()),
            last_reason: Some("await input".into()),
            status: RequirementStatus::WaitingUserInput,
            created_at_ms: now,
            updated_at_ms: now,
        },
    });

    let snapshot = store.snapshot();
    assert_eq!(snapshot.runs["run-1"].job_id, "job-1");
    assert_eq!(
        snapshot.turns["turn-1"].suspension_id.as_deref(),
        Some("susp-1")
    );
    assert_eq!(snapshot.tasks["task-1"].plan_id.as_deref(), Some("plan-1"));
    assert_eq!(
        snapshot.suspensions["susp-1"].question_id.as_deref(),
        Some("q-1")
    );
    assert_eq!(snapshot.tool_invocations["inv-1"].tool_name, "bash_exec");
    assert_eq!(
        snapshot.requirements["req-1"].status,
        RequirementStatus::WaitingUserInput
    );
    assert_eq!(snapshot.index.run_ids_by_job_id["job-1"], "run-1");
    assert_eq!(snapshot.index.task_ids_by_plan_id["plan-1"], vec!["task-1"]);
    assert_eq!(
        snapshot.index.suspension_ids_by_question_id["q-1"],
        "susp-1"
    );
    assert_eq!(snapshot.index.invocation_ids_by_question_id["q-1"], "inv-1");
}

#[test]
fn question_suspension_lifecycle_transitions_active_resolved_cleared() {
    let store = RuntimeStateStore::new();
    let now = now_ms();

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-question-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-question-1".into()),
            question_id: Some("question-1".into()),
            invocation_id: None,
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Active,
            prompt: Some("请选择继续方式".into()),
            options: vec!["继续".into(), "停止".into()],
            raw_payload: json!({"type": "question"}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now,
            updated_at_ms: now,
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-question-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-question-1".into()),
            question_id: Some("question-1".into()),
            invocation_id: None,
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Resolved,
            prompt: Some("请选择继续方式".into()),
            options: vec!["继续".into(), "停止".into()],
            raw_payload: json!({"type": "question"}),
            resolution_source: Some("user".into()),
            answer_summary: Some("继续执行".into()),
            created_at_ms: now,
            updated_at_ms: now + 1,
            resolved_at_ms: Some(now + 1),
            cleared_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-question-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-question-1".into()),
            question_id: Some("question-1".into()),
            invocation_id: None,
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Cleared,
            prompt: Some("请选择继续方式".into()),
            options: vec!["继续".into(), "停止".into()],
            raw_payload: json!({"type": "question"}),
            resolution_source: Some("user".into()),
            answer_summary: Some("继续执行".into()),
            created_at_ms: now,
            updated_at_ms: now + 2,
            resolved_at_ms: Some(now + 1),
            cleared_at_ms: Some(now + 2),
        },
    });

    let snapshot = store.snapshot();
    let suspension = &snapshot.suspensions["susp-question-1"];
    assert_eq!(suspension.status, SuspensionStatus::Cleared);
    assert_eq!(suspension.answer_summary.as_deref(), Some("继续执行"));
    assert_eq!(suspension.resolution_source.as_deref(), Some("user"));
    assert_eq!(suspension.cleared_at_ms, Some(now + 2));
    assert_eq!(
        snapshot.index.suspension_ids_by_question_id["question-1"],
        "susp-question-1"
    );
}

#[test]
fn tool_permission_suspension_lifecycle_transitions_active_resolved_cleared() {
    let store = RuntimeStateStore::new();
    let now = now_ms();

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-tool-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-tool-1".into()),
            question_id: Some("tool-question-1".into()),
            invocation_id: Some("inv-1".into()),
            kind: SuspensionKind::ToolPermission,
            status: SuspensionStatus::Active,
            prompt: Some("允许 bash_exec 执行吗？".into()),
            options: vec!["allow".into(), "deny".into()],
            raw_payload: json!({"type": "tool_permission", "tool_name": "bash_exec"}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now,
            updated_at_ms: now,
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-tool-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-tool-1".into()),
            question_id: Some("tool-question-1".into()),
            invocation_id: Some("inv-1".into()),
            kind: SuspensionKind::ToolPermission,
            status: SuspensionStatus::Resolved,
            prompt: Some("允许 bash_exec 执行吗？".into()),
            options: vec!["allow".into(), "deny".into()],
            raw_payload: json!({"type": "tool_permission", "tool_name": "bash_exec"}),
            resolution_source: Some("user".into()),
            answer_summary: Some("allow".into()),
            created_at_ms: now,
            updated_at_ms: now + 1,
            resolved_at_ms: Some(now + 1),
            cleared_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-tool-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-tool-1".into()),
            question_id: Some("tool-question-1".into()),
            invocation_id: Some("inv-1".into()),
            kind: SuspensionKind::ToolPermission,
            status: SuspensionStatus::Cleared,
            prompt: Some("允许 bash_exec 执行吗？".into()),
            options: vec!["allow".into(), "deny".into()],
            raw_payload: json!({"type": "tool_permission", "tool_name": "bash_exec"}),
            resolution_source: Some("user".into()),
            answer_summary: Some("allow".into()),
            created_at_ms: now,
            updated_at_ms: now + 2,
            resolved_at_ms: Some(now + 1),
            cleared_at_ms: Some(now + 2),
        },
    });

    let snapshot = store.snapshot();
    let suspension = &snapshot.suspensions["susp-tool-1"];
    assert_eq!(suspension.status, SuspensionStatus::Cleared);
    assert_eq!(suspension.invocation_id.as_deref(), Some("inv-1"));
    assert_eq!(suspension.answer_summary.as_deref(), Some("allow"));
    assert_eq!(suspension.cleared_at_ms, Some(now + 2));
    assert_eq!(
        snapshot.index.suspension_ids_by_question_id["tool-question-1"],
        "susp-tool-1"
    );
    assert_eq!(
        snapshot.index.invocation_ids_by_question_id["tool-question-1"],
        "inv-1"
    );
}

#[test]
fn projection_prefers_job_lifecycle_hint_for_pre_execution_phases() {
    let store = RuntimeStateStore::new();
    let now = now_ms();

    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-hint-1".into(),
            trace_id: "trace-hint-1".into(),
            job_id: "job-hint-1".into(),
            chat_id: Some("chat-hint-1".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::ProjectionHintRecorded {
        run_id: "run-hint-1".into(),
        scope: "job".into(),
        key: "lifecycle".into(),
        value: json!({
            "status": "planning",
            "status_label": "planning",
            "current_step": "会话已就绪，正在构建计划"
        }),
    });

    let projection = build_projection(&store.snapshot());
    let summary = projection
        .job_statuses
        .get("job-hint-1")
        .expect("expected job lifecycle summary");
    assert_eq!(summary.status, "planning");
    assert_eq!(summary.status_label, "planning");
    assert_eq!(summary.current_step, "会话已就绪，正在构建计划");
}

#[test]
fn transcript_pairing_validator_reports_missing_tool_result() {
    let messages = vec![MessageRecord {
        message_id: "m-1".into(),
        run_id: "run-1".into(),
        turn_id: Some("turn-1".into()),
        role: MessageRole::Assistant,
        blocks: vec![ContentBlock::ToolUse {
            id: "toolu_1".into(),
            name: "bash_exec".into(),
            input: json!({"command": "pwd"}),
        }],
        tool_use_id: Some("toolu_1".into()),
        metadata: json!({}),
        filtered: false,
        appended_at_ms: now_ms(),
    }];

    let violations = validate_pairing(&messages);
    assert_eq!(violations.len(), 1);
}

fn temp_runtime_state_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("anima_runtime_state_store_{name}_{}.json", now_ms()));
    path
}

#[test]
fn runtime_state_store_persists_and_restores_snapshot_round_trip() {
    let path = temp_runtime_state_path("round_trip");
    let store = RuntimeStateStore::with_persistence(path.clone());
    let now = now_ms();

    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-persist-1".into(),
            trace_id: "trace-persist-1".into(),
            job_id: "job-persist-1".into(),
            chat_id: Some("chat-persist-1".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::RequirementUpserted {
        requirement: RequirementRecord {
            requirement_id: "req-persist-1".into(),
            run_id: "run-persist-1".into(),
            turn_id: None,
            job_id: "job-persist-1".into(),
            original_user_request: "persist me".into(),
            attempted_rounds: 1,
            max_rounds: 3,
            last_result_fingerprint: None,
            last_reason: Some("waiting".into()),
            status: RequirementStatus::WaitingUserInput,
            created_at_ms: now,
            updated_at_ms: now,
        },
    });

    let restored = RuntimeStateStore::with_persistence(path.clone());
    assert_eq!(restored.snapshot(), store.snapshot());
    assert_eq!(restored.next_sequence(), store.next_sequence());

    let _ = std::fs::remove_file(path);
}

#[test]
fn runtime_state_store_restores_next_sequence_continuity() {
    let path = temp_runtime_state_path("sequence");
    let store = RuntimeStateStore::with_persistence(path.clone());
    let now = now_ms();

    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-seq-1".into(),
            trace_id: "trace-seq-1".into(),
            job_id: "job-seq-1".into(),
            chat_id: None,
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        },
    });

    let restored = RuntimeStateStore::with_persistence(path.clone());
    let sequence = restored.append(RuntimeDomainEvent::ProjectionHintRecorded {
        run_id: "run-seq-1".into(),
        scope: "job".into(),
        key: "status".into(),
        value: json!("running"),
    });
    assert_eq!(sequence, 2);
    assert_eq!(restored.next_sequence(), 2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn runtime_state_store_missing_or_corrupt_file_falls_back_to_empty() {
    let missing_path = temp_runtime_state_path("missing");
    let missing = RuntimeStateStore::with_persistence(missing_path.clone());
    assert!(missing.snapshot().runs.is_empty());
    assert_eq!(missing.next_sequence(), 0);

    let corrupt_path = temp_runtime_state_path("corrupt");
    std::fs::write(&corrupt_path, "{not-json").unwrap();
    let corrupt = RuntimeStateStore::with_persistence(corrupt_path.clone());
    assert!(corrupt.snapshot().runs.is_empty());
    assert_eq!(corrupt.next_sequence(), 0);

    let _ = std::fs::remove_file(missing_path);
    let _ = std::fs::remove_file(corrupt_path);
}
