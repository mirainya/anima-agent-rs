//! Agent 核心域：智能体引擎、任务类型、执行器、Worker 池

pub mod agentic_loop_runner;
pub mod context_types;
pub mod core;
pub mod escalation;
pub mod event_emitter;
pub mod executor;
pub mod facade;
pub mod inbound_pipeline;
pub mod initial_execution;
pub mod lifecycle;
pub mod question_continuation;
pub mod requirement;
pub mod runtime_error;
pub mod runtime_helpers;
pub mod runtime_ids;
pub mod session_bootstrap;
pub mod suspension;
pub mod types;
pub mod upstream_resolution;
pub mod worker;

pub use self::core::{
    CoreAgent, CoreAgentStatus, ExecutionStageDurations, ExecutionSummary,
    RuntimeFailureSnapshot, RuntimeFailureStatus, RuntimeTimelineEvent, SessionContext,
};
pub use executor::{
    SdkTaskExecutor, TaskExecutor, TaskExecutorError, UnifiedStreamLine, UnifiedStreamSource,
};
pub use facade::{Agent, AgentStatus};
pub use suspension::{
    PendingQuestion, PendingQuestionSourceKind, QuestionAnswerInput, QuestionDecisionMode,
    QuestionKind, QuestionRiskLevel,
};
pub use types::*;
pub use worker::{
    CurrentTaskInfo, RuntimeEventPublisher, WorkerAgent, WorkerMetrics, WorkerPool,
    WorkerPoolStatus, WorkerStatus,
};

pub(crate) use runtime_helpers::*;
pub(crate) use suspension::{
    classify_question_requires_user_confirmation, detect_pending_question,
};
