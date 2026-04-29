//! 执行循环域：执行驱动、回合协调、上下文装配、需求判定、Agentic Loop

pub mod agentic_loop;
pub mod context_assembly;
pub mod driver;
pub mod requirement_judge;
pub mod turn_coordinator;

pub use agentic_loop::{
    build_api_payload, build_assistant_msg, build_tool_result_msg, continue_agentic_loop,
    parse_response, resume_suspended_tool_invocation, run_agentic_loop, AgenticLoopConfig,
    AgenticLoopError, AgenticLoopOutcome, AgenticLoopResult, AgenticLoopSuspension, ParsedResponse,
    ParsedToolUse, SuspendedToolInvocation,
};
pub use context_assembly::{
    assemble_context, ContextAssemblyMetadata, ContextAssemblyMode, ContextAssemblyRequest,
    ContextAssemblyResult, RequirementContextSnapshot,
};
pub use driver::{build_api_call_task, execute_api_call, ApiCallExecutionRequest, ExecutionKind};
pub use requirement_judge::{
    fingerprint_result, judge_requirement, requirement_unsatisfied_payload, AgentFollowupPlan,
    RequirementJudgeContext, RequirementJudgement, RequirementProgressState, UserInputRequirement,
};
pub use turn_coordinator::{
    assembly_mode_for_source, build_requirement_judge_context, plan_turn_outcome,
    prepare_completed_branch_data, prepare_completion_data, prepare_followup_branch_data,
    prepare_followup_exhausted_payload, prepare_followup_scheduled_payload,
    prepare_message_completed_payload, prepare_pending_question_for_storage,
    prepare_question_asked_payload, prepare_question_resolved_payload,
    prepare_requirement_evaluation, prepare_requirement_evaluation_started_payload,
    prepare_requirement_satisfied_payload, prepare_requirement_unsatisfied_event_payload,
    prepare_summary_input, prepare_upstream_response_observed_payload,
    prepare_user_input_required_payload, prepare_waiting_user_input_branch_data,
    prepare_waiting_user_input_data, resolved_question_to_publish, source_label,
    CompletedBranchData, CompletionData, FollowupBranchData, FollowupExhaustedPayload,
    FollowupScheduledPayload, MessageCompletedPayload, QuestionAskedPayload,
    QuestionResolvedPayload, RequirementEvaluationPreparation, RequirementEvaluationStartedPayload,
    RequirementSatisfiedPayload, RequirementUnsatisfiedEventPayload, SummaryInput, TurnOutcomePlan,
    TurnSource, UpstreamResponseObservedPayload, UserInputRequiredPayload,
    WaitingUserInputBranchData, WaitingUserInputData,
};
