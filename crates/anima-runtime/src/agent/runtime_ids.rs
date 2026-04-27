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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_format() {
        assert_eq!(runtime_run_id("job123"), "run_job123");
    }

    #[test]
    fn turn_id_format() {
        assert_eq!(runtime_turn_id("job1", "initial"), "turn_job1_initial");
    }

    #[test]
    fn task_id_all_phases() {
        assert_eq!(runtime_task_id("j", RuntimeTaskPhase::Main), "task_j_main");
        assert_eq!(runtime_task_id("j", RuntimeTaskPhase::Question), "task_j_question");
        assert_eq!(runtime_task_id("j", RuntimeTaskPhase::ToolPermission), "task_j_tool_permission");
        assert_eq!(runtime_task_id("j", RuntimeTaskPhase::Followup), "task_j_followup");
        assert_eq!(runtime_task_id("j", RuntimeTaskPhase::Requirement), "task_j_requirement");
    }

    #[test]
    fn requirement_id_format() {
        assert_eq!(runtime_requirement_id("j1"), "requirement_j1");
    }

    #[test]
    fn suspension_id_format() {
        assert_eq!(runtime_suspension_id("q1"), "suspension_q1");
    }

    #[test]
    fn execution_kind_labels() {
        assert_eq!(execution_kind_label(ExecutionKind::Initial), "initial");
        assert_eq!(execution_kind_label(ExecutionKind::QuestionContinuation), "question_continuation");
        assert_eq!(execution_kind_label(ExecutionKind::Followup), "followup");
    }
}
