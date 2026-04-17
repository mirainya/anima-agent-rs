use anima_runtime::agent::{
    ExecutionStageDurations, ExecutionSummary, RuntimeFailureSnapshot, RuntimeTimelineEvent,
    TaskExecutor, WorkerMetrics, WorkerStatus,
};
use anima_runtime::bootstrap::RuntimeBootstrapBuilder;
use anima_runtime::bus::{make_inbound, MakeInbound};
use anima_runtime::channel::Channel;
use anima_runtime::runtime::{RuntimeDomainEvent, RuntimeStateStore};
use anima_runtime::tasks::{
    RequirementRecord, RequirementStatus, RunRecord, RunStatus, SuspensionKind, SuspensionRecord,
    SuspensionStatus, TaskKind, TaskRecord, TaskStatus, ToolInvocationRuntimeRecord, TurnRecord,
    TurnStatus,
};
use anima_sdk::facade::Client as SdkClient;
use anima_web::jobs::JobKind;
use anima_web::{routes, web_channel, AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::util::ServiceExt;

fn runtime_event(
    message_id: &str,
    chat_id: Option<&str>,
    event: &str,
    recorded_at_ms: u64,
    payload: Value,
) -> RuntimeTimelineEvent {
    RuntimeTimelineEvent {
        event: event.into(),
        trace_id: message_id.into(),
        message_id: message_id.into(),
        channel: "web".into(),
        chat_id: chat_id.map(|value| value.to_string()),
        sender_id: "web-user".into(),
        recorded_at_ms,
        payload,
    }
}

fn build_state_with_runtime(
    runtime: anima_runtime::bootstrap::RuntimeBootstrap,
    bus: Arc<anima_runtime::bus::Bus>,
    web_channel: Arc<anima_web::web_channel::WebChannel>,
) -> Arc<AppState> {
    Arc::new(AppState {
        runtime: Mutex::new(runtime),
        bus,
        web_channel,
        jobs: Mutex::new(anima_web::jobs::JobStore::default()),
    })
}

fn build_state_with_store(
    runtime: anima_runtime::bootstrap::RuntimeBootstrap,
    bus: Arc<anima_runtime::bus::Bus>,
    web_channel: Arc<anima_web::web_channel::WebChannel>,
    store: anima_web::jobs::JobStore,
) -> Arc<AppState> {
    Arc::new(AppState {
        runtime: Mutex::new(runtime),
        bus,
        web_channel,
        jobs: Mutex::new(store),
    })
}

#[test]
fn build_job_views_show_accepted_job_before_runtime_events() {
    let mut store = anima_web::jobs::JobStore::default();
    let accepted_at_ms = anima_runtime::support::now_ms().saturating_sub(100);
    store.register_accepted_job(anima_web::jobs::AcceptedJob {
        job_id: "job-queued".into(),
        trace_id: "job-queued".into(),
        message_id: "job-queued".into(),
        kind: JobKind::Main,
        parent_job_id: None,
        channel: "web".into(),
        chat_id: Some("queued-chat".into()),
        sender_id: "web-user".into(),
        user_content: "queued message".into(),
        accepted_at_ms,
    });

    let jobs = anima_web::jobs::build_job_views(&[], &[], &[], &[], &store);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "job-queued");
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Queued);
    assert_eq!(jobs[0].kind, JobKind::Main);
    assert_eq!(jobs[0].parent_job_id, None);
    assert_eq!(jobs[0].chat_id.as_deref(), Some("queued-chat"));
    assert!(jobs[0].accepted);
}

#[test]
fn build_job_views_mark_subtask_when_parent_job_id_exists_in_event_payload() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-subtask";
    let timeline = vec![runtime_event(
        message_id,
        Some("subtask-chat"),
        "message_received",
        now.saturating_sub(1_000),
        json!({"parent_job_id": "job-main"}),
    )];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].kind, JobKind::Subtask);
    assert_eq!(jobs[0].parent_job_id.as_deref(), Some("job-main"));
}

#[test]
fn build_job_views_detect_stalled_after_planning_stall() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-question";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("question-chat"),
            "message_received",
            now.saturating_sub(40_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "session_ready",
            now.saturating_sub(35_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "plan_built",
            now.saturating_sub(25_000),
            json!({"plan_type": "single"}),
        ),
    ];
    let summaries = vec![ExecutionSummary {
        trace_id: message_id.into(),
        message_id: message_id.into(),
        channel: "web".into(),
        chat_id: Some("question-chat".into()),
        plan_type: "single".into(),
        status: "running".into(),
        cache_hit: false,
        worker_id: None,
        error_code: None,
        error_stage: None,
        task_duration_ms: 0,
        stages: ExecutionStageDurations {
            context_ms: 1,
            session_ms: 1,
            classify_ms: 1,
            execute_ms: 0,
            total_ms: 3,
        },
    }];
    let workers: Vec<WorkerStatus> = vec![WorkerStatus {
        id: "worker-1".into(),
        status: "idle".into(),
        metrics: WorkerMetrics::default(),
        current_task: None,
    }];
    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &summaries,
        &[],
        &workers,
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Stalled);
    assert_eq!(jobs[0].status_label, "stalled");
    assert!(jobs[0].current_step.contains("长时间没有新的运行时进展"));
    assert!(jobs[0].pending_question.is_none());
}

#[test]
fn build_job_views_exposes_pending_question_and_resolution_transitions() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-pending-question";
    let raw_question = json!({
        "id": "question-1",
        "kind": "input",
        "prompt": "请选择继续方式",
        "options": ["继续执行", "取消"]
    });
    let question_payload = json!({
        "question_id": "question-1",
        "question_kind": "input",
        "prompt": "请选择继续方式",
        "options": ["继续执行", "取消"],
        "raw_question": raw_question,
        "decision_mode": "user_required",
        "risk_level": "high",
        "requires_user_confirmation": true,
        "opencode_session_id": "session-q-1"
    });

    let timeline = vec![
        runtime_event(
            message_id,
            Some("question-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "question_asked",
            now.saturating_sub(3_000),
            question_payload.clone(),
        ),
    ];
    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::WaitingUserInput);
    assert_eq!(
        jobs[0]
            .pending_question
            .as_ref()
            .map(|q| q.question_id.as_str()),
        Some("question-1")
    );
    assert_eq!(
        jobs[0].pending_question.as_ref().map(|q| &q.raw_question),
        question_payload.get("raw_question")
    );
    assert!(jobs[0].current_step.contains("等待用户提供所需输入"));

    let submitted_timeline = vec![
        runtime_event(
            message_id,
            Some("question-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "question_asked",
            now.saturating_sub(3_000),
            question_payload.clone(),
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "question_answer_submitted",
            now.saturating_sub(2_000),
            json!({
                "question_id": "question-1",
                "answer_summary": "继续执行",
                "resolution_source": "user"
            }),
        ),
    ];
    let submitted_jobs = anima_web::jobs::build_job_views(
        &submitted_timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(
        submitted_jobs[0].status,
        anima_web::jobs::JobStatus::WaitingUserInput
    );
    assert!(submitted_jobs[0].current_step.contains("已提交回答"));
    assert_eq!(
        submitted_jobs[0]
            .pending_question
            .as_ref()
            .and_then(|q| q.answer_summary.as_deref()),
        Some("继续执行")
    );

    let resolved_timeline = vec![
        runtime_event(
            message_id,
            Some("question-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "question_asked",
            now.saturating_sub(3_000),
            question_payload,
        ),
        runtime_event(
            message_id,
            Some("question-chat"),
            "question_resolved",
            now.saturating_sub(1_000),
            json!({
                "question_id": "question-1"
            }),
        ),
    ];
    let resolved_jobs = anima_web::jobs::build_job_views(
        &resolved_timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert!(resolved_jobs[0].pending_question.is_none());
}

#[test]
fn build_job_views_prefers_unified_runtime_question_and_tool_state() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-job-runtime".into(),
            trace_id: "trace-runtime".into(),
            job_id: "job-runtime".into(),
            chat_id: Some("runtime-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-runtime".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(5_000),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-runtime".into(),
            run_id: "run-job-runtime".into(),
            turn_id: "turn-runtime".into(),
            task_id: Some("task-runtime".into()),
            question_id: Some("tool-question-runtime".into()),
            invocation_id: Some("inv-runtime".into()),
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Active,
            prompt: Some("允许工具 bash_exec 使用当前参数执行吗？".into()),
            options: vec!["allow".into(), "deny".into()],
            raw_payload: json!({
                "type": "tool_permission",
                "tool_name": "bash_exec",
                "tool_use_id": "toolu_runtime",
                "input_preview": "{\"command\":\"pwd\"}"
            }),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now.saturating_sub(1_000),
            updated_at_ms: now.saturating_sub(1_000),
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::ToolInvocationUpserted {
        invocation: ToolInvocationRuntimeRecord {
            invocation_id: "inv-runtime".into(),
            run_id: "run-job-runtime".into(),
            turn_id: Some("turn-runtime".into()),
            task_id: Some("task-runtime".into()),
            tool_name: "bash_exec".into(),
            tool_use_id: Some("toolu_runtime".into()),
            phase: "permission_requested".into(),
            permission_state: "requested".into(),
            input_preview: Some("{\"command\":\"pwd\"}".into()),
            result_summary: None,
            error_summary: None,
            started_at_ms: now.saturating_sub(900),
            finished_at_ms: None,
        },
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "job-runtime");
    assert_eq!(jobs[0].chat_id.as_deref(), Some("runtime-chat"));
    assert_eq!(jobs[0].trace_id, "trace-runtime");
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::WaitingUserInput);
    let pending = jobs[0]
        .pending_question
        .as_ref()
        .expect("expected runtime pending question");
    assert_eq!(pending.question_id, "tool-question-runtime");
    assert_eq!(pending.raw_question["type"], "tool_permission");
    let tool_state = jobs[0]
        .tool_state
        .as_ref()
        .expect("expected runtime tool_state");
    assert_eq!(tool_state.invocation_id.as_deref(), Some("inv-runtime"));
    assert_eq!(tool_state.phase, "permission_requested");
    assert_eq!(tool_state.tool_name.as_deref(), Some("bash_exec"));
    assert_eq!(tool_state.tool_use_id.as_deref(), Some("toolu_runtime"));
    assert_eq!(tool_state.invocation_status, "permission_requested");
    assert_eq!(tool_state.status_text, "等待用户确认工具调用权限");
    assert!(tool_state.awaits_user_confirmation);
}

#[test]
fn build_job_views_exposes_tool_permission_pending_question() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-tool-permission";
    let question_payload = json!({
        "question_id": "tool-question-1",
        "question_kind": "confirm",
        "prompt": "允许工具 'bash_exec' 使用当前参数执行吗？",
        "options": ["allow", "deny"],
        "raw_question": {
            "type": "tool_permission",
            "tool_name": "bash_exec",
            "tool_use_id": "toolu_123",
            "tool_input": {"command": "rm test.txt"},
            "input_preview": "{\"command\":\"rm test.txt\"}"
        },
        "decision_mode": "user_required",
        "risk_level": "high",
        "requires_user_confirmation": true,
        "opencode_session_id": "session-tool-1"
    });

    let timeline = vec![
        runtime_event(
            message_id,
            Some("tool-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "question_asked",
            now.saturating_sub(3_000),
            question_payload.clone(),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "tool_permission_requested",
            now.saturating_sub(2_500),
            json!({
                "question_id": "tool-question-1",
                "tool_use_id": "toolu_123",
                "tool_name": "bash_exec"
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::WaitingUserInput);
    assert_eq!(jobs[0].current_step, "等待用户确认工具调用权限");
    let pending = jobs[0]
        .pending_question
        .as_ref()
        .expect("expected pending tool permission");
    assert_eq!(pending.question_id, "tool-question-1");
    assert_eq!(pending.raw_question["type"], "tool_permission");
    assert_eq!(pending.raw_question["tool_name"], "bash_exec");
    assert_eq!(pending.raw_question["tool_use_id"], "toolu_123");
    assert_eq!(pending.options, vec!["allow", "deny"]);
    let tool_state = jobs[0].tool_state.as_ref().expect("expected tool_state");
    assert_eq!(tool_state.phase, "permission_requested");
    assert_eq!(tool_state.tool_name.as_deref(), Some("bash_exec"));
    assert_eq!(tool_state.tool_use_id.as_deref(), Some("toolu_123"));
    assert_eq!(tool_state.invocation_status, "permission_requested");
    assert_eq!(tool_state.status_text, "等待用户确认工具调用权限");
    assert!(tool_state.awaits_user_confirmation);
}

#[test]
fn build_job_views_derives_tool_execution_state_after_permission_resolution() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-tool-executing";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("tool-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "tool_permission_resolved",
            now.saturating_sub(3_000),
            json!({
                "question_id": "tool-question-1",
                "tool_use_id": "toolu_123",
                "tool_name": "bash_exec",
                "decision": "allow",
                "invocation_id": "invoke-1",
                "tool_invocation": {
                    "invocation_id": "invoke-1",
                    "tool_name": "bash_exec",
                    "tool_use_id": "toolu_123",
                    "phase": "permission_resolved",
                    "permission_state": "allowed"
                }
            }),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "tool_execution_started",
            now.saturating_sub(2_000),
            json!({
                "invocation_id": "invoke-1",
                "tool_name": "bash_exec",
                "tool_use_id": "toolu_123",
                "phase": "executing",
                "permission_state": "allowed",
                "details": {
                    "tool_input": {"command": "ls"}
                }
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].current_step, "正在执行工具：bash_exec");
    let tool_state = jobs[0].tool_state.as_ref().expect("expected tool_state");
    assert_eq!(tool_state.phase, "executing");
    assert_eq!(tool_state.permission_state.as_deref(), Some("allowed"));
    assert_eq!(tool_state.invocation_status, "executing");
    assert_eq!(tool_state.status_text, "正在执行工具：bash_exec");
    assert_eq!(
        tool_state.input_preview.as_deref(),
        Some("{\"command\":\"ls\"}")
    );
}

#[test]
fn build_job_views_derives_tool_result_state_after_completion() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-tool-result";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("tool-chat"),
            "message_received",
            now.saturating_sub(5_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "tool_execution_finished",
            now.saturating_sub(3_000),
            json!({
                "invocation_id": "invoke-2",
                "tool_name": "read_file",
                "tool_use_id": "toolu_456",
                "phase": "completed",
                "permission_state": "allowed",
                "result_summary": "line 1\nline 2"
            }),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "tool_result_recorded",
            now.saturating_sub(2_000),
            json!({
                "invocation_id": "invoke-2",
                "tool_name": "read_file",
                "tool_use_id": "toolu_456",
                "phase": "result_recorded",
                "permission_state": "allowed",
                "result_summary": "line 1\nline 2"
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].current_step, "工具结果已记录，等待后续推进");
    let tool_state = jobs[0].tool_state.as_ref().expect("expected tool_state");
    assert_eq!(tool_state.phase, "result_recorded");
    assert_eq!(tool_state.invocation_status, "result_recorded");
    assert_eq!(tool_state.status_text, "工具结果已记录，等待后续推进");
    assert_eq!(tool_state.result_preview.as_deref(), Some("line 1\nline 2"));
}

#[test]
fn build_job_views_derives_tool_permission_denied_state() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-tool-denied";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("tool-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("tool-chat"),
            "tool_permission_resolved",
            now.saturating_sub(2_000),
            json!({
                "question_id": "tool-question-2",
                "tool_use_id": "toolu_deny",
                "tool_name": "bash_exec",
                "decision": "deny",
                "invocation_id": "invoke-deny",
                "tool_invocation": {
                    "invocation_id": "invoke-deny",
                    "tool_name": "bash_exec",
                    "tool_use_id": "toolu_deny",
                    "phase": "failed",
                    "permission_state": "denied"
                }
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].current_step, "工具权限已拒绝");
    let tool_state = jobs[0].tool_state.as_ref().expect("expected tool_state");
    assert_eq!(tool_state.phase, "permission_resolved");
    assert_eq!(tool_state.permission_state.as_deref(), Some("denied"));
    assert_eq!(tool_state.invocation_status, "permission_denied");
    assert_eq!(tool_state.status_text, "工具权限已拒绝");
}

#[test]
fn build_job_views_keep_detailed_process_events_and_tolerate_missing_fields() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-process-detail";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("process-chat"),
            "worker_task_assigned",
            now.saturating_sub(3_000),
            json!({
                "task_id": "task-assign",
                "task_type": "api-call",
                "task_summary": "主 agent 已将任务派发给 worker",
                "task_preview": "帮我查询今天的状态",
                "opencode_session_id": "session-process"
            }),
        ),
        runtime_event(
            message_id,
            Some("process-chat"),
            "api_call_started",
            now.saturating_sub(2_000),
            json!({
                "task_id": "task-assign",
                "task_type": "api-call",
                "request_preview": "帮我查询今天的状态",
                "opencode_session_id": "session-process"
            }),
        ),
        runtime_event(
            message_id,
            Some("process-chat"),
            "upstream_response_observed",
            now.saturating_sub(1_000),
            json!({
                "worker_id": "worker-1",
                "task_type": "api-call",
                "provider": "opencode",
                "operation": "send_prompt",
                "response_preview": "已经拿到上游回复",
                "raw_result": {"content": "已经拿到上游回复"},
                "opencode_session_id": "session-process"
            }),
        ),
        runtime_event(
            message_id,
            Some("process-chat"),
            "upstream_response_observed",
            now.saturating_sub(500),
            json!({
                "opencode_session_id": "session-process-legacy"
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs.len(), 1);
    let recent_events = &jobs[0].recent_events;
    assert_eq!(recent_events.len(), 4);
    assert_eq!(recent_events[0].event, "worker_task_assigned");
    assert_eq!(recent_events[1].event, "api_call_started");
    assert_eq!(recent_events[2].event, "upstream_response_observed");
    assert_eq!(recent_events[2].payload["worker_id"], "worker-1");
    assert_eq!(recent_events[2].payload["task_type"], "api-call");
    assert_eq!(
        recent_events[2].payload["response_preview"],
        "已经拿到上游回复"
    );
    assert_eq!(
        recent_events[2].payload["raw_result"]["content"],
        "已经拿到上游回复"
    );
    assert_eq!(recent_events[3].event, "upstream_response_observed");
    assert_eq!(
        recent_events[3].payload["opencode_session_id"],
        "session-process-legacy"
    );
    assert!(recent_events[3].payload.get("worker_id").is_none());
}

#[test]
fn build_job_views_prefers_runtime_waiting_user_input_status() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-status-waiting".into(),
            trace_id: "trace-status-waiting".into(),
            job_id: "job-status-waiting".into(),
            chat_id: Some("status-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-status-waiting".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-status-waiting".into(),
            run_id: "run-status-waiting".into(),
            source: "initial".into(),
            status: TurnStatus::Waiting,
            transcript_checkpoint: 0,
            requirement_id: Some("req-status-waiting".into()),
            suspension_id: Some("susp-status-waiting".into()),
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::RequirementUpserted {
        requirement: RequirementRecord {
            requirement_id: "req-status-waiting".into(),
            run_id: "run-status-waiting".into(),
            turn_id: Some("turn-status-waiting".into()),
            job_id: "job-status-waiting".into(),
            original_user_request: "need more info".into(),
            attempted_rounds: 1,
            max_rounds: 3,
            last_result_fingerprint: None,
            last_reason: Some("await input".into()),
            status: RequirementStatus::WaitingUserInput,
            created_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(500),
        },
    });
    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-status-waiting".into(),
            run_id: "run-status-waiting".into(),
            turn_id: "turn-status-waiting".into(),
            task_id: Some("task-status-waiting".into()),
            question_id: Some("question-status-waiting".into()),
            invocation_id: None,
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Active,
            prompt: Some("请补充部署环境".into()),
            options: vec![],
            raw_payload: json!({"type": "question"}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now.saturating_sub(1_000),
            updated_at_ms: now.saturating_sub(500),
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::WaitingUserInput);
    assert_eq!(jobs[0].status_label, "waiting_user_input");
    assert_eq!(jobs[0].current_step, "请补充部署环境");
}

#[test]
fn build_job_views_prefers_runtime_projection_job_status_summary() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-projection-status".into(),
            trace_id: "trace-projection-status".into(),
            job_id: "job-projection-status".into(),
            chat_id: Some("projection-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-projection-status".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-projection-status".into(),
            run_id: "run-projection-status".into(),
            source: "initial".into(),
            status: TurnStatus::Running,
            transcript_checkpoint: 0,
            requirement_id: None,
            suspension_id: None,
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::ToolInvocationUpserted {
        invocation: ToolInvocationRuntimeRecord {
            invocation_id: "inv-projection-status".into(),
            run_id: "run-projection-status".into(),
            turn_id: Some("turn-projection-status".into()),
            task_id: Some("task-projection-status".into()),
            tool_name: "bash_exec".into(),
            tool_use_id: Some("toolu_projection_status".into()),
            phase: "executing".into(),
            permission_state: "allowed".into(),
            input_preview: Some("{\"command\":\"pwd\"}".into()),
            result_summary: None,
            error_summary: None,
            started_at_ms: now.saturating_sub(200),
            finished_at_ms: None,
        },
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].status_label, "executing");
    assert_eq!(jobs[0].current_step, "正在执行工具：bash_exec");
}

#[test]
fn build_job_views_prefers_runtime_execution_summary_hint() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-summary-runtime".into(),
            trace_id: "trace-summary-runtime".into(),
            job_id: "job-summary-runtime".into(),
            chat_id: Some("summary-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-summary-runtime".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::ProjectionHintRecorded {
        run_id: "run-summary-runtime".into(),
        scope: "execution".into(),
        key: "summary".into(),
        value: json!({
            "trace_id": "trace-summary-runtime",
            "message_id": "job-summary-runtime",
            "channel": "web",
            "chat_id": "summary-chat",
            "plan_type": "runtime-plan",
            "status": "running",
            "cache_hit": true,
            "worker_id": "worker-runtime",
            "error_code": null,
            "error_stage": null,
            "task_duration_ms": 42,
            "stages": {
                "context_ms": 1,
                "session_ms": 2,
                "classify_ms": 3,
                "execute_ms": 36,
                "total_ms": 42
            }
        }),
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0]
            .execution_summary
            .as_ref()
            .and_then(|value| value.get("plan_type"))
            .and_then(|value| value.as_str()),
        Some("runtime-plan")
    );
    assert_eq!(
        jobs[0]
            .execution_summary
            .as_ref()
            .and_then(|value| value.get("worker_id"))
            .and_then(|value| value.as_str()),
        Some("worker-runtime")
    );
    assert_eq!(
        jobs[0]
            .execution_summary
            .as_ref()
            .and_then(|value| value.get("cache_hit"))
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn build_job_views_falls_back_to_runtime_run_failure_snapshot() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-status-failed".into(),
            trace_id: "trace-status-failed".into(),
            job_id: "job-status-failed".into(),
            chat_id: Some("status-chat".into()),
            channel: "web".into(),
            status: RunStatus::Failed,
            current_turn_id: Some("turn-status-failed".into()),
            latest_error: Some("tool execution crashed".into()),
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: Some(now.saturating_sub(200)),
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-status-failed".into(),
            run_id: "run-status-failed".into(),
            source: "initial".into(),
            status: TurnStatus::Failed,
            transcript_checkpoint: 0,
            requirement_id: None,
            suspension_id: None,
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: Some(now.saturating_sub(200)),
        },
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Failed);
    assert_eq!(
        jobs[0]
            .failure
            .as_ref()
            .and_then(|value| value.get("error_code"))
            .and_then(|value| value.as_str()),
        Some("runtime_run_failed")
    );
    assert_eq!(
        jobs[0]
            .failure
            .as_ref()
            .and_then(|value| value.get("internal_message"))
            .and_then(|value| value.as_str()),
        Some("tool execution crashed")
    );
}

#[test]
fn build_job_views_prefers_runtime_completed_status() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-status-completed".into(),
            trace_id: "trace-status-completed".into(),
            job_id: "job-status-completed".into(),
            chat_id: Some("status-chat".into()),
            channel: "web".into(),
            status: RunStatus::Completed,
            current_turn_id: Some("turn-status-completed".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: Some(now.saturating_sub(300)),
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-status-completed".into(),
            run_id: "run-status-completed".into(),
            source: "initial".into(),
            status: TurnStatus::Completed,
            transcript_checkpoint: 0,
            requirement_id: Some("req-status-completed".into()),
            suspension_id: None,
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: Some(now.saturating_sub(300)),
        },
    });
    store.append(RuntimeDomainEvent::RequirementUpserted {
        requirement: RequirementRecord {
            requirement_id: "req-status-completed".into(),
            run_id: "run-status-completed".into(),
            turn_id: Some("turn-status-completed".into()),
            job_id: "job-status-completed".into(),
            original_user_request: "finish task".into(),
            attempted_rounds: 1,
            max_rounds: 3,
            last_result_fingerprint: Some("fp-done".into()),
            last_reason: Some("satisfied".into()),
            status: RequirementStatus::Satisfied,
            created_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(300),
        },
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Completed);
    assert_eq!(jobs[0].status_label, "completed");
}

#[test]
fn build_job_views_keeps_runtime_status_when_legacy_terminal_events_conflict() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-runtime-priority".into(),
            trace_id: "trace-runtime-priority".into(),
            job_id: "job-runtime-priority".into(),
            chat_id: Some("runtime-priority-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-runtime-priority".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-runtime-priority".into(),
            run_id: "run-runtime-priority".into(),
            source: "initial".into(),
            status: TurnStatus::Running,
            transcript_checkpoint: 0,
            requirement_id: None,
            suspension_id: None,
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::ToolInvocationUpserted {
        invocation: ToolInvocationRuntimeRecord {
            invocation_id: "inv-runtime-priority".into(),
            run_id: "run-runtime-priority".into(),
            turn_id: Some("turn-runtime-priority".into()),
            task_id: Some("task-runtime-priority".into()),
            tool_name: "bash_exec".into(),
            tool_use_id: Some("toolu_runtime_priority".into()),
            phase: "executing".into(),
            permission_state: "allowed".into(),
            input_preview: Some("{\"command\":\"pwd\"}".into()),
            result_summary: None,
            error_summary: None,
            started_at_ms: now.saturating_sub(150),
            finished_at_ms: None,
        },
    });

    let timeline = vec![runtime_event(
        "job-runtime-priority",
        Some("runtime-priority-chat"),
        "message_completed",
        now.saturating_sub(100),
        json!({"response_text": "legacy completed"}),
    )];

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].status_label, "executing");
    assert_eq!(jobs[0].current_step, "正在执行工具：bash_exec");
}

#[test]
fn build_job_views_prefers_runtime_failure_over_legacy_failure_snapshot() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-runtime-failure-priority".into(),
            trace_id: "trace-runtime-failure-priority".into(),
            job_id: "job-runtime-failure-priority".into(),
            chat_id: Some("runtime-failure-chat".into()),
            channel: "web".into(),
            status: RunStatus::Failed,
            current_turn_id: Some("turn-runtime-failure-priority".into()),
            latest_error: Some("runtime failure message".into()),
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: Some(now.saturating_sub(300)),
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-runtime-failure-priority".into(),
            run_id: "run-runtime-failure-priority".into(),
            source: "initial".into(),
            status: TurnStatus::Failed,
            transcript_checkpoint: 0,
            requirement_id: None,
            suspension_id: None,
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(300),
            completed_at_ms: Some(now.saturating_sub(300)),
        },
    });

    let legacy_failures = vec![RuntimeFailureSnapshot {
        error_code: "legacy_failure".into(),
        error_stage: "legacy".into(),
        message_id: "job-runtime-failure-priority".into(),
        channel: "web".into(),
        chat_id: Some("runtime-failure-chat".into()),
        occurred_at_ms: now.saturating_sub(100),
        internal_message: "legacy failure message".into(),
    }];

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &legacy_failures,
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Failed);
    assert_eq!(
        jobs[0]
            .failure
            .as_ref()
            .and_then(|value| value.get("error_code"))
            .and_then(|value| value.as_str()),
        Some("runtime_run_failed")
    );
    assert_eq!(
        jobs[0]
            .failure
            .as_ref()
            .and_then(|value| value.get("internal_message"))
            .and_then(|value| value.as_str()),
        Some("runtime failure message")
    );
}

#[test]
fn build_job_views_prefers_runtime_task_orchestration_summary() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-orch-runtime".into(),
            trace_id: "trace-orch-runtime".into(),
            job_id: "job-orch-runtime".into(),
            chat_id: Some("orch-runtime-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: None,
            latest_error: None,
            created_at_ms: now.saturating_sub(5_000),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "plan-task-1".into(),
            run_id: "run-orch-runtime".into(),
            turn_id: None,
            parent_task_id: None,
            trace_id: "trace-orch-runtime".into(),
            job_id: "job-orch-runtime".into(),
            parent_job_id: None,
            plan_id: Some("plan-runtime-1".into()),
            kind: TaskKind::Plan,
            name: "runtime plan".into(),
            task_type: "plan".into(),
            description: "runtime orchestration plan".into(),
            status: TaskStatus::Running,
            execution_mode: None,
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: Some(now.saturating_sub(4_000)),
            updated_at_ms: now.saturating_sub(1_000),
            completed_at_ms: None,
            error: None,
        },
    });
    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "subtask-1".into(),
            run_id: "run-orch-runtime".into(),
            turn_id: None,
            parent_task_id: Some("plan-task-1".into()),
            trace_id: "trace-orch-runtime".into(),
            job_id: "job-orch-runtime".into(),
            parent_job_id: None,
            plan_id: Some("plan-runtime-1".into()),
            kind: TaskKind::Subtask,
            name: "collect-data".into(),
            task_type: "collection".into(),
            description: "collect data".into(),
            status: TaskStatus::Completed,
            execution_mode: Some("serial".into()),
            result_kind: None,
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: Some(now.saturating_sub(3_500)),
            updated_at_ms: now.saturating_sub(2_000),
            completed_at_ms: Some(now.saturating_sub(2_000)),
            error: None,
        },
    });
    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "subtask-2".into(),
            run_id: "run-orch-runtime".into(),
            turn_id: None,
            parent_task_id: Some("plan-task-1".into()),
            trace_id: "trace-orch-runtime".into(),
            job_id: "job-orch-runtime".into(),
            parent_job_id: None,
            plan_id: Some("plan-runtime-1".into()),
            kind: TaskKind::Subtask,
            name: "generate-report".into(),
            task_type: "reporting".into(),
            description: "generate report".into(),
            status: TaskStatus::Running,
            execution_mode: Some("whitelist_parallel".into()),
            result_kind: Some("transform".into()),
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: Some(now.saturating_sub(1_500)),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
            error: None,
        },
    });

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );

    assert_eq!(jobs.len(), 1);
    let orchestration = jobs[0]
        .orchestration
        .as_ref()
        .expect("expected runtime orchestration");
    assert_eq!(orchestration.plan_id.as_deref(), Some("plan-runtime-1"));
    assert_eq!(orchestration.total_subtasks, 2);
    assert_eq!(orchestration.completed_subtasks, 1);
    assert_eq!(orchestration.active_subtasks, 1);
    assert_eq!(orchestration.failed_subtasks, 0);
    assert_eq!(
        orchestration.active_subtask_name.as_deref(),
        Some("generate-report")
    );
    assert_eq!(
        orchestration.active_subtask_type.as_deref(),
        Some("reporting")
    );
    assert_eq!(
        orchestration.active_subtask_id.as_deref(),
        Some("subtask-2")
    );
    assert_eq!(jobs[0].current_step, "正在执行子任务：generate-report");
}

#[test]
fn build_job_views_preserve_orchestration_parallel_payload_details() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-orchestration-p2";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("orch-chat"),
            "orchestration_plan_created",
            now.saturating_sub(3_000),
            json!({
                "plan_id": "plan-p2",
                "subtask_count": 3,
                "parallel_groups": [["collect-data"], ["analyze-data", "generate-report"]]
            }),
        ),
        runtime_event(
            message_id,
            Some("orch-chat"),
            "orchestration_subtask_started",
            now.saturating_sub(2_000),
            json!({
                "plan_id": "plan-p2",
                "subtask_id": "subtask-1",
                "subtask_name": "generate-report",
                "original_task_type": "reporting",
                "lowered_task_type": "transform",
                "parallel_safe": true,
                "parallel_group_index": 1,
                "parallel_group_size": 2,
                "execution_mode": "whitelist_parallel",
                "result_kind": "transform"
            }),
        ),
        runtime_event(
            message_id,
            Some("orch-chat"),
            "orchestration_subtask_completed",
            now.saturating_sub(1_000),
            json!({
                "plan_id": "plan-p2",
                "subtask_id": "subtask-1",
                "subtask_name": "generate-report",
                "original_task_type": "reporting",
                "lowered_task_type": "transform",
                "parallel_safe": true,
                "parallel_group_index": 1,
                "parallel_group_size": 2,
                "execution_mode": "whitelist_parallel",
                "result_kind": "transform",
                "result_preview": "report ready"
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0]
            .orchestration
            .as_ref()
            .map(|view| view.plan_id.as_deref()),
        Some(Some("plan-p2"))
    );
    let recent_events = &jobs[0].recent_events;
    assert_eq!(
        recent_events[1].payload["execution_mode"],
        "whitelist_parallel"
    );
    assert_eq!(recent_events[1].payload["parallel_safe"], true);
    assert_eq!(recent_events[2].payload["result_kind"], "transform");
    assert_eq!(recent_events[2].payload["parallel_group_size"], 2);
}

#[test]
fn build_job_views_show_preparing_context_after_message_received() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-preparing";
    let timeline = vec![runtime_event(
        message_id,
        Some("preparing-chat"),
        "message_received",
        now.saturating_sub(1_000),
        json!({}),
    )];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::PreparingContext);
    assert_eq!(jobs[0].status_label, "preparing_context");
}

#[test]
fn build_job_views_show_planning_after_session_ready() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-planning";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("planning-chat"),
            "message_received",
            now.saturating_sub(2_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("planning-chat"),
            "session_ready",
            now.saturating_sub(1_000),
            json!({}),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Planning);
    assert_eq!(jobs[0].status_label, "planning");
    assert!(jobs[0].current_step.contains("构建计划"));
}

#[test]
fn build_job_views_show_creating_session_when_worker_is_creating_session() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-session-create";
    let timeline = vec![runtime_event(
        message_id,
        Some("create-chat"),
        "message_received",
        now.saturating_sub(1_000),
        json!({}),
    )];
    let workers = vec![WorkerStatus {
        id: "worker-1".into(),
        status: "busy".into(),
        metrics: WorkerMetrics::default(),
        current_task: Some(anima_runtime::agent::CurrentTaskInfo {
            task_id: message_id.into(),
            trace_id: message_id.into(),
            task_type: "session-create".into(),
            content_preview: "create upstream session".into(),
            started_ms: now.saturating_sub(500),
            phase: "api_call_inflight".into(),
            last_progress_at_ms: now.saturating_sub(200),
            opencode_session_id: None,
        }),
    }];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &workers,
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::CreatingSession);
    assert_eq!(jobs[0].status_label, "creating_session");
    assert_eq!(
        jobs[0]
            .worker
            .as_ref()
            .and_then(|worker| worker.phase.as_deref()),
        Some("api_call_inflight")
    );
    assert!(jobs[0].current_step.contains("api_call_inflight"));
}

#[test]
fn build_job_views_expose_orchestration_progress_on_main_job() {
    let now = anima_runtime::support::now_ms();
    let timeline = vec![
        runtime_event(
            "job-main-orch",
            Some("chat-orch"),
            "orchestration_plan_created",
            now.saturating_sub(4_000),
            json!({"plan_id": "plan-1", "subtask_count": 2}),
        ),
        runtime_event(
            "job-main-orch",
            Some("chat-orch"),
            "orchestration_subtask_started",
            now.saturating_sub(3_000),
            json!({"subtask_id": "sub-1", "subtask_name": "design-api", "parent_job_id": "job-main-orch", "plan_id": "plan-1"}),
        ),
        runtime_event(
            "job-main-orch",
            Some("chat-orch"),
            "orchestration_subtask_completed",
            now.saturating_sub(2_000),
            json!({"subtask_id": "sub-1", "subtask_name": "design-api", "parent_job_id": "job-main-orch", "plan_id": "plan-1"}),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );
    assert_eq!(jobs.len(), 1);
    let orchestration = jobs[0]
        .orchestration
        .as_ref()
        .expect("expected orchestration summary");
    assert_eq!(orchestration.plan_id.as_deref(), Some("plan-1"));
    assert_eq!(orchestration.total_subtasks, 2);
    assert_eq!(orchestration.completed_subtasks, 1);
    assert_eq!(jobs[0].current_step, "已进入队列");
}

#[test]
fn build_job_views_show_executing_when_worker_is_busy_on_non_session_task() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-executing";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("executing-chat"),
            "message_received",
            now.saturating_sub(3_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("executing-chat"),
            "session_ready",
            now.saturating_sub(2_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("executing-chat"),
            "plan_built",
            now.saturating_sub(1_500),
            json!({"plan_type": "single"}),
        ),
    ];
    let workers = vec![WorkerStatus {
        id: "worker-1".into(),
        status: "busy".into(),
        metrics: WorkerMetrics::default(),
        current_task: Some(anima_runtime::agent::CurrentTaskInfo {
            task_id: message_id.into(),
            trace_id: message_id.into(),
            task_type: "api-call".into(),
            content_preview: "run plan".into(),
            started_ms: now.saturating_sub(500),
            phase: "api_call_inflight".into(),
            last_progress_at_ms: now.saturating_sub(200),
            opencode_session_id: None,
        }),
    }];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &workers,
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].status_label, "executing");
    assert_eq!(
        jobs[0]
            .worker
            .as_ref()
            .and_then(|worker| worker.phase.as_deref()),
        Some("api_call_inflight")
    );
    assert!(jobs[0].current_step.contains("worker 正在执行"));
    assert!(jobs[0].current_step.contains("api_call_inflight"));
}

#[test]
fn build_job_views_worker_overrides_runtime_projection_status_when_active() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-worker-override".into(),
            trace_id: "trace-worker-override".into(),
            job_id: "job-worker-override".into(),
            chat_id: Some("worker-override-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-worker-override".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-worker-override".into(),
            run_id: "run-worker-override".into(),
            source: "initial".into(),
            status: TurnStatus::Running,
            transcript_checkpoint: 0,
            requirement_id: None,
            suspension_id: None,
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: None,
        },
    });
    let workers = vec![WorkerStatus {
        id: "worker-1".into(),
        status: "busy".into(),
        metrics: WorkerMetrics::default(),
        current_task: Some(anima_runtime::agent::CurrentTaskInfo {
            task_id: "job-worker-override".into(),
            trace_id: "job-worker-override".into(),
            task_type: "api-call".into(),
            content_preview: "run plan".into(),
            started_ms: now.saturating_sub(500),
            phase: "api_call_inflight".into(),
            last_progress_at_ms: now.saturating_sub(100),
            opencode_session_id: None,
        }),
    }];

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &workers,
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].status_label, "executing");
    assert!(jobs[0].current_step.contains("worker 正在执行"));
    assert!(jobs[0].current_step.contains("api_call_inflight"));
}

#[test]
fn build_job_views_waiting_user_input_runtime_status_is_not_overridden_by_worker() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-worker-waiting-priority".into(),
            trace_id: "trace-worker-waiting-priority".into(),
            job_id: "job-worker-waiting-priority".into(),
            chat_id: Some("worker-waiting-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-worker-waiting-priority".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(4_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TurnUpserted {
        turn: TurnRecord {
            turn_id: "turn-worker-waiting-priority".into(),
            run_id: "run-worker-waiting-priority".into(),
            source: "initial".into(),
            status: TurnStatus::Waiting,
            transcript_checkpoint: 0,
            requirement_id: Some("req-worker-waiting-priority".into()),
            suspension_id: Some("susp-worker-waiting-priority".into()),
            started_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(200),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::RequirementUpserted {
        requirement: RequirementRecord {
            requirement_id: "req-worker-waiting-priority".into(),
            run_id: "run-worker-waiting-priority".into(),
            turn_id: Some("turn-worker-waiting-priority".into()),
            job_id: "job-worker-waiting-priority".into(),
            original_user_request: "need approval".into(),
            attempted_rounds: 1,
            max_rounds: 3,
            last_result_fingerprint: None,
            last_reason: Some("await input".into()),
            status: RequirementStatus::WaitingUserInput,
            created_at_ms: now.saturating_sub(3_000),
            updated_at_ms: now.saturating_sub(200),
        },
    });
    store.append(RuntimeDomainEvent::SuspensionUpserted {
        suspension: SuspensionRecord {
            suspension_id: "susp-worker-waiting-priority".into(),
            run_id: "run-worker-waiting-priority".into(),
            turn_id: "turn-worker-waiting-priority".into(),
            task_id: Some("task-worker-waiting-priority".into()),
            question_id: Some("question-worker-waiting-priority".into()),
            invocation_id: None,
            kind: SuspensionKind::Question,
            status: SuspensionStatus::Active,
            prompt: Some("请确认是否继续".into()),
            options: vec![],
            raw_payload: json!({"type": "question"}),
            resolution_source: None,
            answer_summary: None,
            created_at_ms: now.saturating_sub(1_000),
            updated_at_ms: now.saturating_sub(200),
            resolved_at_ms: None,
            cleared_at_ms: None,
        },
    });
    let workers = vec![WorkerStatus {
        id: "worker-1".into(),
        status: "busy".into(),
        metrics: WorkerMetrics::default(),
        current_task: Some(anima_runtime::agent::CurrentTaskInfo {
            task_id: "job-worker-waiting-priority".into(),
            trace_id: "job-worker-waiting-priority".into(),
            task_type: "api-call".into(),
            content_preview: "run plan".into(),
            started_ms: now.saturating_sub(500),
            phase: "api_call_inflight".into(),
            last_progress_at_ms: now.saturating_sub(100),
            opencode_session_id: None,
        }),
    }];

    let jobs = anima_web::jobs::build_job_views_with_runtime(
        &[],
        &[],
        &[],
        &workers,
        &anima_web::jobs::JobStore::default(),
        &store.snapshot(),
    );
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::WaitingUserInput);
    assert_eq!(jobs[0].status_label, "waiting_user_input");
    assert_eq!(jobs[0].current_step, "请确认是否继续");
}

#[test]
fn build_job_views_waiting_upstream_input_does_not_override_active_worker() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-active-worker";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("active-chat"),
            "message_received",
            now.saturating_sub(40_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("active-chat"),
            "session_ready",
            now.saturating_sub(35_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("active-chat"),
            "plan_built",
            now.saturating_sub(25_000),
            json!({"plan_type": "single"}),
        ),
    ];
    let workers = vec![WorkerStatus {
        id: "worker-1".into(),
        status: "busy".into(),
        metrics: WorkerMetrics::default(),
        current_task: Some(anima_runtime::agent::CurrentTaskInfo {
            task_id: message_id.into(),
            trace_id: message_id.into(),
            task_type: "api-call".into(),
            content_preview: "still executing".into(),
            started_ms: now.saturating_sub(2_000),
            phase: "api_call_inflight".into(),
            last_progress_at_ms: now.saturating_sub(500),
            opencode_session_id: None,
        }),
    }];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &workers,
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
}

#[test]
fn build_job_views_waiting_upstream_input_does_not_override_completed_message() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-completed";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("completed-chat"),
            "message_received",
            now.saturating_sub(40_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("completed-chat"),
            "session_ready",
            now.saturating_sub(35_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("completed-chat"),
            "plan_built",
            now.saturating_sub(25_000),
            json!({"plan_type": "single"}),
        ),
        runtime_event(
            message_id,
            Some("completed-chat"),
            "requirement_satisfied",
            now.saturating_sub(21_000),
            json!({"response_preview": "preview"}),
        ),
        runtime_event(
            message_id,
            Some("completed-chat"),
            "message_completed",
            now.saturating_sub(20_000),
            json!({
                "response_preview": "preview",
                "response_text": "full response text"
            }),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Completed);
    assert_eq!(jobs[0].status_label, "completed");
    assert!(jobs[0].current_step.contains("满足需求"));
    assert_eq!(
        jobs[0]
            .recent_events
            .last()
            .and_then(|event| event.payload.get("response_text"))
            .and_then(|value| value.as_str()),
        Some("full response text")
    );
}

#[test]
fn build_job_views_show_executing_after_cache_signal_without_active_worker() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-cache-progress";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("cache-chat"),
            "message_received",
            now.saturating_sub(40_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("cache-chat"),
            "session_ready",
            now.saturating_sub(35_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("cache-chat"),
            "plan_built",
            now.saturating_sub(30_000),
            json!({"plan_type": "single"}),
        ),
        runtime_event(
            message_id,
            Some("cache-chat"),
            "cache_miss",
            now.saturating_sub(20_000),
            json!({"plan_type": "single"}),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].status_label, "executing");
    assert!(jobs[0].current_step.contains("执行阶段"));
}

#[test]
fn accepted_review_keeps_completed_job_completed() {
    let mut store = anima_web::jobs::JobStore::default();
    let now = anima_runtime::support::now_ms();
    let job_id = "job-accepted";
    let timeline = vec![
        runtime_event(
            job_id,
            Some("review-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "session_ready",
            now.saturating_sub(3_000),
            json!({}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "plan_built",
            now.saturating_sub(2_000),
            json!({"plan_type": "single"}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "requirement_satisfied",
            now.saturating_sub(1_500),
            json!({"response_preview": "done"}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "message_completed",
            now.saturating_sub(1_000),
            json!({"response_text": "done"}),
        ),
    ];
    store.record_review(
        job_id.into(),
        anima_web::jobs::JobReviewInput {
            user_verdict: anima_web::jobs::UserVerdict::Accepted,
            reason: None,
            note: None,
        },
    );

    let jobs = anima_web::jobs::build_job_views(&timeline, &[], &[], &[], &store);

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Completed);
    assert_eq!(jobs[0].status_label, "completed");
    assert_eq!(
        jobs[0].review.as_ref().map(|review| &review.verdict),
        Some(&anima_web::jobs::UserVerdict::Accepted)
    );
}

#[test]
fn rejected_review_keeps_completed_job_completed() {
    let mut store = anima_web::jobs::JobStore::default();
    let now = anima_runtime::support::now_ms();
    let job_id = "job-rejected";
    let timeline = vec![
        runtime_event(
            job_id,
            Some("review-chat"),
            "message_received",
            now.saturating_sub(4_000),
            json!({}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "session_ready",
            now.saturating_sub(3_000),
            json!({}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "plan_built",
            now.saturating_sub(2_000),
            json!({"plan_type": "single"}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "requirement_satisfied",
            now.saturating_sub(1_500),
            json!({"response_preview": "done"}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "message_completed",
            now.saturating_sub(1_000),
            json!({"response_text": "done"}),
        ),
    ];
    store.record_review(
        job_id.into(),
        anima_web::jobs::JobReviewInput {
            user_verdict: anima_web::jobs::UserVerdict::Rejected,
            reason: None,
            note: None,
        },
    );

    let jobs = anima_web::jobs::build_job_views(&timeline, &[], &[], &[], &store);

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Completed);
    assert_eq!(jobs[0].status_label, "completed");
    assert_eq!(
        jobs[0].review.as_ref().map(|review| &review.verdict),
        Some(&anima_web::jobs::UserVerdict::Rejected)
    );
}

#[test]
fn review_does_not_override_runtime_failure() {
    let mut store = anima_web::jobs::JobStore::default();
    let now = anima_runtime::support::now_ms();
    let job_id = "job-failed-with-review";
    let timeline = vec![
        runtime_event(
            job_id,
            Some("review-chat"),
            "message_received",
            now.saturating_sub(3_000),
            json!({}),
        ),
        runtime_event(
            job_id,
            Some("review-chat"),
            "message_failed",
            now.saturating_sub(1_000),
            json!({"error_code": "task_execution_failed"}),
        ),
    ];
    store.record_review(
        job_id.into(),
        anima_web::jobs::JobReviewInput {
            user_verdict: anima_web::jobs::UserVerdict::Accepted,
            reason: None,
            note: None,
        },
    );

    let jobs = anima_web::jobs::build_job_views(&timeline, &[], &[], &[], &store);

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Failed);
    assert_eq!(jobs[0].status_label, "failed");
    assert_eq!(
        jobs[0].review.as_ref().map(|review| &review.verdict),
        Some(&anima_web::jobs::UserVerdict::Accepted)
    );
}

#[test]
fn build_job_views_show_failed_after_session_create_failed_event() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-session-create-failed";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("failed-chat"),
            "message_received",
            now.saturating_sub(3_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("failed-chat"),
            "session_create_failed",
            now.saturating_sub(2_000),
            json!({"error_code": "session_create_failed", "error_stage": "session_create"}),
        ),
    ];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Failed);
    assert_eq!(jobs[0].status_label, "failed");
}

#[test]
fn build_job_views_show_executing_after_cache_hit_without_active_worker() {
    let now = anima_runtime::support::now_ms();
    let message_id = "job-cache-hit-progress";
    let timeline = vec![
        runtime_event(
            message_id,
            Some("cache-hit-chat"),
            "message_received",
            now.saturating_sub(40_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("cache-hit-chat"),
            "session_ready",
            now.saturating_sub(35_000),
            json!({}),
        ),
        runtime_event(
            message_id,
            Some("cache-hit-chat"),
            "plan_built",
            now.saturating_sub(30_000),
            json!({"plan_type": "single"}),
        ),
        runtime_event(
            message_id,
            Some("cache-hit-chat"),
            "cache_hit",
            now.saturating_sub(20_000),
            json!({"plan_type": "single"}),
        ),
    ];
    let summaries = vec![ExecutionSummary {
        trace_id: message_id.into(),
        message_id: message_id.into(),
        channel: "web".into(),
        chat_id: Some("cache-hit-chat".into()),
        plan_type: "single".into(),
        status: "success".into(),
        cache_hit: true,
        worker_id: None,
        error_code: None,
        error_stage: None,
        task_duration_ms: 0,
        stages: ExecutionStageDurations {
            context_ms: 1,
            session_ms: 1,
            classify_ms: 1,
            execute_ms: 0,
            total_ms: 3,
        },
    }];

    let jobs = anima_web::jobs::build_job_views(
        &timeline,
        &summaries,
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
    );

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, anima_web::jobs::JobStatus::Executing);
    assert_eq!(jobs[0].status_label, "executing");
    assert!(jobs[0].current_step.contains("执行阶段"));
}

#[derive(Debug, Default)]
struct MockExecutor;

#[derive(Debug, Default)]
struct IdleExecutor;

#[derive(Debug, Default)]
struct FailingExecutor;

#[derive(Debug, Default)]
struct OrchestrationQuestionExecutor;

#[derive(Debug, Default)]
struct OrchestrationFollowupExecutor;

fn mock_response_to_sse_lines(response: &Value) -> Vec<String> {
    let text = response
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();

    vec![
        format!(
            "data: {}",
            json!({"type": "message_start", "message": {"id": "msg_mock"}})
        ),
        String::new(),
        format!(
            "data: {}",
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text", "text": ""}
            })
        ),
        String::new(),
        format!(
            "data: {}",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": text}
            })
        ),
        String::new(),
        format!("data: {}", json!({"type": "content_block_stop", "index": 0})),
        String::new(),
        format!(
            "data: {}",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}})
        ),
        String::new(),
        format!("data: {}", json!({"type": "message_stop"})),
        String::new(),
    ]
}

impl TaskExecutor for MockExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        Ok(json!({
            "content": format!("reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-web"}))
    }

    fn send_prompt_streaming(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<anima_runtime::agent::executor::UnifiedStreamSource, String> {
        let response = self.send_prompt(client, session_id, content)?;
        Ok(Box::new(
            mock_response_to_sse_lines(&response).into_iter().map(Ok),
        ))
    }
}

impl TaskExecutor for IdleExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({
            "content": "idle"
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-web-idle"}))
    }
}

impl TaskExecutor for FailingExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("upstream exploded".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-web-fail"}))
    }
}

impl TaskExecutor for OrchestrationQuestionExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text.contains("[orchestration/") {
            Ok(json!({
                "question": {
                    "id": "status-orch-question-1",
                    "kind": "input",
                    "prompt": "需要确认 status orchestration 是否继续",
                    "options": ["继续", "停止"]
                }
            }))
        } else {
            Ok(json!({
                "content": format!("reply[{session_id}]: orchestration continued with {text}")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-web-orch-question"}))
    }
}

impl TaskExecutor for OrchestrationFollowupExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text.contains("[orchestration/") {
            Ok(json!({
                "content": "I need more information before I can conclude this orchestration task."
            }))
        } else {
            Ok(json!({
                "content": format!("reply[{session_id}]: orchestration followup completed")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-web-orch-followup"}))
    }
}

#[test]
fn jobs_api_prefers_runtime_payload_hierarchy_for_subtasks() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(IdleExecutor))
        .build();
    let bus = runtime.bus.clone();
    runtime.start();

    let mut store = anima_web::jobs::JobStore::default();
    store.register_accepted_job(anima_web::jobs::AcceptedJob {
        job_id: "job-main".into(),
        trace_id: "job-main".into(),
        message_id: "job-main".into(),
        kind: JobKind::Main,
        parent_job_id: None,
        channel: "web".into(),
        chat_id: Some("hierarchy-chat".into()),
        sender_id: "web-user".into(),
        user_content: "main task".into(),
        accepted_at_ms: anima_runtime::support::now_ms().saturating_sub(4_000),
    });

    runtime
        .agent
        .core_agent()
        .process_inbound_message(make_inbound(MakeInbound {
            channel: "web".into(),
            sender_id: Some("web-user".into()),
            chat_id: Some("hierarchy-chat".into()),
            content: "sub task".into(),
            metadata: Some(json!({
                "parent_job_id": "job-main",
                "subtask_id": "job-subtask",
                "plan_id": "plan-1"
            })),
            ..Default::default()
        }));

    let state = build_state_with_store(runtime, bus, web_channel, store);
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let response = tokio_rt
        .block_on(async {
            app.oneshot(
                Request::builder()
                    .uri("/api/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
        })
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio_rt
        .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let jobs = payload["jobs"].as_array().unwrap();

    let main_job = jobs.iter().find(|job| job["job_id"] == "job-main").unwrap();
    assert_eq!(main_job["kind"], "main");
    assert!(main_job["parent_job_id"].is_null());

    let subtask_job = jobs
        .iter()
        .find(|job| job["job_id"] == "job-subtask")
        .unwrap();
    assert_eq!(subtask_job["message_id"], "job-subtask");
    assert_eq!(subtask_job["kind"], "subtask");
    assert_eq!(subtask_job["parent_job_id"], "job-main");

    let mut status = state.runtime.lock().agent.status();
    for _ in 0..120 {
        let has_activity = status
            .core
            .runtime_timeline
            .iter()
            .any(|event| event.message_id == "job-subtask" && event.event == "message_received");
        if has_activity {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        status = state.runtime.lock().agent.status();
    }
    let subtask_events = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == "job-subtask")
        .collect::<Vec<_>>();
    assert!(subtask_events
        .iter()
        .any(|event| event.event == "message_received"));
    assert!(subtask_events
        .iter()
        .any(|event| event.event == "plan_built"));
    assert!(subtask_events
        .iter()
        .all(|event| event.payload["parent_job_id"] == "job-main"));

    state.runtime.lock().stop();
}

#[test]
fn jobs_api_returns_subtask_hierarchy_from_store_metadata() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(IdleExecutor))
        .build();
    let bus = runtime.bus.clone();
    let mut store = anima_web::jobs::JobStore::default();
    store.register_accepted_job(anima_web::jobs::AcceptedJob {
        job_id: "job-main".into(),
        trace_id: "job-main".into(),
        message_id: "job-main".into(),
        kind: JobKind::Main,
        parent_job_id: None,
        channel: "web".into(),
        chat_id: Some("hierarchy-chat".into()),
        sender_id: "web-user".into(),
        user_content: "main task".into(),
        accepted_at_ms: anima_runtime::support::now_ms().saturating_sub(2_000),
    });
    store.register_accepted_job(anima_web::jobs::AcceptedJob {
        job_id: "job-subtask".into(),
        trace_id: "job-subtask".into(),
        message_id: "job-subtask".into(),
        kind: JobKind::Subtask,
        parent_job_id: Some("job-main".into()),
        channel: "web".into(),
        chat_id: Some("hierarchy-chat".into()),
        sender_id: "web-user".into(),
        user_content: "sub task".into(),
        accepted_at_ms: anima_runtime::support::now_ms().saturating_sub(1_000),
    });
    let state = build_state_with_store(runtime, bus, web_channel, store);

    let app = routes::create_routes().with_state(state);
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let response = tokio_rt
        .block_on(async {
            app.oneshot(
                Request::builder()
                    .uri("/api/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
        })
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio_rt
        .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let jobs = payload["jobs"].as_array().unwrap();

    let main_job = jobs.iter().find(|job| job["job_id"] == "job-main").unwrap();
    assert_eq!(main_job["kind"], "main");
    assert!(main_job["parent_job_id"].is_null());

    let subtask_job = jobs
        .iter()
        .find(|job| job["job_id"] == "job-subtask")
        .unwrap();
    assert_eq!(subtask_job["kind"], "subtask");
    assert_eq!(subtask_job["parent_job_id"], "job-main");
}

#[test]
fn jobs_and_status_api_share_runtime_projection_semantics() {
    let now = anima_runtime::support::now_ms();
    let store = RuntimeStateStore::new();
    store.append(RuntimeDomainEvent::RunUpserted {
        run: RunRecord {
            run_id: "run-sync-runtime".into(),
            trace_id: "trace-sync-runtime".into(),
            job_id: "job-sync-runtime".into(),
            chat_id: Some("sync-chat".into()),
            channel: "web".into(),
            status: RunStatus::Running,
            current_turn_id: Some("turn-sync-runtime".into()),
            latest_error: None,
            created_at_ms: now.saturating_sub(5_000),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
        },
    });
    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "plan-task-sync".into(),
            run_id: "run-sync-runtime".into(),
            turn_id: Some("turn-sync-runtime".into()),
            parent_task_id: None,
            trace_id: "trace-sync-runtime".into(),
            job_id: "job-sync-runtime".into(),
            parent_job_id: None,
            plan_id: Some("plan-sync-runtime".into()),
            kind: TaskKind::Plan,
            name: "orchestration_plan".into(),
            task_type: "orchestration_plan".into(),
            description: "sync orchestration".into(),
            status: TaskStatus::Running,
            execution_mode: None,
            result_kind: Some("orchestration_plan".into()),
            specialist_type: Some("data-analysis".into()),
            dependencies: vec![],
            metadata: json!({}),
            started_at_ms: Some(now.saturating_sub(4_000)),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
            error: None,
        },
    });
    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "subtask-sync-1".into(),
            run_id: "run-sync-runtime".into(),
            turn_id: Some("turn-sync-runtime".into()),
            parent_task_id: Some("plan-task-sync".into()),
            trace_id: "trace-sync-runtime".into(),
            job_id: "job-sync-runtime".into(),
            parent_job_id: None,
            plan_id: Some("plan-sync-runtime".into()),
            kind: TaskKind::Subtask,
            name: "collect-data".into(),
            task_type: "data-collection".into(),
            description: "collect data".into(),
            status: TaskStatus::Completed,
            execution_mode: Some("serial".into()),
            result_kind: Some("upstream".into()),
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({
                "original_task_type": "data-collection",
                "lowered_task_type": "api-call"
            }),
            started_at_ms: Some(now.saturating_sub(3_000)),
            updated_at_ms: now.saturating_sub(2_000),
            completed_at_ms: Some(now.saturating_sub(2_000)),
            error: None,
        },
    });
    store.append(RuntimeDomainEvent::TaskUpserted {
        task: TaskRecord {
            task_id: "subtask-sync-2".into(),
            run_id: "run-sync-runtime".into(),
            turn_id: Some("turn-sync-runtime".into()),
            parent_task_id: Some("plan-task-sync".into()),
            trace_id: "trace-sync-runtime".into(),
            job_id: "job-sync-runtime".into(),
            parent_job_id: None,
            plan_id: Some("plan-sync-runtime".into()),
            kind: TaskKind::Subtask,
            name: "generate-report".into(),
            task_type: "reporting".into(),
            description: "generate report".into(),
            status: TaskStatus::Running,
            execution_mode: Some("whitelist_parallel".into()),
            result_kind: Some("transform".into()),
            specialist_type: None,
            dependencies: vec![],
            metadata: json!({
                "original_task_type": "reporting",
                "lowered_task_type": "transform"
            }),
            started_at_ms: Some(now.saturating_sub(1_500)),
            updated_at_ms: now.saturating_sub(500),
            completed_at_ms: None,
            error: None,
        },
    });

    let runtime_snapshot = store.snapshot();
    let runtime_projection = anima_runtime::runtime::build_projection(&runtime_snapshot);
    let jobs = anima_web::jobs::build_job_views_with_projection(
        &[],
        &[],
        &[],
        &[],
        &anima_web::jobs::JobStore::default(),
        &runtime_snapshot,
        &runtime_projection,
    );

    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    let orchestration = job
        .orchestration
        .as_ref()
        .expect("expected runtime orchestration");
    assert_eq!(orchestration.plan_id.as_deref(), Some("plan-sync-runtime"));
    assert_eq!(orchestration.total_subtasks, 2);
    assert_eq!(orchestration.completed_subtasks, 1);
    assert_eq!(orchestration.active_subtasks, 1);
    assert_eq!(
        orchestration.active_subtask_name.as_deref(),
        Some("generate-report")
    );
    assert_eq!(
        orchestration.active_subtask_type.as_deref(),
        Some("reporting")
    );

    let unified_tasks = runtime_projection.tasks;
    let unified_job_status = runtime_projection
        .job_statuses
        .get("job-sync-runtime")
        .expect("expected runtime job status");
    let unified_orchestration = runtime_projection
        .orchestration
        .get("job-sync-runtime")
        .expect("expected runtime orchestration summary");
    let unified_execution_summary = runtime_projection
        .execution_summaries
        .get("job-sync-runtime")
        .expect("expected execution summary");

    assert_eq!(
        unified_orchestration.plan_id.as_deref(),
        orchestration.plan_id.as_deref()
    );
    assert_eq!(
        unified_orchestration.total_subtasks,
        orchestration.total_subtasks
    );
    assert_eq!(
        unified_orchestration.completed_subtasks,
        orchestration.completed_subtasks
    );
    assert_eq!(
        unified_orchestration.active_subtasks,
        orchestration.active_subtasks
    );
    assert_eq!(
        unified_orchestration.active_subtask_name.as_deref(),
        orchestration.active_subtask_name.as_deref()
    );
    assert_eq!(
        unified_orchestration.active_subtask_type.as_deref(),
        orchestration.active_subtask_type.as_deref()
    );
    assert_eq!(job.status_label, unified_job_status.status_label);
    assert_eq!(unified_execution_summary.plan_type, "orchestration-v1");
    assert_eq!(unified_execution_summary.status, "running");
    assert_eq!(job.current_step, "正在执行子任务：generate-report");
    assert_eq!(
        unified_job_status.current_step,
        "正在执行子任务：generate-report"
    );
    assert!(unified_tasks.iter().any(|task| {
        task.kind == TaskKind::Subtask
            && task.name == "generate-report"
            && task.task_type == "reporting"
            && task.metadata["lowered_task_type"] == "transform"
    }));
}

#[test]
fn status_api_exposes_runtime_summary() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(MockExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    bus.publish_inbound(make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: Some("web-session".into()),
        content: "status please".into(),
        session_key: Some("web-session".into()),
        ..Default::default()
    }))
    .unwrap();

    let state = build_state_with_runtime(runtime, bus, web_channel);

    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let mut payload: Option<Value> = None;
    for _ in 0..120 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let current_payload: Value = serde_json::from_slice(&body).unwrap();
        let timeline_events = current_payload["runtime_timeline"]
            .as_array()
            .map(|timeline| {
                timeline
                    .iter()
                    .map(|entry| entry["event"].as_str().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let processed = current_payload["metrics"]["counters"]["messages_processed"]
            .as_u64()
            .unwrap_or_default();
        let has_completed_job = current_payload["jobs"]
            .as_array()
            .map(|jobs| jobs.iter().any(|job| job["status"] == "completed"))
            .unwrap_or(false);
        if processed >= 1 && (timeline_events.contains(&"message_completed") || has_completed_job) {
            payload = Some(current_payload);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(75));
    }
    let payload = payload.expect("expected completed status payload");

    assert_eq!(payload["agent"]["running"], true);
    assert_eq!(payload["agent"]["status"], "running");
    assert_eq!(payload["agent"]["sessions_count"], 1);
    assert_eq!(payload["worker_pool"]["status"], "running");
    assert!(payload["worker_pool"]["size"].as_u64().unwrap() >= 1);
    assert!(
        payload["metrics"]["counters"]["messages_processed"]
            .as_u64()
            .unwrap()
            >= 1
    );
    assert_eq!(payload["metrics"]["gauges"]["sessions_active"], 1);
    assert!(payload["metrics"]["counters"]["bus_inbound_dropped_total"].is_number());
    assert!(payload["metrics"]["gauges"]["bus_inbound_queue_depth"].is_number());
    assert_eq!(payload["warnings"]["bus_overflow_active"], false);
    assert_eq!(payload["warnings"]["bus_drop_total"], 0);
    assert_eq!(payload["recent_sessions"].as_array().unwrap().len(), 1);
    assert_eq!(payload["recent_sessions"][0]["chat_id"], "web-session");
    assert_eq!(
        payload["recent_sessions"][0]["session_id"],
        "mock-session-web"
    );
    assert!(payload["failures"]["last_failure"].is_null());
    assert_eq!(
        payload["failures"]["counts_by_error_code"]
            .as_object()
            .unwrap()
            .len(),
        0
    );
    assert!(payload["unified_runtime"]["execution_summaries"].is_object());
    assert!(payload["unified_runtime"]["failures"].is_object());
    assert!(payload["unified_runtime"]["orchestration"].is_object());
    assert!(payload["unified_runtime"]["pending_questions"].is_object());
    assert!(payload["unified_runtime"]["tool_states"].is_object());
    assert!(payload["unified_runtime"]["job_statuses"].is_object());

    let timeline = payload["runtime_timeline"].as_array().unwrap();
    assert!(!timeline.is_empty());
    let timeline_events = timeline
        .iter()
        .map(|entry| entry["event"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(timeline_events.contains(&"message_received"));
    assert!(timeline_events.contains(&"session_ready"));
    assert!(timeline_events.contains(&"plan_built"));
    assert!(timeline_events.contains(&"worker_task_assigned"));
    assert!(timeline_events.contains(&"api_call_started"));
    assert!(timeline_events.contains(&"upstream_response_observed"));
    let has_message_completed = timeline_events.contains(&"message_completed");

    let assignment_event = timeline
        .iter()
        .find(|entry| {
            entry["event"] == "worker_task_assigned" && entry["payload"]["task_type"] == "api-call"
        })
        .expect("expected api-call worker_task_assigned event");
    assert_eq!(assignment_event["payload"]["task_type"], "api-call");
    assert!(assignment_event["payload"]["task_summary"]
        .as_str()
        .unwrap_or("")
        .contains("派发"));
    assert_eq!(
        assignment_event["payload"]["opencode_session_id"],
        "mock-session-web"
    );

    let api_started_event = timeline
        .iter()
        .find(|entry| entry["event"] == "api_call_started")
        .expect("expected api_call_started event");
    assert_eq!(api_started_event["payload"]["task_type"], "api-call");
    assert_eq!(
        api_started_event["payload"]["opencode_session_id"],
        "mock-session-web"
    );
    assert!(!api_started_event["payload"]["request_preview"]
        .as_str()
        .unwrap_or("")
        .is_empty());

    let upstream_event = timeline
        .iter()
        .find(|entry| entry["event"] == "upstream_response_observed")
        .expect("expected upstream_response_observed event");
    assert_eq!(upstream_event["payload"]["task_type"], "api-call");
    assert_eq!(upstream_event["payload"]["provider"], "opencode");
    assert_eq!(upstream_event["payload"]["operation"], "send_prompt");
    assert_eq!(
        upstream_event["payload"]["opencode_session_id"],
        "mock-session-web"
    );
    assert!(
        upstream_event["payload"]["worker_id"].is_string()
            || upstream_event["payload"]["worker_id"].is_null()
    );
    assert!(upstream_event["payload"]["response_preview"]
        .as_str()
        .unwrap_or("")
        .contains("reply[mock-session-web]"));
    assert!(upstream_event["payload"]["raw_result"]["content"]
        .as_str()
        .map(|content| !content.is_empty())
        .unwrap_or(true));

    let summaries = payload["recent_execution_summaries"].as_array().unwrap();
    let summary = summaries
        .iter()
        .rev()
        .find(|entry| entry["chat_id"] == "web-session")
        .expect("expected runtime summary for web-session");
    assert_eq!(summary["plan_type"], "single");
    assert_eq!(summary["status"], "success");
    assert_eq!(summary["cache_hit"], false);
    assert!(
        summary["stages"]["total_ms"].as_u64().unwrap()
            >= summary["stages"]["execute_ms"].as_u64().unwrap()
    );

    let jobs = payload["jobs"].as_array().unwrap();
    assert!(
        has_message_completed || jobs.iter().any(|job| job["status"] == "completed"),
        "expected completed timeline event or completed job projection"
    );
    let job = jobs
        .iter()
        .find(|job| job["chat_id"] == "web-session")
        .expect("expected web-session job");
    assert_eq!(job["status"], "completed");
    assert_eq!(job["kind"], "main");
    let unified_status = payload["unified_runtime"]["job_statuses"]
        .as_object()
        .unwrap()
        .values()
        .find(|entry| entry["status_label"] == "completed")
        .expect("expected completed unified runtime job status");
    assert_eq!(unified_status["status_label"], "completed");
    assert!(job["parent_job_id"].is_null());
    assert_eq!(job["chat_id"], "web-session");
    assert!(job["user_content"].is_null());
    assert!(job["elapsed_ms"].as_u64().is_some());
    let recent_events = job["recent_events"].as_array().unwrap();
    assert!(!recent_events.is_empty());
    let recent_upstream = recent_events
        .iter()
        .find(|event| event["event"] == "upstream_response_observed")
        .expect("expected upstream response in recent events");
    let unified_execution = payload["unified_runtime"]["execution_summaries"]
        .as_object()
        .unwrap()
        .values()
        .find(|entry| entry["plan_type"] == "single" && entry["status"] == "success")
        .expect("expected unified runtime execution summary");
    assert_eq!(unified_execution["plan_type"], "single");
    assert_eq!(unified_execution["status"], "success");
    assert_eq!(recent_upstream["payload"]["task_type"], "api-call");
    assert_eq!(recent_upstream["payload"]["provider"], "opencode");
    assert_eq!(recent_upstream["payload"]["operation"], "send_prompt");
    assert!(recent_upstream["payload"]["response_preview"]
        .as_str()
        .unwrap_or("")
        .contains("reply[mock-session-web]"));
    assert!(recent_upstream["payload"]["raw_result"]["content"]
        .as_str()
        .map(|content| !content.is_empty())
        .unwrap_or(true));

    state.runtime.lock().stop();
}

#[test]
fn status_api_exposes_failure_snapshot_and_counts() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(FailingExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    bus.publish_inbound(make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: Some("web-failure".into()),
        content: "please fail".into(),
        session_key: Some("web-failure".into()),
        ..Default::default()
    }))
    .unwrap();

    let state = build_state_with_runtime(runtime, bus, web_channel);

    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let mut payload: Option<Value> = None;
    for _ in 0..40 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let current_payload: Value = serde_json::from_slice(&body).unwrap();
        if current_payload["failures"]["last_failure"].is_object() {
            payload = Some(current_payload);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let payload = payload.expect("expected failure snapshot to appear");

    assert_eq!(
        payload["failures"]["last_failure"]["error_code"],
        "task_execution_failed"
    );
    assert_eq!(
        payload["failures"]["last_failure"]["error_stage"],
        "plan_execute"
    );
    assert_eq!(payload["failures"]["last_failure"]["channel"], "web");
    assert_eq!(
        payload["failures"]["last_failure"]["chat_id"],
        "web-failure"
    );
    assert!(payload["failures"]["last_failure"]["internal_message"]
        .as_str()
        .map(|message| !message.is_empty())
        .unwrap_or(false));
    assert!(
        payload["failures"]["counts_by_error_code"]["task_execution_failed"]
            .as_u64()
            .unwrap_or_default()
            >= 1
    );
    let jobs = payload["jobs"].as_array().unwrap();
    let failed_job = jobs
        .iter()
        .find(|job| job["chat_id"] == "web-failure")
        .expect("expected failure job for web-failure");
    assert_eq!(failed_job["status"], "failed");
    assert_eq!(failed_job["failure"]["error_code"], "task_execution_failed");
    assert!(failed_job["pending_question"].is_null());

    state.runtime.lock().stop();
}

#[test]
fn question_answer_api_rejects_job_without_real_pending_question() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(FailingExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    bus.publish_inbound(make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: Some("web-question-answer-failure".into()),
        content: "please fail".into(),
        session_key: Some("web-question-answer-failure".into()),
        ..Default::default()
    }))
    .unwrap();

    let state = build_state_with_runtime(runtime, bus, web_channel);
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();

    let mut failed_job_id: Option<String> = None;
    for _ in 0..40 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let jobs = payload["jobs"].as_array().unwrap();
        if let Some(job) = jobs.iter().find(|job| job["status"] == "failed") {
            failed_job_id = job["job_id"].as_str().map(ToString::to_string);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let job_id = failed_job_id.expect("expected failed job id");
    let response = tokio_rt
        .block_on(async {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/jobs/{job_id}/question-answer"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"question_id":"fake-question","source":"user","answer_type":"text","answer":"继续"}"#))
                    .unwrap(),
            )
            .await
        })
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio_rt
        .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["ok"], false);
    assert!(payload["error"]
        .as_str()
        .unwrap_or("")
        .contains("No pending question"));

    state.runtime.lock().stop();
}

#[test]
fn sessions_api_lists_history_and_send_alias_reuses_existing_session() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(MockExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    let state = build_state_with_runtime(runtime, bus.clone(), web_channel);
    {
        let manager = &state.runtime.lock().agent.session_manager;
        let session = manager.create_session(
            "web",
            anima_runtime::channel::session::SessionCreateOptions {
                id: Some("session-api-1".into()),
                ..Default::default()
            },
        );
        manager.add_to_history(
            &session.id,
            json!({
                "role": "user",
                "content": "hello from history",
                "recorded_at_ms": 100
            }),
        );
        manager.add_to_history(
            &session.id,
            json!({
                "role": "assistant",
                "content": "history reply",
                "recorded_at_ms": 101
            }),
        );
    }
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();

    let sessions_response = tokio_rt
        .block_on(async {
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/sessions")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
        })
        .unwrap();
    assert_eq!(sessions_response.status(), StatusCode::OK);
    let sessions_body = tokio_rt
        .block_on(async { axum::body::to_bytes(sessions_response.into_body(), usize::MAX).await })
        .unwrap();
    let sessions_payload: Value = serde_json::from_slice(&sessions_body).unwrap();
    let session_row = sessions_payload["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["session_id"] == "session-api-1")
        .cloned()
        .expect("expected session row");
    assert_eq!(session_row["chat_id"], "session-api-1");
    assert_eq!(session_row["channel"], "web");
    assert_eq!(session_row["history_len"], 2);
    assert_eq!(
        session_row["last_user_message_preview"],
        "hello from history"
    );
    assert!(session_row["last_active"].as_u64().unwrap() > 0);

    let history_response = tokio_rt
        .block_on(async {
            app.clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/sessions/session-api-1/history")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
        })
        .unwrap();
    assert_eq!(history_response.status(), StatusCode::OK);
    let history_body = tokio_rt
        .block_on(async { axum::body::to_bytes(history_response.into_body(), usize::MAX).await })
        .unwrap();
    let history_payload: Value = serde_json::from_slice(&history_body).unwrap();
    assert_eq!(history_payload["ok"], true);
    assert_eq!(history_payload["session_id"], "session-api-1");
    assert_eq!(history_payload["history"][0]["role"], "user");
    assert_eq!(
        history_payload["history"][0]["content"],
        "hello from history"
    );
    assert_eq!(history_payload["history"][0]["recorded_at"], 100);
    assert_eq!(history_payload["history"][1]["role"], "assistant");

    let send_response = tokio_rt
        .block_on(async {
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/sessions/session-api-1/send")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"content":"continue here"}"#))
                        .unwrap(),
                )
                .await
        })
        .unwrap();
    assert_eq!(send_response.status(), StatusCode::OK);
    let send_body = tokio_rt
        .block_on(async { axum::body::to_bytes(send_response.into_body(), usize::MAX).await })
        .unwrap();
    let send_payload: Value = serde_json::from_slice(&send_body).unwrap();
    assert_eq!(send_payload["accepted"], true);
    assert_eq!(send_payload["chat_id"], "session-api-1");
    assert_eq!(send_payload["session_id"], "session-api-1");
    assert!(send_payload["job_id"].as_str().is_some());

    state.runtime.lock().stop();
}

#[test]
fn status_api_includes_accepted_job_before_runtime_events() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let runtime = RuntimeBootstrapBuilder::new().with_cli_enabled(false).build();
    let bus = runtime.bus.clone();

    let mut store = anima_web::jobs::JobStore::default();
    store.register_accepted_job(anima_web::jobs::AcceptedJob {
        job_id: "job-queued-status".into(),
        trace_id: "trace-queued-status".into(),
        message_id: "job-queued-status".into(),
        kind: JobKind::Main,
        parent_job_id: None,
        channel: "web".into(),
        chat_id: Some("queued-chat".into()),
        sender_id: "web-user".into(),
        user_content: "queued message".into(),
        accepted_at_ms: anima_runtime::support::now_ms().saturating_sub(100),
    });

    let state = build_state_with_store(runtime, bus, web_channel, store);
    let app = routes::create_routes().with_state(state);
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let response = tokio_rt
        .block_on(async {
            app.oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
        })
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio_rt
        .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();

    let jobs = payload["jobs"].as_array().expect("expected jobs array");
    let job = jobs
        .iter()
        .find(|job| job["job_id"] == "job-queued-status")
        .expect("expected accepted queued job in status response");
    assert_eq!(job["status"], "queued");
    assert_eq!(job["kind"], "main");
    assert_eq!(job["chat_id"], "queued-chat");
    assert_eq!(job["accepted"], true);
    assert!(job["pending_question"].is_null());
}

#[test]
fn review_api_rejects_unknown_job() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let runtime = RuntimeBootstrapBuilder::new().with_cli_enabled(false).build();
    let bus = runtime.bus.clone();
    let state = build_state_with_runtime(runtime, bus, web_channel);
    let app = routes::create_routes().with_state(state);
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();

    let response = tokio_rt
        .block_on(async {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/jobs/job-missing/review")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"user_verdict":"accepted"}"#))
                    .unwrap(),
            )
            .await
        })
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio_rt
        .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["job_id"], "job-missing");
    assert_eq!(payload["error"], "job_not_found");

    assert!(payload.get("review").is_none() || payload["review"].is_null());
    assert!(payload.get("job").is_none() || payload["job"].is_null());
}

#[test]
fn send_api_returns_job_id_and_review_records_feedback_only() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(MockExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    let state = build_state_with_runtime(runtime, bus.clone(), web_channel);
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();

    let send_response = tokio_rt
        .block_on(async {
            app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/send")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            r#"{"content":"review me","session_id":"review-chat"}"#,
                        ))
                        .unwrap(),
                )
                .await
        })
        .unwrap();
    assert_eq!(send_response.status(), StatusCode::OK);
    let send_body = tokio_rt
        .block_on(async { axum::body::to_bytes(send_response.into_body(), usize::MAX).await })
        .unwrap();
    let send_payload: Value = serde_json::from_slice(&send_body).unwrap();
    let job_id = send_payload["job_id"].as_str().unwrap().to_string();
    assert_eq!(send_payload["accepted"], true);
    assert_eq!(send_payload["chat_id"], "review-chat");

    let mut saw_completed = false;
    for _ in 0..120 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let jobs = payload["jobs"].as_array().unwrap();
        if jobs.iter().any(|job| {
            job["job_id"] == job_id
                && (job["status"] == "completed"
                    || job["recent_events"]
                        .as_array()
                        .map(|events| {
                            events
                                .iter()
                                .any(|event| event["event"] == "message_completed")
                        })
                        .unwrap_or(false))
        }) {
            saw_completed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(75));
    }
    if !saw_completed {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let jobs_debug = payload["jobs"].clone();
        panic!("expected job to reach completed, latest jobs: {jobs_debug}");
    }

    let review_response = tokio_rt
        .block_on(async {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/jobs/{job_id}/review"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"user_verdict":"accepted"}"#))
                    .unwrap(),
            )
            .await
        })
        .unwrap();
    assert_eq!(review_response.status(), StatusCode::OK);
    let review_body = tokio_rt
        .block_on(async { axum::body::to_bytes(review_response.into_body(), usize::MAX).await })
        .unwrap();
    let review_payload: Value = serde_json::from_slice(&review_body).unwrap();
    assert_eq!(review_payload["review"]["verdict"], "accepted");
    assert_eq!(review_payload["job"]["status"], "completed");
    assert_eq!(review_payload["job"]["kind"], "main");
    assert!(review_payload["job"]["parent_job_id"].is_null());
    assert_eq!(review_payload["job"]["review"]["verdict"], "accepted");

    state.runtime.lock().stop();
}

#[test]
fn status_api_exposes_orchestration_p2_question_observability() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(OrchestrationQuestionExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    bus.publish_inbound(make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: Some("web-orch-question".into()),
        content: "create REST API endpoint".into(),
        session_key: Some("web-orch-question".into()),
        ..Default::default()
    }))
    .unwrap();

    let state = build_state_with_runtime(runtime, bus, web_channel);
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let mut payload: Option<Value> = None;
    for _ in 0..120 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let current_payload: Value = serde_json::from_slice(&body).unwrap();
        let events = current_payload["runtime_timeline"]
            .as_array()
            .map(|timeline| {
                timeline
                    .iter()
                    .map(|entry| entry["event"].as_str().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let has_waiting_question_job = current_payload["jobs"]
            .as_array()
            .map(|jobs| {
                jobs.iter().any(|job| {
                    job["status"] == "waiting_user_input"
                        && job["pending_question"].is_object()
                })
            })
            .unwrap_or(false);
        if has_waiting_question_job
            && events.contains(&"orchestration_selected")
            && events.contains(&"orchestration_plan_created")
            && events.contains(&"orchestration_subtask_started")
            && events.contains(&"orchestration_subtask_completed")
        {
            payload = Some(current_payload);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(75));
    }
    let payload = payload.expect("expected orchestration question payload");

    let timeline = payload["runtime_timeline"].as_array().unwrap();
    let events = timeline
        .iter()
        .map(|entry| entry["event"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(events.contains(&"orchestration_selected"));
    assert!(events.contains(&"orchestration_plan_created"));
    assert!(events.contains(&"orchestration_subtask_started"));
    assert!(events.contains(&"orchestration_subtask_completed"));
    assert!(
        events.contains(&"question_asked")
            || payload["jobs"]
                .as_array()
                .map(|jobs| {
                    jobs.iter().any(|job| {
                        job["status"] == "waiting_user_input" && job["pending_question"].is_object()
                    })
                })
                .unwrap_or(false)
    );

    assert!(!events.contains(&"orchestration_fallback"));

    let orch_started = timeline
        .iter()
        .find(|entry| entry["event"] == "orchestration_subtask_started")
        .expect("expected orchestration_subtask_started event");
    assert_eq!(orch_started["payload"]["original_task_type"], "design");
    assert_eq!(orch_started["payload"]["lowered_task_type"], "api-call");
    assert_eq!(orch_started["payload"]["execution_mode"], "serial");
    assert_eq!(orch_started["payload"]["result_kind"], "upstream");
    assert_eq!(orch_started["payload"]["parallel_safe"], false);

    let job = payload["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["chat_id"] == "web-orch-question")
        .expect("expected orchestration question job");
    assert_eq!(job["status"], "waiting_user_input");
    assert_eq!(job["kind"], "main");
    assert!(job["pending_question"].is_object());
    assert_eq!(
        job["pending_question"]["question_id"],
        "status-orch-question-1"
    );
    assert_eq!(
        job["pending_question"]["prompt"],
        "需要确认 status orchestration 是否继续"
    );
    assert!(payload["unified_runtime"]["pending_questions"]
        .as_object()
        .unwrap()
        .contains_key(job["job_id"].as_str().unwrap()));
    assert!(payload["unified_runtime"]["orchestration"]
        .as_object()
        .unwrap()
        .contains_key(job["job_id"].as_str().unwrap()));
    assert_eq!(
        job["orchestration"]["plan_id"].as_str().unwrap_or(""),
        job["recent_events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["event"] == "orchestration_plan_created")
            .and_then(|event| event["payload"]["plan_id"].as_str())
            .unwrap_or("")
    );
    assert!(job["orchestration"].is_object());
    assert!(
        job["orchestration"]["plan_id"].is_string() || job["orchestration"]["plan_id"].is_null()
    );

    state.runtime.lock().stop();
}

#[test]
fn status_api_exposes_orchestration_p2_followup_observability() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(OrchestrationFollowupExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    bus.publish_inbound(make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: Some("web-orch-followup".into()),
        content: "create REST API endpoint".into(),
        session_key: Some("web-orch-followup".into()),
        ..Default::default()
    }))
    .unwrap();

    let state = build_state_with_runtime(runtime, bus, web_channel);
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let mut payload: Option<Value> = None;
    for _ in 0..40 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let current_payload: Value = serde_json::from_slice(&body).unwrap();
        let events = current_payload["runtime_timeline"]
            .as_array()
            .map(|timeline| {
                timeline
                    .iter()
                    .map(|entry| entry["event"].as_str().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if events.contains(&"requirement_followup_scheduled")
            && events.contains(&"message_completed")
        {
            payload = Some(current_payload);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let payload = payload.expect("expected orchestration followup payload");

    let timeline = payload["runtime_timeline"].as_array().unwrap();
    let events = timeline
        .iter()
        .map(|entry| entry["event"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(events.contains(&"orchestration_subtask_completed"));
    assert!(events.contains(&"requirement_unsatisfied"));
    assert!(events.contains(&"requirement_followup_scheduled"));
    assert!(events.contains(&"requirement_satisfied"));
    assert!(events.contains(&"message_completed"));
    assert!(!events.contains(&"orchestration_fallback"));

    let summaries = payload["recent_execution_summaries"].as_array().unwrap();
    let summary = summaries
        .iter()
        .rev()
        .find(|entry| entry["chat_id"] == "web-orch-followup")
        .expect("expected orchestration followup summary");
    assert_eq!(summary["status"], "success");

    let orch_completed = timeline
        .iter()
        .find(|entry| {
            entry["event"] == "orchestration_subtask_completed"
                && entry["payload"]["execution_mode"] == "serial"
                && entry["payload"]["result_kind"] == "upstream"
        })
        .expect("expected orchestration_subtask_completed in runtime timeline");
    assert_eq!(orch_completed["payload"]["lowered_task_type"], "api-call");

    let job = payload["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["status"] == "completed")
        .expect("expected completed followup job");
    assert_eq!(job["status"], "completed");
    assert!(job["pending_question"].is_null());

    state.runtime.lock().stop();
}

#[test]
fn status_api_exposes_orchestration_fallback_observability() {
    let web_channel = Arc::new(web_channel::WebChannel::new());
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(Arc::new(MockExecutor))
        .build();
    runtime
        .registry
        .register(web_channel.clone() as Arc<dyn Channel>, None);
    let bus = runtime.bus.clone();
    runtime.start();

    bus.publish_inbound(make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: Some("web-orch-fallback".into()),
        content: "[orchestration-fail] create REST API endpoint".into(),
        session_key: Some("web-orch-fallback".into()),
        ..Default::default()
    }))
    .unwrap();

    let state = build_state_with_runtime(runtime, bus, web_channel);
    let app = routes::create_routes().with_state(state.clone());
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let mut payload: Option<Value> = None;
    for _ in 0..120 {
        let response = tokio_rt
            .block_on(async {
                app.clone()
                    .oneshot(
                        Request::builder()
                            .uri("/api/status")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();
        let body = tokio_rt
            .block_on(async { axum::body::to_bytes(response.into_body(), usize::MAX).await })
            .unwrap();
        let current_payload: Value = serde_json::from_slice(&body).unwrap();
        let events = current_payload["runtime_timeline"]
            .as_array()
            .map(|timeline| {
                timeline
                    .iter()
                    .map(|entry| entry["event"].as_str().unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let fallback_job_completed = current_payload["jobs"]
            .as_array()
            .map(|jobs| {
                jobs.iter().any(|job| {
                    job["chat_id"] == "web-orch-fallback" && job["status"] == "completed"
                })
            })
            .unwrap_or(false);
        if events.contains(&"orchestration_fallback") && fallback_job_completed {
            payload = Some(current_payload);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(75));
    }
    let payload = payload.expect("expected orchestration fallback payload");

    let timeline = payload["runtime_timeline"].as_array().unwrap();
    let events = timeline
        .iter()
        .map(|entry| entry["event"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(events.contains(&"orchestration_selected"));
    assert!(events.contains(&"orchestration_fallback"));
    assert!(events.contains(&"worker_task_assigned"));
    assert!(events.contains(&"api_call_started"));
    assert!(!events.contains(&"orchestration_plan_created"));

    let fallback_event = timeline
        .iter()
        .find(|entry| entry["event"] == "orchestration_fallback")
        .expect("expected orchestration_fallback event");
    assert_eq!(fallback_event["payload"]["plan_type"], "orchestration-v1");
    assert_eq!(fallback_event["payload"]["fallback_plan_type"], "single");
    assert_eq!(
        fallback_event["payload"]["reason"],
        "forced orchestration fallback"
    );

    let summaries = payload["recent_execution_summaries"].as_array().unwrap();
    if let Some(summary) = summaries
        .iter()
        .rev()
        .find(|entry| entry["chat_id"] == "web-orch-fallback")
    {
        assert_eq!(summary["plan_type"], "orchestration-v1");
        assert_eq!(summary["status"], "success");
    }

    let job = payload["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["chat_id"] == "web-orch-fallback")
        .expect("expected fallback job");
    assert_eq!(job["status"], "completed");
    assert_eq!(job["kind"], "main");
    assert!(job["pending_question"].is_null());
    assert!(job["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event"] == "upstream_response_observed"));
    assert!(timeline
        .iter()
        .any(|entry| entry["event"] == "orchestration_fallback"));

    state.runtime.lock().stop();
}
