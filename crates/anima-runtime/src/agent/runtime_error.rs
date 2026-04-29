use super::context_types::RuntimeErrorInfo;
use crate::tools::result::ToolError;
use std::fmt;

/// Typed error for agent-layer operations, replacing `Result<_, String>`.
#[derive(Debug, Clone)]
pub enum AgentError {
    /// Arc::get_mut failed — another reference exists.
    ArcMutationConflict(&'static str),
    /// Upstream API call failed (wraps RuntimeError.internal_message).
    ExecutionFailed(String),
    /// Continuation after question/answer/auto-resolve failed.
    ContinuationFailed(String),
    /// No pending question found for the given job.
    NoPendingQuestion(String),
    /// Question ID mismatch when answering.
    QuestionIdMismatch {
        job_id: String,
        expected: String,
        got: String,
    },
    /// Missing inbound context for a pending question.
    MissingInboundContext(String),
    /// Orchestrator: forced fallback (test sentinel).
    OrchestrationForcedFallback,
    /// Orchestrator: LLM decided no decomposition needed.
    OrchestrationNoDecomposition,
    /// Classification confidence below threshold.
    ClassificationBelowThreshold { confidence: f64, threshold: f64 },
    /// Bus channel send failed.
    BusSendFailed(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArcMutationConflict(target) => {
                write!(
                    f,
                    "cannot mutate shared {target} — ensure no other Arc references exist"
                )
            }
            Self::ExecutionFailed(msg) => write!(f, "{msg}"),
            Self::ContinuationFailed(msg) => write!(f, "{msg}"),
            Self::NoPendingQuestion(job_id) => {
                write!(f, "no pending question for job: {job_id}")
            }
            Self::QuestionIdMismatch {
                job_id,
                expected,
                got,
            } => write!(
                f,
                "question ID mismatch for job {job_id}: expected {expected}, got {got}"
            ),
            Self::MissingInboundContext(job_id) => {
                write!(
                    f,
                    "missing inbound context for pending question on job: {job_id}"
                )
            }
            Self::OrchestrationForcedFallback => write!(f, "forced orchestration fallback"),
            Self::OrchestrationNoDecomposition => write!(f, "llm_no_decomposition"),
            Self::ClassificationBelowThreshold {
                confidence,
                threshold,
            } => write!(
                f,
                "classification confidence {confidence:.2} below threshold {threshold:.2}"
            ),
            Self::BusSendFailed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AgentError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeErrorStage {
    SessionCreate,
    PlanExecute,
    WorkerPool,
    TaskExecution,
}

impl RuntimeErrorStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SessionCreate => "session_create",
            Self::PlanExecute => "plan_execute",
            Self::WorkerPool => "worker_pool",
            Self::TaskExecution => "task_execution",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeErrorKind {
    SessionCreateFailed,
    UpstreamTimeout,
    UpstreamStreamFailed,
    ResponseParseFailed,
    ToolValidationFailed,
    ToolPermissionDenied,
    ToolTimeout,
    ToolExecutionFailed,
    WorkerUnavailable,
    WorkerCapacityExhausted,
    InvalidTaskPayload,
    UnknownTaskType,
    TaskExecutionFailed,
}

impl RuntimeErrorKind {
    pub(crate) fn as_code(self) -> &'static str {
        match self {
            Self::SessionCreateFailed => "session_create_failed",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::UpstreamStreamFailed => "upstream_stream_failed",
            Self::ResponseParseFailed => "task_execution_failed",
            Self::ToolValidationFailed => "tool_validation_failed",
            Self::ToolPermissionDenied => "tool_permission_denied",
            Self::ToolTimeout => "tool_timeout",
            Self::ToolExecutionFailed => "task_execution_failed",
            Self::WorkerUnavailable => "worker_unavailable",
            Self::WorkerCapacityExhausted => "worker_capacity_exhausted",
            Self::InvalidTaskPayload => "invalid_task_payload",
            Self::UnknownTaskType => "unknown_task_type",
            Self::TaskExecutionFailed => "task_execution_failed",
        }
    }

    pub(crate) fn user_message(self) -> &'static str {
        match self {
            Self::SessionCreateFailed => "无法创建上游会话，请确认 opencode-server 是否正常运行。",
            Self::UpstreamTimeout => "上游模型响应超时，请稍后重试。",
            Self::UpstreamStreamFailed => {
                "上游模型流式响应异常中断，请稍后重试或检查代理服务状态。"
            }
            Self::WorkerUnavailable => "当前执行器未就绪，暂时无法处理请求。",
            Self::WorkerCapacityExhausted => "当前执行队列繁忙，请稍后再试。",
            Self::InvalidTaskPayload => "运行时生成了无效任务，请检查主链路任务构建逻辑。",
            Self::UnknownTaskType => "运行时生成了未支持的任务类型。",
            Self::ToolValidationFailed => "工具输入校验失败，请检查参数后重试。",
            Self::ToolPermissionDenied => "工具调用未获授权，已停止当前工具执行。",
            Self::ToolTimeout => "工具执行超时，请稍后重试或缩小操作范围。",
            Self::ResponseParseFailed | Self::ToolExecutionFailed | Self::TaskExecutionFailed => {
                "任务执行失败，请查看运行时事件获取详细原因。"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub(crate) kind: RuntimeErrorKind,
    pub(crate) stage: RuntimeErrorStage,
    pub(crate) internal_message: String,
}

impl RuntimeError {
    pub(crate) fn new(
        kind: RuntimeErrorKind,
        stage: RuntimeErrorStage,
        internal_message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            stage,
            internal_message: internal_message.into(),
        }
    }

    pub(crate) fn from_tool_error(error: &ToolError, stage: RuntimeErrorStage) -> Self {
        match error {
            ToolError::ValidationFailed(message) => Self::new(
                RuntimeErrorKind::ToolValidationFailed,
                stage,
                message.clone(),
            ),
            ToolError::PermissionDenied(message) => Self::new(
                RuntimeErrorKind::ToolPermissionDenied,
                stage,
                message.clone(),
            ),
            ToolError::Timeout {
                tool_name,
                timeout_ms,
            } => Self::new(
                RuntimeErrorKind::ToolTimeout,
                stage,
                format!("tool '{tool_name}' timed out after {timeout_ms}ms"),
            ),
            ToolError::Internal(message) => Self::new(
                RuntimeErrorKind::ToolExecutionFailed,
                stage,
                message.clone(),
            ),
        }
    }

    pub fn message(&self) -> &str {
        &self.internal_message
    }

    pub(crate) fn to_error_info(&self) -> RuntimeErrorInfo {
        RuntimeErrorInfo {
            code: self.kind.as_code(),
            stage: self.stage.as_str(),
            user_message: self.kind.user_message().into(),
            internal_message: self.internal_message.clone(),
        }
    }
}

impl From<String> for RuntimeError {
    fn from(message: String) -> Self {
        RuntimeError::new(
            RuntimeErrorKind::TaskExecutionFailed,
            RuntimeErrorStage::TaskExecution,
            message,
        )
    }
}

impl From<&str> for RuntimeError {
    fn from(message: &str) -> Self {
        RuntimeError::from(message.to_string())
    }
}

fn classify_session_create_error(raw: &str, raw_lower: &str) -> Option<RuntimeErrorInfo> {
    let looks_like_session_transport_error = raw_lower.contains("http transport error")
        || raw_lower.contains("error sending request")
        || raw_lower.contains("/session)")
        || raw_lower.contains("/session ")
        || raw_lower.ends_with("/session")
        || raw_lower.contains("/session?");

    if raw.contains("OpenCode session")
        || raw.contains("no ID returned")
        || raw.contains("Failed to create session")
        || raw_lower.contains("create session")
        || raw_lower.contains("session-create")
        || looks_like_session_transport_error
    {
        return Some(
            RuntimeError::new(
                RuntimeErrorKind::SessionCreateFailed,
                RuntimeErrorStage::SessionCreate,
                raw,
            )
            .to_error_info(),
        );
    }

    None
}

fn classify_plan_execute_error(raw: &str, raw_lower: &str) -> Option<RuntimeErrorInfo> {
    let looks_like_upstream_stream_error = raw_lower.contains("empty_stream")
        || raw_lower.contains("upstream stream closed before first payload")
        || raw_lower.contains("stream disconnected before completion")
        || raw_lower.contains("stream closed before response.completed");
    let looks_like_upstream_timeout = raw_lower.contains("request timeout")
        || raw_lower.contains("408 request timeout")
        || raw_lower.contains("timed out")
        || raw_lower.contains("timeout");

    if looks_like_upstream_timeout {
        return Some(
            RuntimeError::new(
                RuntimeErrorKind::UpstreamTimeout,
                RuntimeErrorStage::PlanExecute,
                raw,
            )
            .to_error_info(),
        );
    }

    if looks_like_upstream_stream_error {
        return Some(
            RuntimeError::new(
                RuntimeErrorKind::UpstreamStreamFailed,
                RuntimeErrorStage::PlanExecute,
                raw,
            )
            .to_error_info(),
        );
    }

    None
}

fn classify_worker_pool_error(raw: &str) -> Option<RuntimeErrorInfo> {
    if raw.contains("Worker pool is not running") || raw.contains("Worker is not running") {
        return Some(
            RuntimeError::new(
                RuntimeErrorKind::WorkerUnavailable,
                RuntimeErrorStage::WorkerPool,
                raw,
            )
            .to_error_info(),
        );
    }

    if raw.contains("Worker is busy") || raw.contains("No available worker") {
        return Some(
            RuntimeError::new(
                RuntimeErrorKind::WorkerCapacityExhausted,
                RuntimeErrorStage::WorkerPool,
                raw,
            )
            .to_error_info(),
        );
    }

    None
}

pub(crate) fn classify_runtime_error(
    error: Option<&str>,
    fallback_stage: Option<&'static str>,
) -> RuntimeErrorInfo {
    let raw = error.unwrap_or("Unknown runtime error");
    let raw_lower = raw.to_ascii_lowercase();

    if fallback_stage == Some(RuntimeErrorStage::SessionCreate.as_str()) {
        if let Some(info) = classify_session_create_error(raw, &raw_lower) {
            return info;
        }
    }

    if fallback_stage == Some(RuntimeErrorStage::PlanExecute.as_str()) {
        if let Some(info) = classify_plan_execute_error(raw, &raw_lower) {
            return info;
        }
    }

    if let Some(info) = classify_worker_pool_error(raw) {
        return info;
    }

    if raw.contains("Missing required fields")
        || raw.contains("Missing query")
        || raw.contains("Missing transform data")
    {
        return RuntimeError::new(
            RuntimeErrorKind::InvalidTaskPayload,
            match fallback_stage {
                Some("session_create") => RuntimeErrorStage::SessionCreate,
                Some("plan_execute") => RuntimeErrorStage::PlanExecute,
                Some("worker_pool") => RuntimeErrorStage::WorkerPool,
                _ => RuntimeErrorStage::TaskExecution,
            },
            raw,
        )
        .to_error_info();
    }

    if raw.contains("Unknown task type") {
        return RuntimeError::new(
            RuntimeErrorKind::UnknownTaskType,
            match fallback_stage {
                Some("session_create") => RuntimeErrorStage::SessionCreate,
                Some("plan_execute") => RuntimeErrorStage::PlanExecute,
                Some("worker_pool") => RuntimeErrorStage::WorkerPool,
                _ => RuntimeErrorStage::TaskExecution,
            },
            raw,
        )
        .to_error_info();
    }

    RuntimeError::new(
        RuntimeErrorKind::TaskExecutionFailed,
        match fallback_stage {
            Some("session_create") => RuntimeErrorStage::SessionCreate,
            Some("plan_execute") => RuntimeErrorStage::PlanExecute,
            Some("worker_pool") => RuntimeErrorStage::WorkerPool,
            _ => RuntimeErrorStage::TaskExecution,
        },
        raw,
    )
    .to_error_info()
}

#[cfg(test)]
mod tests {
    use super::{RuntimeError, RuntimeErrorKind, RuntimeErrorStage};
    use crate::tools::result::ToolError;

    #[test]
    fn maps_tool_validation_error_to_runtime_error() {
        let error = RuntimeError::from_tool_error(
            &ToolError::ValidationFailed("missing path".into()),
            RuntimeErrorStage::PlanExecute,
        );

        assert_eq!(error.kind, RuntimeErrorKind::ToolValidationFailed);
        assert_eq!(error.stage, RuntimeErrorStage::PlanExecute);
        assert_eq!(error.internal_message, "missing path");
        let info = error.to_error_info();
        assert_eq!(info.code, "tool_validation_failed");
    }

    #[test]
    fn maps_tool_permission_denied_error_to_runtime_error() {
        let error = RuntimeError::from_tool_error(
            &ToolError::PermissionDenied("blocked by policy".into()),
            RuntimeErrorStage::PlanExecute,
        );

        assert_eq!(error.kind, RuntimeErrorKind::ToolPermissionDenied);
        assert_eq!(error.stage, RuntimeErrorStage::PlanExecute);
        assert_eq!(error.internal_message, "blocked by policy");
        let info = error.to_error_info();
        assert_eq!(info.code, "tool_permission_denied");
    }

    #[test]
    fn maps_tool_timeout_error_to_runtime_error() {
        let error = RuntimeError::from_tool_error(
            &ToolError::Timeout {
                tool_name: "bash_exec".into(),
                timeout_ms: 3_000,
            },
            RuntimeErrorStage::PlanExecute,
        );

        assert_eq!(error.kind, RuntimeErrorKind::ToolTimeout);
        assert_eq!(error.stage, RuntimeErrorStage::PlanExecute);
        assert!(error.internal_message.contains("bash_exec"));
        let info = error.to_error_info();
        assert_eq!(info.code, "tool_timeout");
    }

    #[test]
    fn maps_tool_internal_error_to_generic_tool_execution_failure() {
        let error = RuntimeError::from_tool_error(
            &ToolError::Internal("spawn failed".into()),
            RuntimeErrorStage::PlanExecute,
        );

        assert_eq!(error.kind, RuntimeErrorKind::ToolExecutionFailed);
        assert_eq!(error.stage, RuntimeErrorStage::PlanExecute);
        assert_eq!(error.internal_message, "spawn failed");
        let info = error.to_error_info();
        assert_eq!(info.code, "task_execution_failed");
    }
}
