pub(crate) mod balancer;
pub mod circuit_breaker;
pub(crate) mod control;
pub(crate) mod core;
pub(crate) mod diagnostics;
pub(crate) mod message;
pub(crate) mod queue;
pub(crate) mod router;

pub use balancer::{
    make_target, Balancer, BalancerDiagnosticsSnapshot, BalancerMetrics, BalancerMissReason,
    BalancerOptions, BalancerRuntimeConfig, BalancerSelectionDiagnostic, BalancerStatsSnapshot,
    BalancerStatusSnapshot, BalancerStrategy, BalancerTargetDiagnostic, BalancerTargetStatus,
    CircuitBreakerConfig, CircuitState, HealthPolicy, LoadUpdate, SelectionReason, Target,
    TargetExcludeReason, TargetHealth, TargetLoad, TargetOptions, TargetStatus,
};
pub use control::{
    apply_dispatcher_control, dispatcher_status, DispatcherControlCommand,
    DispatcherControlResponse,
};
pub use core::{start_dispatch_scheduler, start_dispatcher_outbound_loop, Dispatcher};
pub use diagnostics::{
    DispatchDiagnosticSnapshot, DispatchFailureStage, DispatchOutcomeReason,
    DispatcherRouteReport, DispatcherRouteStatus, DispatcherState, DispatcherStats,
    DispatcherStatsSnapshot, DispatcherStatus, RouteTargetStats,
};
pub use message::{DispatchMessage, DispatcherConfig};
pub use queue::{DispatchEnvelope, DispatchQueue, DispatchQueueSnapshot};
pub use router::{constant_hashing_index, round_robin_index};
// circuit_breaker is accessed via dispatcher::circuit_breaker:: to avoid name conflicts with balancer
