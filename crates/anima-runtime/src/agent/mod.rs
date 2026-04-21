//! Agent 核心域：智能体引擎、任务类型、执行器、Worker 池

pub(crate) mod agentic_loop_runner;
pub(crate) mod context_types;
pub mod core;
pub(crate) mod escalation;
pub mod event_emitter;
pub mod executor;
pub(crate) mod facade;
pub(crate) mod inbound_pipeline;
pub(crate) mod initial_execution;
pub(crate) mod lifecycle;
pub(crate) mod question_continuation;
pub(crate) mod requirement;
pub mod runtime_error;
pub(crate) mod runtime_helpers;
pub(crate) mod runtime_ids;
pub(crate) mod session_bootstrap;
pub mod suspension;
pub mod types;
pub(crate) mod upstream_resolution;
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
pub use types::{
    make_task, make_task_result, ExecutionPlan, ExecutionPlanKind, MakeTask, MakeTaskResult, Task,
    TaskResult,
};
pub use worker::{
    CurrentTaskInfo, RuntimeEventPublisher, WorkerAgent, WorkerMetrics, WorkerPool,
    WorkerPoolStatus, WorkerStatus,
};

pub(crate) use runtime_helpers::extract_response_text;
pub(crate) use suspension::{
    classify_question_requires_user_confirmation, detect_pending_question,
};
