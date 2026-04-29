use crate::agent::core::PendingQuestion;
use crate::runtime::snapshot::RuntimeStateSnapshot;
use crate::tasks::model::{
    RequirementRecord, RequirementStatus, RunRecord, RunStatus, SuspensionRecord, TaskKind,
    TaskRecord, TaskStatus, ToolInvocationRuntimeRecord, TurnRecord, TurnStatus,
};
use crate::tasks::{
    active_requirement, active_suspension, invocation_by_question_id, latest_tool_invocation,
    plan_task, subtasks_for_plan,
};
use crate::transcript::model::MessageRecord;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionExecutionSummary {
    pub plan_type: String,
    pub status: String,
    pub cache_hit: bool,
    pub worker_id: Option<String>,
    pub error_code: Option<String>,
    pub error_stage: Option<String>,
    pub task_duration_ms: u64,
    pub stages: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionJobStatusSummary {
    pub status: String,
    pub status_label: String,
    pub current_step: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionFailureSummary {
    pub error_code: String,
    pub error_stage: String,
    pub message_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub occurred_at_ms: u64,
    pub internal_message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionOrchestrationSummary {
    pub plan_id: Option<String>,
    pub active_subtask_name: Option<String>,
    pub active_subtask_type: Option<String>,
    pub active_subtask_id: Option<String>,
    pub total_subtasks: usize,
    pub active_subtasks: usize,
    pub completed_subtasks: usize,
    pub failed_subtasks: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionToolStateSummary {
    pub invocation_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub phase: String,
    pub permission_state: Option<String>,
    pub input_preview: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub awaits_user_confirmation: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeProjectionView {
    pub runs: Vec<RunRecord>,
    pub turns: Vec<TurnRecord>,
    pub tasks: Vec<TaskRecord>,
    pub suspensions: Vec<SuspensionRecord>,
    pub tool_invocations: Vec<ToolInvocationRuntimeRecord>,
    pub requirements: Vec<RequirementRecord>,
    pub transcript: Vec<MessageRecord>,
    pub execution_summaries: HashMap<String, ProjectionExecutionSummary>,
    pub failures: HashMap<String, ProjectionFailureSummary>,
    pub orchestration: HashMap<String, ProjectionOrchestrationSummary>,
    pub pending_questions: HashMap<String, PendingQuestion>,
    pub tool_states: HashMap<String, ProjectionToolStateSummary>,
    pub job_statuses: HashMap<String, ProjectionJobStatusSummary>,
}

fn derive_execution_summary(
    snapshot: &RuntimeStateSnapshot,
    run: &RunRecord,
) -> ProjectionExecutionSummary {
    let hinted = snapshot
        .projection_hints
        .get(&run.run_id)
        .and_then(|scoped| scoped.get("execution.summary"))
        .cloned();
    if let Some(value) = hinted {
        if let Ok(summary) = serde_json::from_value::<ProjectionExecutionSummary>(value.clone()) {
            return summary;
        }
        return ProjectionExecutionSummary {
            plan_type: value
                .get("plan_type")
                .and_then(Value::as_str)
                .unwrap_or("runtime")
                .to_string(),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("running")
                .to_string(),
            cache_hit: value
                .get("cache_hit")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            worker_id: value
                .get("worker_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            error_code: value
                .get("error_code")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            error_stage: value
                .get("error_stage")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            task_duration_ms: value
                .get("task_duration_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            stages: value.get("stages").cloned().unwrap_or_else(|| json!({})),
        };
    }

    let requirement = active_requirement(snapshot, &run.run_id);
    let waiting_for_input = active_suspension(snapshot, &run.run_id).is_some();
    let plan_type = if snapshot
        .tasks
        .values()
        .any(|task| task.run_id == run.run_id && task.kind == TaskKind::Plan)
    {
        "orchestration-v1"
    } else {
        "single"
    };
    let status = match run.status {
        RunStatus::Completed => "success",
        RunStatus::Failed => "failure",
        RunStatus::Running => {
            if waiting_for_input
                || requirement.is_some_and(|record| {
                    matches!(record.status, RequirementStatus::WaitingUserInput)
                })
            {
                "waiting_user_input"
            } else {
                "running"
            }
        }
    };

    ProjectionExecutionSummary {
        plan_type: plan_type.to_string(),
        status: status.to_string(),
        cache_hit: false,
        worker_id: None,
        error_code: None,
        error_stage: None,
        task_duration_ms: run
            .completed_at_ms
            .unwrap_or(run.updated_at_ms)
            .saturating_sub(run.created_at_ms),
        stages: json!({}),
    }
}

fn derive_failure_summary(run: &RunRecord) -> Option<ProjectionFailureSummary> {
    let latest_error = run.latest_error.as_ref()?;
    Some(ProjectionFailureSummary {
        error_code: "runtime_run_failed".into(),
        error_stage: "runtime".into(),
        message_id: run.job_id.clone(),
        channel: run.channel.clone(),
        chat_id: run.chat_id.clone(),
        occurred_at_ms: run.completed_at_ms.unwrap_or(run.updated_at_ms),
        internal_message: latest_error.clone(),
    })
}

fn derive_orchestration_summary(
    snapshot: &RuntimeStateSnapshot,
    run: &RunRecord,
) -> Option<ProjectionOrchestrationSummary> {
    let mut plan_tasks = snapshot
        .tasks
        .values()
        .filter(|task| task.run_id == run.run_id && task.kind == TaskKind::Plan)
        .collect::<Vec<_>>();
    plan_tasks.sort_by_key(|task| task.updated_at_ms);
    let latest_plan = plan_tasks.pop()?;
    let plan_id = latest_plan
        .plan_id
        .clone()
        .or_else(|| Some(latest_plan.task_id.clone()))?;
    plan_task(snapshot, &plan_id)?;
    let subtasks = subtasks_for_plan(snapshot, &plan_id);
    let active_subtask = subtasks
        .iter()
        .filter(|task| task.status == TaskStatus::Running)
        .max_by_key(|task| task.updated_at_ms)
        .copied();
    Some(ProjectionOrchestrationSummary {
        plan_id: Some(plan_id),
        active_subtask_name: active_subtask.map(|task| task.name.clone()),
        active_subtask_type: active_subtask.map(|task| task.task_type.clone()),
        active_subtask_id: active_subtask.map(|task| task.task_id.clone()),
        total_subtasks: subtasks.len(),
        active_subtasks: subtasks
            .iter()
            .filter(|task| task.status == TaskStatus::Running)
            .count(),
        completed_subtasks: subtasks
            .iter()
            .filter(|task| task.status == TaskStatus::Completed)
            .count(),
        failed_subtasks: subtasks
            .iter()
            .filter(|task| task.status == TaskStatus::Failed)
            .count(),
    })
}

fn derive_pending_question(
    snapshot: &RuntimeStateSnapshot,
    run: &RunRecord,
) -> Option<PendingQuestion> {
    let suspension = active_suspension(snapshot, &run.run_id)?;
    let raw_question = suspension.raw_payload.clone();
    Some(PendingQuestion {
        question_id: suspension
            .question_id
            .clone()
            .unwrap_or_else(|| suspension.suspension_id.clone()),
        job_id: run.job_id.clone(),
        opencode_session_id: raw_question
            .get("opencode_session_id")
            .or_else(|| raw_question.get("session_id"))
            .or_else(|| {
                raw_question
                    .get("opencode_session_id")
                    .and_then(|value| value.get("id"))
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        question_kind: crate::agent::QuestionKind::Confirm,
        prompt: suspension.prompt.clone().unwrap_or_default(),
        options: suspension.options.clone(),
        raw_question,
        decision_mode: crate::agent::QuestionDecisionMode::UserRequired,
        risk_level: crate::agent::QuestionRiskLevel::High,
        requires_user_confirmation: true,
        source_kind: match suspension.kind {
            crate::tasks::model::SuspensionKind::ToolPermission => {
                crate::agent::PendingQuestionSourceKind::ToolPermission
            }
            crate::tasks::model::SuspensionKind::SubtaskBlocked => {
                crate::agent::PendingQuestionSourceKind::SubtaskBlocked
            }
            crate::tasks::model::SuspensionKind::HumanApproval => {
                crate::agent::PendingQuestionSourceKind::PlanApproval
            }
            _ => crate::agent::PendingQuestionSourceKind::UpstreamQuestion,
        },
        continuation_token: Some(suspension.suspension_id.clone()),
        asked_at_ms: suspension.created_at_ms,
        answer_submitted: false,
        answer_summary: suspension.answer_summary.clone(),
        resolution_source: suspension.resolution_source.clone(),
        inbound: None,
    })
}

fn derive_tool_state_summary(
    invocation: &ToolInvocationRuntimeRecord,
    pending_question: Option<&PendingQuestion>,
) -> ProjectionToolStateSummary {
    let awaits_user_confirmation = pending_question
        .map(|question| {
            question.requires_user_confirmation
                && question.raw_question.get("type").and_then(Value::as_str)
                    == Some("tool_permission")
        })
        .unwrap_or_else(|| invocation.permission_state == "requested");

    ProjectionToolStateSummary {
        invocation_id: Some(invocation.invocation_id.clone()),
        tool_name: Some(invocation.tool_name.clone()),
        tool_use_id: invocation.tool_use_id.clone(),
        phase: invocation.phase.clone(),
        permission_state: Some(invocation.permission_state.clone()),
        input_preview: invocation.input_preview.clone(),
        result_preview: invocation.result_summary.clone(),
        error: invocation.error_summary.clone(),
        awaits_user_confirmation,
    }
}

pub fn followup_current_step() -> String {
    "主 agent 判断结果尚未满足需求，正在自动继续补充处理".to_string()
}

pub fn session_ready_current_step() -> String {
    "会话已就绪，正在构建计划".to_string()
}

pub fn planning_ready_current_step(plan_type: &str) -> String {
    format!("已完成规划，准备执行 {plan_type}")
}

pub fn executing_plan_current_step(plan_type: &str) -> String {
    format!("已进入执行阶段，正在处理 {plan_type}")
}

pub fn completed_current_step() -> String {
    "主 agent 已确认结果满足需求".to_string()
}

pub fn failed_current_step() -> String {
    "执行失败".to_string()
}

pub fn tool_permission_resolved_current_step() -> String {
    "工具权限已确认，准备执行".to_string()
}

pub fn tool_execution_finished_current_step() -> String {
    "工具执行完成，正在整理结果".to_string()
}

pub fn tool_execution_failed_current_step() -> String {
    "工具执行失败，正在处理失败结果".to_string()
}

pub fn tool_result_recorded_current_step() -> String {
    "工具结果已记录，等待后续推进".to_string()
}

pub fn tool_phase_current_step(phase: &str) -> String {
    format!("工具调用阶段：{phase}")
}

pub fn executing_job_current_step() -> String {
    "该 Job 已经过了规划阶段，正在执行实际任务。".to_string()
}

pub fn preparing_context_current_step() -> String {
    "正在准备上下文".to_string()
}

pub fn queued_current_step() -> String {
    "已进入队列".to_string()
}

pub fn creating_session_current_step(phase: Option<&str>) -> String {
    match phase {
        Some(phase) if !phase.trim().is_empty() => format!("正在创建上游会话（{phase}）"),
        _ => "正在创建上游会话".to_string(),
    }
}

pub fn worker_executing_current_step(task_type: &str, phase: Option<&str>) -> String {
    match phase {
        Some(phase) if !phase.trim().is_empty() => {
            format!("worker 正在执行 {task_type}（{phase}）")
        }
        _ => format!("worker 正在执行 {task_type}"),
    }
}

pub fn planning_stalled_current_step() -> String {
    "规划已完成，但长时间没有新的运行时进展".to_string()
}

pub fn waiting_user_input_current_step() -> String {
    "等待用户提供所需输入".to_string()
}

fn question_waiting_step(question: &PendingQuestion) -> String {
    let is_tool_permission =
        question.raw_question.get("type").and_then(Value::as_str) == Some("tool_permission");
    if question.answer_summary.is_some() {
        if is_tool_permission {
            "已提交工具权限决定，等待主 agent 继续处理".to_string()
        } else {
            "已提交回答，等待主 agent 继续处理".to_string()
        }
    } else if is_tool_permission {
        "等待用户确认工具调用权限".to_string()
    } else if !question.prompt.trim().is_empty() {
        question.prompt.clone()
    } else {
        waiting_user_input_current_step()
    }
}

fn derive_job_status_summary(
    snapshot: &RuntimeStateSnapshot,
    run: &RunRecord,
    pending_question: Option<&PendingQuestion>,
    tool_state: Option<&ProjectionToolStateSummary>,
    orchestration: Option<&ProjectionOrchestrationSummary>,
) -> ProjectionJobStatusSummary {
    let lifecycle_hint = snapshot
        .projection_hints
        .get(&run.run_id)
        .and_then(|scoped| scoped.get("job.lifecycle"))
        .cloned();

    let current_turn = run
        .current_turn_id
        .as_ref()
        .and_then(|turn_id| snapshot.turns.get(turn_id));
    let requirement = active_requirement(snapshot, &run.run_id);

    if let Some(requirement) = requirement {
        match requirement.status {
            RequirementStatus::Exhausted => {
                return ProjectionJobStatusSummary {
                    status: "failed".into(),
                    status_label: "failed".into(),
                    current_step: failed_current_step(),
                };
            }
            RequirementStatus::Satisfied if run.status == RunStatus::Completed => {
                return ProjectionJobStatusSummary {
                    status: "completed".into(),
                    status_label: "completed".into(),
                    current_step: completed_current_step(),
                };
            }
            RequirementStatus::WaitingUserInput => {
                return ProjectionJobStatusSummary {
                    status: "waiting_user_input".into(),
                    status_label: "waiting_user_input".into(),
                    current_step: pending_question
                        .map(question_waiting_step)
                        .unwrap_or_else(waiting_user_input_current_step),
                };
            }
            RequirementStatus::FollowupScheduled => {
                return ProjectionJobStatusSummary {
                    status: "executing".into(),
                    status_label: "executing".into(),
                    current_step: followup_current_step(),
                };
            }
            RequirementStatus::Satisfied | RequirementStatus::Active => {}
        }
    }

    if let Some(question) = pending_question {
        return ProjectionJobStatusSummary {
            status: "waiting_user_input".into(),
            status_label: "waiting_user_input".into(),
            current_step: question_waiting_step(question),
        };
    }

    if let Some(tool_state) = tool_state {
        let tool_label = tool_state.tool_name.as_deref().unwrap_or("工具");
        let current_step = match tool_state.phase.as_str() {
            "permission_resolved" => tool_permission_resolved_current_step(),
            "execution_started" | "executing" => format!("正在执行工具：{tool_label}"),
            "execution_finished" | "completed" => tool_execution_finished_current_step(),
            "execution_failed" | "failed" => tool_execution_failed_current_step(),
            "result_recorded" => tool_result_recorded_current_step(),
            other => tool_phase_current_step(other),
        };
        return ProjectionJobStatusSummary {
            status: "executing".into(),
            status_label: "executing".into(),
            current_step,
        };
    }

    let orchestration_step = orchestration.and_then(|summary| {
        if let Some(active_subtask_name) = summary.active_subtask_name.as_ref() {
            Some(format!("正在执行子任务：{active_subtask_name}"))
        } else if summary.total_subtasks > 0 {
            Some(format!(
                "orchestration v1：共 {} 个子任务",
                summary.total_subtasks
            ))
        } else {
            None
        }
    });

    match run.status {
        RunStatus::Completed => ProjectionJobStatusSummary {
            status: "completed".into(),
            status_label: "completed".into(),
            current_step: completed_current_step(),
        },
        RunStatus::Failed => ProjectionJobStatusSummary {
            status: "failed".into(),
            status_label: "failed".into(),
            current_step: failed_current_step(),
        },
        RunStatus::Running => {
            if let Some(turn) = current_turn {
                match turn.status {
                    TurnStatus::Waiting => {
                        return ProjectionJobStatusSummary {
                            status: "waiting_user_input".into(),
                            status_label: "waiting_user_input".into(),
                            current_step: pending_question
                                .map(question_waiting_step)
                                .unwrap_or_else(waiting_user_input_current_step),
                        };
                    }
                    TurnStatus::Completed => {
                        return ProjectionJobStatusSummary {
                            status: "completed".into(),
                            status_label: "completed".into(),
                            current_step: completed_current_step(),
                        };
                    }
                    TurnStatus::Failed => {
                        return ProjectionJobStatusSummary {
                            status: "failed".into(),
                            status_label: "failed".into(),
                            current_step: failed_current_step(),
                        };
                    }
                    TurnStatus::Running => {}
                }
            }

            if let Some(value) = lifecycle_hint {
                if let Ok(summary) =
                    serde_json::from_value::<ProjectionJobStatusSummary>(value.clone())
                {
                    return summary;
                }
                if let Some(status_label) = value.get("status_label").and_then(Value::as_str) {
                    let current_step = value
                        .get("current_step")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .unwrap_or_else(|| match status_label {
                            "queued" => queued_current_step(),
                            "preparing_context" => preparing_context_current_step(),
                            "creating_session" => "正在创建上游会话".to_string(),
                            "planning" => "正在构建计划".to_string(),
                            "stalled" => planning_stalled_current_step(),
                            _ => orchestration_step
                                .clone()
                                .unwrap_or_else(executing_job_current_step),
                        });
                    return ProjectionJobStatusSummary {
                        status: value
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or(status_label)
                            .to_string(),
                        status_label: status_label.to_string(),
                        current_step,
                    };
                }
            }

            ProjectionJobStatusSummary {
                status: "executing".into(),
                status_label: "executing".into(),
                current_step: orchestration_step.unwrap_or_else(executing_job_current_step),
            }
        }
    }
}

pub fn build_projection(snapshot: &RuntimeStateSnapshot) -> RuntimeProjectionView {
    let runs = snapshot.runs.values().cloned().collect::<Vec<_>>();
    let mut execution_summaries = HashMap::new();
    let mut failures = HashMap::new();
    let mut orchestration = HashMap::new();
    let mut pending_questions = HashMap::new();
    let mut tool_states = HashMap::new();
    let mut job_statuses = HashMap::new();

    for run in &runs {
        execution_summaries.insert(run.job_id.clone(), derive_execution_summary(snapshot, run));
        if let Some(failure) = derive_failure_summary(run) {
            failures.insert(run.job_id.clone(), failure);
        }
        let orchestration_summary = derive_orchestration_summary(snapshot, run);
        if let Some(summary) = orchestration_summary.clone() {
            orchestration.insert(run.job_id.clone(), summary);
        }
        let pending_question = derive_pending_question(snapshot, run);
        let tool_state_summary = if let Some(question) = pending_question.as_ref() {
            invocation_by_question_id(snapshot, &question.question_id)
                .map(|invocation| derive_tool_state_summary(invocation, Some(question)))
        } else {
            None
        }
        .or_else(|| {
            latest_tool_invocation(snapshot, &run.run_id)
                .map(|invocation| derive_tool_state_summary(invocation, pending_question.as_ref()))
        });
        if let Some(tool_state) = tool_state_summary.clone() {
            tool_states.insert(run.job_id.clone(), tool_state);
        }
        let tool_state = tool_states.get(&run.job_id);
        let status_summary = derive_job_status_summary(
            snapshot,
            run,
            pending_question.as_ref(),
            tool_state,
            orchestration_summary.as_ref(),
        );
        job_statuses.insert(run.job_id.clone(), status_summary);
        if let Some(question) = pending_question {
            pending_questions.insert(run.job_id.clone(), question);
        }
    }

    RuntimeProjectionView {
        runs,
        turns: snapshot.turns.values().cloned().collect(),
        tasks: snapshot.tasks.values().cloned().collect(),
        suspensions: snapshot.suspensions.values().cloned().collect(),
        tool_invocations: snapshot.tool_invocations.values().cloned().collect(),
        requirements: snapshot.requirements.values().cloned().collect(),
        transcript: snapshot.transcript.clone(),
        execution_summaries,
        failures,
        orchestration,
        pending_questions,
        tool_states,
        job_statuses,
    }
}
