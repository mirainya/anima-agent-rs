use anima_runtime::agent::{ExecutionStageDurations, ExecutionSummary, PendingQuestion, QuestionDecisionMode, QuestionKind, QuestionRiskLevel};
use anima_runtime::context_assembly::{
    assemble_context, ContextAssemblyMode, ContextAssemblyRequest,
};
use serde_json::json;

fn make_pending_question() -> PendingQuestion {
    PendingQuestion {
        question_id: "question-1".into(),
        job_id: "job-1".into(),
        opencode_session_id: "session-1".into(),
        question_kind: QuestionKind::Input,
        prompt: "请选择继续方式".into(),
        options: vec!["继续执行".into(), "取消".into()],
        raw_question: json!({
            "prompt": "请选择继续方式",
            "options": ["继续执行", "取消"]
        }),
        decision_mode: QuestionDecisionMode::UserRequired,
        risk_level: QuestionRiskLevel::High,
        requires_user_confirmation: true,
        asked_at_ms: 1,
        answer_submitted: true,
        answer_summary: Some("继续执行".into()),
        resolution_source: Some("user".into()),
        inbound: None,
    }
}

fn make_summary(status: &str) -> ExecutionSummary {
    ExecutionSummary {
        trace_id: "trace-1".into(),
        message_id: "job-1".into(),
        channel: "web".into(),
        chat_id: Some("chat-1".into()),
        plan_type: "single".into(),
        status: status.into(),
        cache_hit: false,
        worker_id: None,
        error_code: None,
        error_stage: None,
        task_duration_ms: 10,
        stages: ExecutionStageDurations {
            context_ms: 1,
            session_ms: 2,
            classify_ms: 3,
            execute_ms: 4,
            total_ms: 10,
        },
    }
}

#[test]
fn initial_context_assembly_uses_chat_id_for_memory_and_history_keys() {
    let assembled = assemble_context(ContextAssemblyRequest {
        mode: ContextAssemblyMode::Initial,
        inbound_id: "job-1".into(),
        channel: "web".into(),
        chat_id: Some("chat-1".into()),
        original_user_request: "请继续处理部署任务".into(),
        opencode_session_id: "session-1".into(),
        pending_question: None,
        latest_summary: Some(make_summary("followup_pending")),
        attempted_rounds: 1,
        max_rounds: 3,
        previous_fingerprint: Some("fp-1".into()),
        followup_prompt: None,
    });

    assert_eq!(assembled.metadata.memory_key, "web:chat-1");
    assert_eq!(assembled.metadata.history_session_id, "chat-1");
    assert_eq!(assembled.metadata.opencode_session_id, "session-1");
    assert_eq!(assembled.prompt_text, "请继续处理部署任务");
    assert_eq!(assembled.requirement_snapshot.latest_summary_status.as_deref(), Some("followup_pending"));
}

#[test]
fn question_continuation_context_assembly_keeps_legacy_answer_summary_prompt() {
    let assembled = assemble_context(ContextAssemblyRequest {
        mode: ContextAssemblyMode::QuestionContinuation,
        inbound_id: "job-1".into(),
        channel: "test".into(),
        chat_id: Some("chat-question".into()),
        original_user_request: "Need continuation".into(),
        opencode_session_id: "session-question".into(),
        pending_question: Some(make_pending_question()),
        latest_summary: None,
        attempted_rounds: 0,
        max_rounds: 3,
        previous_fingerprint: None,
        followup_prompt: None,
    });

    assert_eq!(assembled.prompt_text, "继续执行");
    assert_eq!(assembled.requirement_snapshot.pending_question_prompt.as_deref(), Some("请选择继续方式"));
    assert_eq!(assembled.requirement_snapshot.pending_question_answer_summary.as_deref(), Some("继续执行"));
}

#[test]
fn followup_context_assembly_prefers_explicit_followup_prompt() {
    let assembled = assemble_context(ContextAssemblyRequest {
        mode: ContextAssemblyMode::Followup,
        inbound_id: "job-2".into(),
        channel: "test".into(),
        chat_id: None,
        original_user_request: "Need better completion".into(),
        opencode_session_id: "session-followup".into(),
        pending_question: None,
        latest_summary: None,
        attempted_rounds: 2,
        max_rounds: 3,
        previous_fingerprint: Some("fp-repeat".into()),
        followup_prompt: Some("请继续推进并给出真正完成结果".into()),
    });

    assert_eq!(assembled.metadata.memory_key, "test:job-2");
    assert_eq!(assembled.metadata.history_session_id, "job-2");
    assert_eq!(assembled.prompt_text, "请继续推进并给出真正完成结果");
    assert_eq!(assembled.requirement_snapshot.attempted_rounds, 2);
    assert_eq!(assembled.requirement_snapshot.previous_fingerprint.as_deref(), Some("fp-repeat"));
}
