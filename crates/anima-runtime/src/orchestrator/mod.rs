//! 编排与调度域：编排引擎、并行池、专家池

pub mod core;
pub mod llm_context_infer;
pub(crate) mod llm_decompose;
pub mod parallel_pool;
pub mod specialist_pool;

pub use self::core::{
    AgentOrchestrator, LoweredTask, LoweringPrimitive, OrchestrationExecutionResult,
    OrchestrationPlan, OrchestratorConfig, OrchestratorMetrics, PlanProgress, SubTask,
};
pub use parallel_pool::{
    ParallelPool, ParallelPoolConfig, ParallelPoolMetrics, ParallelResult, ParallelTask,
};
pub use specialist_pool::{
    LoadBalanceStrategy, RegisterSpecialistOpts, Specialist, SpecialistInfo, SpecialistMetrics,
    SpecialistPool, SpecialistPoolConfig, SpecialistPoolMetrics, SpecialistStatus,
};
