use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    Waiting,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Main,
    Plan,
    Subtask,
    Followup,
    Question,
    ToolInvocation,
    Requirement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Suspended,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspensionKind {
    Question,
    ToolPermission,
    FollowupWait,
    ExternalCallback,
    HumanApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspensionStatus {
    Active,
    Resolved,
    Cleared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Active,
    Satisfied,
    WaitingUserInput,
    FollowupScheduled,
    Exhausted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub trace_id: String,
    pub job_id: String,
    pub chat_id: Option<String>,
    pub channel: String,
    pub status: RunStatus,
    pub current_turn_id: Option<String>,
    pub latest_error: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_id: String,
    pub run_id: String,
    pub source: String,
    pub status: TurnStatus,
    pub transcript_checkpoint: usize,
    pub requirement_id: Option<String>,
    pub suspension_id: Option<String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub run_id: String,
    pub turn_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub trace_id: String,
    pub job_id: String,
    pub parent_job_id: Option<String>,
    pub plan_id: Option<String>,
    pub kind: TaskKind,
    pub name: String,
    pub task_type: String,
    pub description: String,
    pub status: TaskStatus,
    pub execution_mode: Option<String>,
    pub result_kind: Option<String>,
    pub specialist_type: Option<String>,
    pub dependencies: Vec<String>,
    pub metadata: Value,
    pub started_at_ms: Option<u64>,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuspensionRecord {
    pub suspension_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub task_id: Option<String>,
    pub question_id: Option<String>,
    pub invocation_id: Option<String>,
    pub kind: SuspensionKind,
    pub status: SuspensionStatus,
    pub prompt: Option<String>,
    pub options: Vec<String>,
    pub raw_payload: Value,
    pub resolution_source: Option<String>,
    pub answer_summary: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub resolved_at_ms: Option<u64>,
    pub cleared_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocationRuntimeRecord {
    pub invocation_id: String,
    pub run_id: String,
    pub turn_id: Option<String>,
    pub task_id: Option<String>,
    pub tool_name: String,
    pub tool_use_id: Option<String>,
    pub phase: String,
    pub permission_state: String,
    pub input_preview: Option<String>,
    pub result_summary: Option<String>,
    pub error_summary: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequirementRecord {
    pub requirement_id: String,
    pub run_id: String,
    pub turn_id: Option<String>,
    pub job_id: String,
    pub original_user_request: String,
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub last_result_fingerprint: Option<String>,
    pub last_reason: Option<String>,
    pub status: RequirementStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RuntimeTaskIndex {
    pub run_ids_by_job_id: HashMap<String, String>,
    pub task_ids_by_plan_id: HashMap<String, Vec<String>>,
    pub suspension_ids_by_question_id: HashMap<String, String>,
    pub invocation_ids_by_question_id: HashMap<String, String>,
}
