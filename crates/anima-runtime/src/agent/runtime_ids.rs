use crate::execution::driver::ExecutionKind;

use super::context_types::RuntimeTaskPhase;

pub(crate) fn runtime_run_id(job_id: &str) -> String {
    format!("run_{job_id}")
}

pub(crate) fn runtime_turn_id(job_id: &str, source: &str) -> String {
    format!("turn_{job_id}_{source}")
}

pub(crate) fn runtime_task_id(job_id: &str, phase: RuntimeTaskPhase) -> String {
    let label = match phase {
        RuntimeTaskPhase::Main => "main",
        RuntimeTaskPhase::Question => "question",
        RuntimeTaskPhase::ToolPermission => "tool_permission",
        RuntimeTaskPhase::Followup => "followup",
        RuntimeTaskPhase::Requirement => "requirement",
    };
    format!("task_{job_id}_{label}")
}

pub(crate) fn runtime_requirement_id(job_id: &str) -> String {
    format!("requirement_{job_id}")
}

pub(crate) fn runtime_suspension_id(question_id: &str) -> String {
    format!("suspension_{question_id}")
}

pub(crate) fn execution_kind_label(kind: ExecutionKind) -> &'static str {
    match kind {
        ExecutionKind::Initial => "initial",
        ExecutionKind::QuestionContinuation => "question_continuation",
        ExecutionKind::Followup => "followup",
    }
}
