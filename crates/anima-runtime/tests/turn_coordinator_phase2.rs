use anima_runtime::agent::{
    ExecutionStageDurations, ExecutionSummary, PendingQuestion, PendingQuestionSourceKind,
    QuestionDecisionMode, QuestionKind, QuestionRiskLevel,
};
use anima_runtime::context_assembly::{
    assemble_context, ContextAssemblyMode, ContextAssemblyRequest,
};
use anima_runtime::requirement_judge::{
    AgentFollowupPlan, RequirementJudgement, UserInputRequirement,
};
use anima_runtime::turn_coordinator::{
    plan_turn_outcome, prepare_completion_data, prepare_followup_exhausted_payload,
    prepare_followup_scheduled_payload, prepare_message_completed_payload,
    prepare_question_asked_payload, prepare_requirement_evaluation,
    prepare_requirement_unsatisfied_event_payload, prepare_summary_input,
    prepare_user_input_required_payload, TurnOutcomePlan, TurnSource,
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
        source_kind: PendingQuestionSourceKind::UpstreamQuestion,
        continuation_token: None,
        asked_at_ms: 1,
        answer_submitted: false,
        answer_summary: None,
        resolution_source: None,
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
fn plan_turn_outcome_routes_all_three_main_paths() {
    let complete = plan_turn_outcome(RequirementJudgement::Satisfied, true);
    assert_eq!(
        complete,
        TurnOutcomePlan::Complete {
            should_resolve_question: true
        }
    );

    let ask = plan_turn_outcome(
        RequirementJudgement::NeedsUserInput(Box::new(UserInputRequirement {
            reason: "缺少用户输入".into(),
            missing_requirements: vec!["部署环境".into()],
            pending_question: make_pending_question(),
        })),
        false,
    );
    match ask {
        TurnOutcomePlan::AskUserInput {
            should_resolve_question,
            requirement,
        } => {
            assert!(!should_resolve_question);
            assert_eq!(requirement.reason, "缺少用户输入");
        }
        other => panic!("unexpected outcome: {other:?}"),
    }

    let followup = plan_turn_outcome(
        RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
            reason: "继续推进".into(),
            missing_requirements: vec!["补全最终结果".into()],
            followup_prompt: "继续执行".into(),
            result_fingerprint: "fp-1".into(),
        })),
        false,
    );
    match followup {
        TurnOutcomePlan::ScheduleFollowup { plan } => {
            assert_eq!(plan.followup_prompt, "继续执行");
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[test]
fn prepare_requirement_evaluation_preserves_source_label_and_snapshot_fields() {
    let assembly = assemble_context(ContextAssemblyRequest {
        mode: ContextAssemblyMode::QuestionContinuation,
        inbound_id: "job-1".into(),
        channel: "web".into(),
        chat_id: Some("chat-1".into()),
        original_user_request: "Need continuation".into(),
        opencode_session_id: "session-1".into(),
        pending_question: Some(make_pending_question()),
        latest_summary: Some(make_summary("waiting_user_input")),
        attempted_rounds: 2,
        max_rounds: 3,
        previous_fingerprint: Some("fp-prev".into()),
        followup_prompt: None,
    });

    let prepared = prepare_requirement_evaluation(
        TurnSource::QuestionContinuation,
        assembly,
        "job-1".into(),
        "trace-1".into(),
        Some("chat-1".into()),
        Some(json!({"content": "continued"})),
    );

    assert_eq!(prepared.source_label, "question_continuation");
    assert_eq!(prepared.judge_context.attempted_rounds, 2);
    assert_eq!(prepared.judge_context.max_rounds, 3);
    assert_eq!(
        prepared.judge_context.previous_fingerprint.as_deref(),
        Some("fp-prev")
    );
    assert_eq!(prepared.judge_context.opencode_session_id, "session-1");
}

#[test]
fn prepare_standardized_payloads_and_summaries_match_expected_shapes() {
    let completion = prepare_completion_data("final answer".into());
    assert_eq!(completion.summary_status, "success");
    assert_eq!(completion.assistant_entry["role"], "assistant");
    assert_eq!(completion.assistant_entry["content"], "final answer");

    let completed_payload = prepare_message_completed_payload("final answer");
    assert_eq!(completed_payload.status, "success");
    assert_eq!(completed_payload.response_text, "final answer");

    let summary = prepare_summary_input(
        "waiting_user_input".into(),
        Some("worker-1".into()),
        55,
        1,
        2,
        3,
        4,
        10,
    );
    assert_eq!(summary.status, "waiting_user_input");
    assert_eq!(summary.worker_id.as_deref(), Some("worker-1"));
    assert_eq!(summary.stages.total_ms, 10);

    let question_payload = prepare_question_asked_payload(&make_pending_question());
    assert_eq!(question_payload.question_id, "question-1");
    assert_eq!(question_payload.prompt, "请选择继续方式");
    assert!(question_payload.requires_user_confirmation);

    let user_input_requirement = UserInputRequirement {
        reason: "缺少用户输入".into(),
        missing_requirements: vec!["部署环境".into()],
        pending_question: make_pending_question(),
    };
    let user_input_payload = prepare_user_input_required_payload(&user_input_requirement);
    assert_eq!(user_input_payload.reason, "缺少用户输入");
    assert_eq!(user_input_payload.question_id, "question-1");

    let unsatisfied_payload = prepare_requirement_unsatisfied_event_payload(
        "尚未满足",
        &["补全最终结果".into()],
        Some(&json!({"content": "intermediate"})),
    );
    assert_eq!(unsatisfied_payload.payload["reason"], "尚未满足");

    let followup_scheduled = prepare_followup_scheduled_payload(
        2,
        3,
        "继续推进".into(),
        vec!["补全最终结果".into()],
        "继续执行".into(),
    );
    assert_eq!(followup_scheduled.attempted_rounds, 2);
    assert_eq!(followup_scheduled.followup_prompt, "继续执行");

    let followup_exhausted = prepare_followup_exhausted_payload(
        4,
        3,
        "重复输出".into(),
        vec!["避免重复".into()],
        "fp-repeat".into(),
    );
    assert_eq!(followup_exhausted.max_rounds, 3);
    assert_eq!(followup_exhausted.result_fingerprint, "fp-repeat");
}
