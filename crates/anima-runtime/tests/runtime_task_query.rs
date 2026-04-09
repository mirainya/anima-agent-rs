use anima_runtime::runtime::{RuntimeDomainEvent, RuntimeStateStore};
use anima_runtime::support::now_ms;
use anima_runtime::tasks::{
    active_requirement, active_suspension, active_turn, invocation_by_question_id,
    latest_tool_invocation, plan_task, run_by_job_id, subtasks_for_plan, suspension_by_question_id,
    tasks_for_job, tasks_for_plan, RequirementRecord, RequirementStatus, RunRecord, RunStatus,
    SuspensionKind, SuspensionRecord, SuspensionStatus, TaskKind, TaskRecord, TaskStatus,
    ToolInvocationRuntimeRecord, TurnRecord, TurnStatus,
};
use serde_json::json;

#[test]
fn query_helpers_read_registry_backed_runtime_snapshot() {
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
            task_id: "task-plan-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            parent_task_id: None,
            trace_id: "trace-1".into(),
            job_id: "job-1".into(),
            parent_job_id: None,
            plan_id: Some("plan-1".into()),
            kind: TaskKind::Plan,
            name: "plan".into(),
            task_type: "web-app".into(),
            description: "top level plan".into(),
            status: TaskStatus::Running,
            execution_mode: Some("serial".into()),
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({"matched_rule": "web-app"}),
            started_at_ms: Some(now),
            updated_at_ms: now,
            completed_at_ms: None,
            error: None,
        },
    });

    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "task-sub-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            parent_task_id: Some("task-plan-1".into()),
            trace_id: "trace-1".into(),
            job_id: "sub-job-1".into(),
            parent_job_id: Some("job-1".into()),
            plan_id: Some("plan-1".into()),
            kind: TaskKind::Subtask,
            name: "api design".into(),
            task_type: "api-design".into(),
            description: "design api".into(),
            status: TaskStatus::Running,
            execution_mode: Some("serial".into()),
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({"parallel_group_index": 0}),
            started_at_ms: Some(now + 1),
            updated_at_ms: now + 1,
            completed_at_ms: None,
            error: None,
        },
    });

    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            task_id: Some("task-sub-1".into()),
            question_id: Some("question-1".into()),
            invocation_id: Some("inv-1".into()),
            kind: SuspensionKind::ToolPermission,
            status: SuspensionStatus::Active,
            prompt: Some("allow bash?".into()),
            options: vec!["allow".into(), "deny".into()],
            raw_payload: json!({"type": "tool_permission"}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now + 2,
            updated_at_ms: now + 2,
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::ToolInvocationUpserted {
        invocation: ToolInvocationRuntimeRecord {
            invocation_id: "inv-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            task_id: Some("task-sub-1".into()),
            tool_name: "bash_exec".into(),
            tool_use_id: Some("toolu_1".into()),
            phase: "permission_requested".into(),
            permission_state: "requested".into(),
            input_preview: Some("pwd".into()),
            result_summary: None,
            error_summary: None,
            started_at_ms: now + 2,
            finished_at_ms: None,
        },
    });

    store.append(RuntimeDomainEvent::RequirementUpserted {
        requirement: RequirementRecord {
            requirement_id: "req-1".into(),
            run_id: "run-1".into(),
            turn_id: Some("turn-1".into()),
            job_id: "job-1".into(),
            original_user_request: "build web app".into(),
            attempted_rounds: 1,
            max_rounds: 3,
            last_result_fingerprint: Some("fp-1".into()),
            last_reason: Some("awaiting permission".into()),
            status: RequirementStatus::WaitingUserInput,
            created_at_ms: now,
            updated_at_ms: now + 2,
        },
    });

    let snapshot = store.snapshot();

    assert_eq!(
        run_by_job_id(&snapshot, "job-1").map(|run| run.run_id.as_str()),
        Some("run-1")
    );
    assert_eq!(
        active_turn(&snapshot, "run-1").map(|turn| turn.turn_id.as_str()),
        Some("turn-1")
    );
    assert_eq!(tasks_for_job(&snapshot, "job-1").len(), 2);
    assert_eq!(tasks_for_plan(&snapshot, "plan-1").len(), 2);
    assert_eq!(
        plan_task(&snapshot, "plan-1").map(|task| task.task_id.as_str()),
        Some("task-plan-1")
    );
    assert_eq!(subtasks_for_plan(&snapshot, "plan-1").len(), 1);
    assert_eq!(
        active_suspension(&snapshot, "run-1").map(|record| record.suspension_id.as_str()),
        Some("susp-1")
    );
    assert_eq!(
        suspension_by_question_id(&snapshot, "question-1")
            .map(|record| record.suspension_id.as_str()),
        Some("susp-1")
    );
    assert_eq!(
        active_requirement(&snapshot, "run-1").map(|record| record.requirement_id.as_str()),
        Some("req-1")
    );
    assert_eq!(
        latest_tool_invocation(&snapshot, "run-1").map(|record| record.invocation_id.as_str()),
        Some("inv-1")
    );
    assert_eq!(
        invocation_by_question_id(&snapshot, "question-1")
            .map(|record| record.invocation_id.as_str()),
        Some("inv-1")
    );
}
