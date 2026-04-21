use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserVerdict {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    PreparingContext,
    CreatingSession,
    Planning,
    Executing,
    WaitingUserInput,
    Stalled,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobEventView {
    pub event: String,
    pub recorded_at_ms: u64,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobReviewView {
    pub verdict: UserVerdict,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub reviewed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionView {
    pub question_id: String,
    pub question_kind: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub raw_question: Value,
    pub decision_mode: String,
    pub risk_level: String,
    pub requires_user_confirmation: bool,
    pub source_kind: String,
    pub opencode_session_id: Option<String>,
    pub answer_summary: Option<String>,
    pub resolution_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerTaskView {
    pub worker_id: String,
    pub status: String,
    pub task_id: String,
    pub trace_id: String,
    pub task_type: String,
    pub elapsed_ms: u64,
    pub content_preview: String,
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStateView {
    pub invocation_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub phase: String,
    pub permission_state: Option<String>,
    pub invocation_status: String,
    pub status_text: String,
    pub input_preview: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub awaits_user_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Main,
    Subtask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestrationView {
    pub plan_id: Option<String>,
    pub active_subtask_name: Option<String>,
    pub active_subtask_type: Option<String>,
    pub active_subtask_id: Option<String>,
    pub total_subtasks: usize,
    pub active_subtasks: usize,
    pub completed_subtasks: usize,
    pub failed_subtasks: usize,
    pub child_job_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobView {
    pub job_id: String,
    pub trace_id: String,
    pub message_id: String,
    pub kind: JobKind,
    pub parent_job_id: Option<String>,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: String,
    pub user_content: Option<String>,
    pub status: JobStatus,
    pub status_label: String,
    pub accepted: bool,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub elapsed_ms: u64,
    pub current_step: String,
    pub pending_question: Option<QuestionView>,
    pub recent_events: Vec<JobEventView>,
    pub worker: Option<WorkerTaskView>,
    pub tool_state: Option<ToolStateView>,
    pub execution_summary: Option<Value>,
    pub failure: Option<Value>,
    pub review: Option<JobReviewView>,
    pub orchestration: Option<OrchestrationView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedJob {
    pub job_id: String,
    pub trace_id: String,
    pub message_id: String,
    pub kind: JobKind,
    pub parent_job_id: Option<String>,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: String,
    pub user_content: String,
    pub accepted_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobReviewInput {
    pub user_verdict: UserVerdict,
    pub reason: Option<String>,
    pub note: Option<String>,
}
