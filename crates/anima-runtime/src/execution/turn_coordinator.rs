use crate::agent::{ExecutionStageDurations, PendingQuestion};
use crate::bus::InboundMessage;
use crate::execution::context_assembly::{ContextAssemblyMode, ContextAssemblyResult};
use crate::execution::requirement_judge::{
    requirement_unsatisfied_payload, AgentFollowupPlan, RequirementJudgeContext,
    RequirementJudgement, UserInputRequirement,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSource {
    Initial,
    QuestionContinuation,
    AgentFollowup,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequirementEvaluationPreparation {
    pub source_label: &'static str,
    pub assembly_mode: ContextAssemblyMode,
    pub assembly: ContextAssemblyResult,
    pub judge_context: RequirementJudgeContext,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnOutcomePlan {
    Complete {
        should_resolve_question: bool,
    },
    AskUserInput {
        should_resolve_question: bool,
        requirement: Box<UserInputRequirement>,
    },
    ScheduleFollowup {
        plan: Box<AgentFollowupPlan>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionData {
    pub response_text: String,
    pub assistant_entry: Value,
    pub summary_status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaitingUserInputData {
    pub question: PendingQuestion,
    pub summary_status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummaryInput {
    pub status: String,
    pub worker_id: Option<String>,
    pub error_code: Option<String>,
    pub error_stage: Option<String>,
    pub task_duration_ms: u64,
    pub stages: ExecutionStageDurations,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageCompletedPayload {
    pub status: String,
    pub response_preview: String,
    pub response_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserInputRequiredPayload {
    pub reason: String,
    pub missing_requirements: Vec<String>,
    pub question_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuestionResolvedPayload {
    pub question_id: String,
    pub answer_summary: Option<String>,
    pub resolution_source: Option<String>,
    pub opencode_session_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequirementUnsatisfiedEventPayload {
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuestionAskedPayload {
    pub question_id: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub raw_question: Value,
    pub opencode_session_id: String,
    pub requires_user_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamResponseObservedPayload {
    pub worker_id: Option<String>,
    pub task_type: String,
    pub provider: Option<String>,
    pub operation: Option<String>,
    pub opencode_session_id: String,
    pub response_preview: String,
    pub raw_result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequirementEvaluationStartedPayload {
    pub source: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequirementSatisfiedPayload {
    pub response_preview: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FollowupScheduledPayload {
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub reason: String,
    pub missing_requirements: Vec<String>,
    pub followup_prompt: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FollowupExhaustedPayload {
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub reason: String,
    pub missing_requirements: Vec<String>,
    pub result_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedBranchData {
    pub resolved_question: Option<QuestionResolvedPayload>,
    pub completion: CompletionData,
    pub summary: SummaryInput,
    pub message_completed: MessageCompletedPayload,
    pub requirement_satisfied: RequirementSatisfiedPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaitingUserInputBranchData {
    pub resolved_question: Option<QuestionResolvedPayload>,
    pub unsatisfied: RequirementUnsatisfiedEventPayload,
    pub user_input_required: UserInputRequiredPayload,
    pub waiting: WaitingUserInputData,
    pub summary: SummaryInput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FollowupBranchData {
    pub unsatisfied: RequirementUnsatisfiedEventPayload,
    pub pending_summary: SummaryInput,
    pub exhausted_summary: SummaryInput,
    pub scheduled: FollowupScheduledPayload,
    pub exhausted: FollowupExhaustedPayload,
}

pub fn assembly_mode_for_source(source: TurnSource) -> ContextAssemblyMode {
    match source {
        TurnSource::Initial => ContextAssemblyMode::Initial,
        TurnSource::QuestionContinuation => ContextAssemblyMode::QuestionContinuation,
        TurnSource::AgentFollowup => ContextAssemblyMode::Followup,
    }
}

pub fn source_label(source: TurnSource) -> &'static str {
    match source {
        TurnSource::Initial => "initial",
        TurnSource::QuestionContinuation => "question_continuation",
        TurnSource::AgentFollowup => "agent_followup",
    }
}

pub fn build_requirement_judge_context(
    assembly: &ContextAssemblyResult,
    job_id: String,
    trace_id: String,
    chat_id: Option<String>,
    raw_result: Option<serde_json::Value>,
) -> RequirementJudgeContext {
    let snapshot = &assembly.requirement_snapshot;
    RequirementJudgeContext {
        original_user_request: snapshot.original_user_request.clone(),
        job_id,
        trace_id,
        chat_id,
        opencode_session_id: assembly.metadata.opencode_session_id.clone(),
        raw_result,
        attempted_rounds: snapshot.attempted_rounds,
        max_rounds: snapshot.max_rounds,
        previous_fingerprint: snapshot.previous_fingerprint.clone(),
    }
}

pub fn prepare_requirement_evaluation(
    source: TurnSource,
    assembly: ContextAssemblyResult,
    job_id: String,
    trace_id: String,
    chat_id: Option<String>,
    raw_result: Option<serde_json::Value>,
) -> RequirementEvaluationPreparation {
    RequirementEvaluationPreparation {
        source_label: source_label(source),
        assembly_mode: assembly_mode_for_source(source),
        judge_context: build_requirement_judge_context(
            &assembly, job_id, trace_id, chat_id, raw_result,
        ),
        assembly,
    }
}

pub fn plan_turn_outcome(
    judgement: RequirementJudgement,
    has_resolved_question: bool,
) -> TurnOutcomePlan {
    match judgement {
        RequirementJudgement::Satisfied => TurnOutcomePlan::Complete {
            should_resolve_question: has_resolved_question,
        },
        RequirementJudgement::NeedsUserInput(requirement) => TurnOutcomePlan::AskUserInput {
            should_resolve_question: has_resolved_question,
            requirement,
        },
        RequirementJudgement::NeedsAgentFollowup(plan) => {
            TurnOutcomePlan::ScheduleFollowup { plan }
        }
    }
}

pub fn resolved_question_to_publish(
    should_resolve_question: bool,
    resolved_question: Option<&PendingQuestion>,
) -> Option<&PendingQuestion> {
    if should_resolve_question {
        resolved_question
    } else {
        None
    }
}

pub fn prepare_pending_question_for_storage(
    mut question: PendingQuestion,
    inbound: &InboundMessage,
) -> PendingQuestion {
    question.job_id = inbound.id.clone();
    question.inbound = Some(inbound.clone());
    question
}

pub fn prepare_completion_data(response_text: String) -> CompletionData {
    CompletionData {
        assistant_entry: json!({"role": "assistant", "content": response_text.clone()}),
        response_text,
        summary_status: "success".into(),
    }
}

pub fn prepare_waiting_user_input_data(
    question: PendingQuestion,
    inbound: &InboundMessage,
) -> WaitingUserInputData {
    WaitingUserInputData {
        question: prepare_pending_question_for_storage(question, inbound),
        summary_status: "waiting_user_input".into(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_summary_input(
    status: String,
    worker_id: Option<String>,
    task_duration_ms: u64,
    context_ms: u64,
    session_ms: u64,
    classify_ms: u64,
    execute_ms: u64,
    total_ms: u64,
) -> SummaryInput {
    SummaryInput {
        status,
        worker_id,
        error_code: None,
        error_stage: None,
        task_duration_ms,
        stages: ExecutionStageDurations {
            context_ms,
            session_ms,
            classify_ms,
            execute_ms,
            total_ms,
        },
    }
}

pub fn prepare_message_completed_payload(response_text: &str) -> MessageCompletedPayload {
    let response_preview = if response_text.chars().count() > 120 {
        format!("{}…", response_text.chars().take(120).collect::<String>())
    } else {
        response_text.to_string()
    };
    MessageCompletedPayload {
        status: "success".into(),
        response_preview,
        response_text: response_text.to_string(),
    }
}

pub fn prepare_user_input_required_payload(
    requirement: &UserInputRequirement,
) -> UserInputRequiredPayload {
    UserInputRequiredPayload {
        reason: requirement.reason.clone(),
        missing_requirements: requirement.missing_requirements.clone(),
        question_id: requirement.pending_question.question_id.clone(),
        prompt: requirement.pending_question.prompt.clone(),
    }
}

pub fn prepare_question_resolved_payload(question: &PendingQuestion) -> QuestionResolvedPayload {
    QuestionResolvedPayload {
        question_id: question.question_id.clone(),
        answer_summary: question.answer_summary.clone(),
        resolution_source: question.resolution_source.clone(),
        opencode_session_id: question.opencode_session_id.clone(),
    }
}

pub fn prepare_requirement_unsatisfied_event_payload(
    reason: &str,
    missing_requirements: &[String],
    raw_result: Option<&Value>,
) -> RequirementUnsatisfiedEventPayload {
    RequirementUnsatisfiedEventPayload {
        payload: requirement_unsatisfied_payload(reason, missing_requirements, raw_result),
    }
}

pub fn prepare_question_asked_payload(question: &PendingQuestion) -> QuestionAskedPayload {
    QuestionAskedPayload {
        question_id: question.question_id.clone(),
        prompt: question.prompt.clone(),
        options: question.options.clone(),
        raw_question: question.raw_question.clone(),
        opencode_session_id: question.opencode_session_id.clone(),
        requires_user_confirmation: question.requires_user_confirmation,
    }
}

pub fn prepare_upstream_response_observed_payload(
    worker_id: Option<String>,
    task_type: String,
    provider: Option<String>,
    operation: Option<String>,
    opencode_session_id: String,
    response_preview: String,
    raw_result: Option<Value>,
) -> UpstreamResponseObservedPayload {
    UpstreamResponseObservedPayload {
        worker_id,
        task_type,
        provider,
        operation,
        opencode_session_id,
        response_preview,
        raw_result,
    }
}

pub fn prepare_requirement_evaluation_started_payload(
    source: &'static str,
) -> RequirementEvaluationStartedPayload {
    RequirementEvaluationStartedPayload { source }
}

pub fn prepare_requirement_satisfied_payload(response_text: &str) -> RequirementSatisfiedPayload {
    let response_preview = if response_text.chars().count() > 120 {
        format!("{}…", response_text.chars().take(120).collect::<String>())
    } else {
        response_text.to_string()
    };
    RequirementSatisfiedPayload { response_preview }
}

pub fn prepare_followup_scheduled_payload(
    attempted_rounds: usize,
    max_rounds: usize,
    reason: String,
    missing_requirements: Vec<String>,
    followup_prompt: String,
) -> FollowupScheduledPayload {
    FollowupScheduledPayload {
        attempted_rounds,
        max_rounds,
        reason,
        missing_requirements,
        followup_prompt,
    }
}

pub fn prepare_followup_exhausted_payload(
    attempted_rounds: usize,
    max_rounds: usize,
    reason: String,
    missing_requirements: Vec<String>,
    result_fingerprint: String,
) -> FollowupExhaustedPayload {
    FollowupExhaustedPayload {
        attempted_rounds,
        max_rounds,
        reason,
        missing_requirements,
        result_fingerprint,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_completed_branch_data(
    resolved_question: Option<&PendingQuestion>,
    should_resolve_question: bool,
    response_text: String,
    worker_id: Option<String>,
    task_duration_ms: u64,
    context_ms: u64,
    session_ms: u64,
    classify_ms: u64,
    execute_ms: u64,
    total_ms: u64,
) -> CompletedBranchData {
    let completion = prepare_completion_data(response_text.clone());
    CompletedBranchData {
        resolved_question: resolved_question_to_publish(should_resolve_question, resolved_question)
            .map(prepare_question_resolved_payload),
        summary: prepare_summary_input(
            completion.summary_status.clone(),
            worker_id,
            task_duration_ms,
            context_ms,
            session_ms,
            classify_ms,
            execute_ms,
            total_ms,
        ),
        message_completed: prepare_message_completed_payload(&response_text),
        requirement_satisfied: prepare_requirement_satisfied_payload(&response_text),
        completion,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_waiting_user_input_branch_data(
    resolved_question: Option<&PendingQuestion>,
    should_resolve_question: bool,
    requirement: UserInputRequirement,
    inbound: &InboundMessage,
    worker_id: Option<String>,
    task_duration_ms: u64,
    context_ms: u64,
    session_ms: u64,
    classify_ms: u64,
    execute_ms: u64,
    total_ms: u64,
    raw_result: Option<&Value>,
) -> WaitingUserInputBranchData {
    let unsatisfied = prepare_requirement_unsatisfied_event_payload(
        &requirement.reason,
        &requirement.missing_requirements,
        raw_result,
    );
    let user_input_required = prepare_user_input_required_payload(&requirement);
    let waiting = prepare_waiting_user_input_data(requirement.pending_question, inbound);
    let summary = prepare_summary_input(
        waiting.summary_status.clone(),
        worker_id,
        task_duration_ms,
        context_ms,
        session_ms,
        classify_ms,
        execute_ms,
        total_ms,
    );
    WaitingUserInputBranchData {
        resolved_question: resolved_question_to_publish(should_resolve_question, resolved_question)
            .map(prepare_question_resolved_payload),
        unsatisfied,
        user_input_required,
        waiting,
        summary,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_followup_branch_data(
    reason: String,
    missing_requirements: Vec<String>,
    followup_prompt: String,
    result_fingerprint: String,
    attempted_rounds: usize,
    max_rounds: usize,
    worker_id: Option<String>,
    task_duration_ms: u64,
    context_ms: u64,
    session_ms: u64,
    classify_ms: u64,
    execute_ms: u64,
    total_ms: u64,
) -> FollowupBranchData {
    let unsatisfied =
        prepare_requirement_unsatisfied_event_payload(&reason, &missing_requirements, None);
    let pending_summary = prepare_summary_input(
        "followup_pending".into(),
        worker_id.clone(),
        task_duration_ms,
        context_ms,
        session_ms,
        classify_ms,
        execute_ms,
        total_ms,
    );
    let exhausted_summary = SummaryInput {
        status: "followup_exhausted".into(),
        worker_id,
        error_code: Some("requirement_followup_exhausted".into()),
        error_stage: Some("requirement_judge".into()),
        task_duration_ms,
        stages: ExecutionStageDurations {
            context_ms,
            session_ms,
            classify_ms,
            execute_ms,
            total_ms,
        },
    };
    let scheduled = prepare_followup_scheduled_payload(
        attempted_rounds,
        max_rounds,
        reason.clone(),
        missing_requirements.clone(),
        followup_prompt,
    );
    let exhausted = prepare_followup_exhausted_payload(
        attempted_rounds,
        max_rounds,
        reason,
        missing_requirements,
        result_fingerprint,
    );
    FollowupBranchData {
        unsatisfied,
        pending_summary,
        exhausted_summary,
        scheduled,
        exhausted,
    }
}
