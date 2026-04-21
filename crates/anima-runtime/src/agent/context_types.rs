//! Agent 内部类型定义：执行上下文、准备阶段结构体、任务阶段枚举

use crate::bus::InboundMessage;
use crate::execution::agentic_loop::AgenticLoopConfig;
use crate::execution::driver::ExecutionKind;
use crate::execution::turn_coordinator::{TurnOutcomePlan, TurnSource};

use super::suspension::{PendingQuestion, SuspendedToolInvocationState};
use super::types::{ExecutionPlan, TaskResult};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeErrorInfo {
    pub(crate) code: &'static str,
    pub(crate) stage: &'static str,
    pub(crate) user_message: String,
    pub(crate) internal_message: String,
}

pub(crate) const DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExecutionContext {
    pub(crate) memory_key: String,
    pub(crate) history_session_id: String,
    pub(crate) opencode_session_id: String,
    pub(crate) plan_type: String,
    pub(crate) context_ms: u64,
    pub(crate) session_ms: u64,
    pub(crate) classify_ms: u64,
    pub(crate) execute_ms: u64,
    pub(crate) total_ms: u64,
    pub(crate) cache_hit: bool,
}

pub(crate) struct ToolPermissionResumePreparation {
    pub(crate) suspended: SuspendedToolInvocationState,
    pub(crate) config: AgenticLoopConfig,
    pub(crate) resumed_messages: Vec<crate::messages::types::InternalMsg>,
}

pub(crate) struct InitialAgenticLoopRunPreparation {
    pub(crate) initial_messages: Vec<crate::messages::types::InternalMsg>,
    pub(crate) config: AgenticLoopConfig,
}

pub(crate) struct InitialPlanDispatchContext<'a> {
    pub(crate) inbound_msg: &'a InboundMessage,
    pub(crate) plan: &'a ExecutionPlan,
    pub(crate) opencode_session_id: &'a str,
    pub(crate) memory_key: &'a str,
    pub(crate) execution_kind: ExecutionKind,
}

pub(crate) struct InitialExecutionOutcome {
    pub(crate) plan_type: String,
    pub(crate) cache_hit: bool,
    pub(crate) execute_ms: u64,
    pub(crate) result: TaskResult,
}

pub(crate) struct ProcessInboundResolutionContext<'a> {
    pub(crate) inbound_msg: &'a InboundMessage,
    pub(crate) key: &'a str,
    pub(crate) opencode_session_id: &'a str,
    pub(crate) plan_type: &'a str,
    pub(crate) context_ms: u64,
    pub(crate) session_ms: u64,
    pub(crate) classify_ms: u64,
    pub(crate) execute_ms: u64,
    pub(crate) total_ms: u64,
    pub(crate) cache_hit: bool,
}

pub(crate) struct InboundContextPreparation {
    pub(crate) key: String,
    pub(crate) context_ms: u64,
}

pub(crate) struct SessionPreparation {
    pub(crate) opencode_session_id: String,
    pub(crate) session_ms: u64,
}

pub(crate) struct PlanPreparation {
    pub(crate) plan: ExecutionPlan,
    pub(crate) classify_ms: u64,
}

pub(crate) struct AgentFollowupPreparation {
    pub(crate) progress: crate::execution::requirement_judge::RequirementProgressState,
    pub(crate) next_round: usize,
    pub(crate) fingerprint: String,
    pub(crate) followup_branch: crate::execution::turn_coordinator::FollowupBranchData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeTaskPhase {
    Main,
    Question,
    ToolPermission,
    Followup,
    Requirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuccessSource {
    Initial,
    QuestionContinuation,
    AgentFollowup,
}

pub(crate) struct UpstreamTurnContext<'a> {
    pub(crate) inbound_msg: &'a InboundMessage,
    pub(crate) exec_ctx: &'a ExecutionContext,
    pub(crate) result: &'a TaskResult,
    pub(crate) resolved_question: Option<&'a PendingQuestion>,
    pub(crate) turn_source_label: &'a str,
}

pub(crate) struct UpstreamRequirementContext {
    pub(crate) turn_plan: TurnOutcomePlan,
    pub(crate) turn_source_label: &'static str,
}

pub(crate) fn turn_source_from_success(source: SuccessSource) -> TurnSource {
    match source {
        SuccessSource::Initial => TurnSource::Initial,
        SuccessSource::QuestionContinuation => TurnSource::QuestionContinuation,
        SuccessSource::AgentFollowup => TurnSource::AgentFollowup,
    }
}

pub(crate) fn memory_key(inbound_msg: &InboundMessage) -> String {
    format!(
        "{}:{}",
        inbound_msg.channel,
        inbound_msg
            .chat_id
            .clone()
            .unwrap_or_else(|| inbound_msg.id.clone())
    )
}
