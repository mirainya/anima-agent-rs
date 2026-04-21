pub(crate) mod events;
pub(crate) mod projection;
pub(crate) mod reducer;
pub(crate) mod snapshot;
pub(crate) mod state_store;

pub use events::RuntimeDomainEvent;
pub use projection::{
    build_projection, completed_current_step, creating_session_current_step,
    executing_job_current_step, executing_plan_current_step, failed_current_step,
    followup_current_step, planning_ready_current_step, planning_stalled_current_step,
    preparing_context_current_step, queued_current_step, session_ready_current_step,
    tool_execution_failed_current_step, tool_execution_finished_current_step,
    tool_permission_resolved_current_step, tool_phase_current_step,
    tool_result_recorded_current_step, waiting_user_input_current_step,
    worker_executing_current_step, ProjectionExecutionSummary, ProjectionFailureSummary,
    ProjectionJobStatusSummary, ProjectionOrchestrationSummary, ProjectionToolStateSummary,
    RuntimeProjectionView,
};
pub use reducer::reduce_event;
pub use snapshot::{PersistedRuntimeState, RuntimeDomainEventEnvelope, RuntimeStateSnapshot};
pub use state_store::{RuntimeStateStore, SharedRuntimeStateStore};
