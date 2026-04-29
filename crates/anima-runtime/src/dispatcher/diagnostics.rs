use crate::channel::ChannelLookupReason;
use crate::dispatcher::balancer::{Balancer, BalancerMissReason};
use crate::support::now_ms;
use indexmap::IndexMap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RouteTargetStats {
    pub selected: usize,
    pub success: usize,
    pub failures: usize,
    pub last_failure_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatcherStatsSnapshot {
    pub dispatched: usize,
    pub errors: usize,
    pub channel_not_found: usize,
    pub balancer_selected: usize,
    pub balancer_misses: usize,
    pub balancer_miss_reasons: IndexMap<BalancerMissReason, usize>,
    pub channel_lookup_failures: IndexMap<ChannelLookupReason, usize>,
    pub target_stats: IndexMap<String, RouteTargetStats>,
}

#[derive(Debug, Default)]
pub struct DispatcherStats {
    dispatched: AtomicUsize,
    errors: AtomicUsize,
    channel_not_found: AtomicUsize,
    balancer_selected: AtomicUsize,
    balancer_misses: AtomicUsize,
    balancer_miss_reasons: Mutex<IndexMap<BalancerMissReason, usize>>,
    channel_lookup_failures: Mutex<IndexMap<ChannelLookupReason, usize>>,
    target_stats: Mutex<IndexMap<String, RouteTargetStats>>,
}

impl DispatcherStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> DispatcherStatsSnapshot {
        DispatcherStatsSnapshot {
            dispatched: self.dispatched.load(Ordering::SeqCst),
            errors: self.errors.load(Ordering::SeqCst),
            channel_not_found: self.channel_not_found.load(Ordering::SeqCst),
            balancer_selected: self.balancer_selected.load(Ordering::SeqCst),
            balancer_misses: self.balancer_misses.load(Ordering::SeqCst),
            balancer_miss_reasons: self.balancer_miss_reasons.lock().clone(),
            channel_lookup_failures: self.channel_lookup_failures.lock().clone(),
            target_stats: self.target_stats.lock().clone(),
        }
    }

    pub(crate) fn record_balancer_selected(&self, target_id: &str) {
        self.balancer_selected.fetch_add(1, Ordering::SeqCst);
        let mut target_stats = self.target_stats.lock();
        let entry = target_stats.entry(target_id.to_string()).or_default();
        entry.selected += 1;
    }

    pub(crate) fn record_balancer_miss(&self, reason: Option<BalancerMissReason>) {
        self.balancer_misses.fetch_add(1, Ordering::SeqCst);
        if let Some(reason) = reason {
            let mut miss_reasons = self.balancer_miss_reasons.lock();
            *miss_reasons.entry(reason).or_insert(0) += 1;
        }
    }

    pub(crate) fn record_dispatch_success(&self, target_id: Option<&str>) {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        if let Some(target_id) = target_id {
            let mut target_stats = self.target_stats.lock();
            let entry = target_stats.entry(target_id.to_string()).or_default();
            entry.success += 1;
        }
    }

    pub(crate) fn record_dispatch_error(&self, target_id: Option<&str>) {
        self.errors.fetch_add(1, Ordering::SeqCst);
        if let Some(target_id) = target_id {
            let mut target_stats = self.target_stats.lock();
            let entry = target_stats.entry(target_id.to_string()).or_default();
            entry.failures += 1;
            entry.last_failure_at = Some(now_ms());
        }
    }

    pub(crate) fn record_channel_not_found(
        &self,
        target_id: Option<&str>,
        lookup_reason: ChannelLookupReason,
    ) {
        self.channel_not_found.fetch_add(1, Ordering::SeqCst);
        {
            let mut lookup_failures = self.channel_lookup_failures.lock();
            *lookup_failures.entry(lookup_reason).or_insert(0) += 1;
        }
        if let Some(target_id) = target_id {
            let mut target_stats = self.target_stats.lock();
            let entry = target_stats.entry(target_id.to_string()).or_default();
            entry.failures += 1;
            entry.last_failure_at = Some(now_ms());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatcherState {
    Stopped,
    Running,
    Paused,
    Draining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchFailureStage {
    BalancerSelection,
    ChannelLookup,
    Send,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchOutcomeReason {
    SelectedAndSent,
    BalancerMiss,
    ChannelNotFound,
    SendFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchDiagnosticSnapshot {
    pub message_id: String,
    pub channel: String,
    pub account_id: String,
    pub selection_key: Option<String>,
    pub selected_target_id: Option<String>,
    pub balancer_miss_reason: Option<BalancerMissReason>,
    pub channel_lookup_reason: Option<ChannelLookupReason>,
    pub failure_stage: Option<DispatchFailureStage>,
    pub send_error: Option<String>,
    pub outcome: DispatchOutcomeReason,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatcherRouteStatus {
    pub balancer_id: String,
    pub strategy: String,
    pub target_count: usize,
    pub available_target_count: usize,
    pub healthy_target_count: usize,
    pub open_circuit_count: usize,
    pub half_open_target_count: usize,
    pub pending_queue_depth: usize,
    pub last_miss_reason: Option<BalancerMissReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatcherRouteReport {
    pub channel: String,
    pub pending_queue_depth: usize,
    pub balancer_id: String,
    pub strategy: String,
    pub target_count: usize,
    pub available_target_count: usize,
    pub healthy_target_count: usize,
    pub open_circuit_count: usize,
    pub half_open_target_count: usize,
    pub last_miss_reason: Option<BalancerMissReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatcherStatus {
    pub state: DispatcherState,
    pub last_dispatch_at: Option<u64>,
    pub last_error_at: Option<u64>,
    pub queue_depth: usize,
    pub routes: IndexMap<String, DispatcherRouteStatus>,
}

impl Default for DispatcherStatus {
    fn default() -> Self {
        Self {
            state: DispatcherState::Stopped,
            last_dispatch_at: None,
            last_error_at: None,
            queue_depth: 0,
            routes: IndexMap::new(),
        }
    }
}

pub(crate) struct DispatcherRuntimeState {
    pub state: Mutex<DispatcherState>,
    pub last_dispatch_at: AtomicU64,
    pub last_error_at: AtomicU64,
    pub last_dispatch_diagnostic: Mutex<Option<DispatchDiagnosticSnapshot>>,
}

impl Default for DispatcherRuntimeState {
    fn default() -> Self {
        Self {
            state: Mutex::new(DispatcherState::Stopped),
            last_dispatch_at: AtomicU64::new(0),
            last_error_at: AtomicU64::new(0),
            last_dispatch_diagnostic: Mutex::new(None),
        }
    }
}

impl DispatcherRuntimeState {
    pub(crate) fn set_state(&self, state: DispatcherState) {
        *self.state.lock() = state;
    }

    pub(crate) fn current_state(&self) -> DispatcherState {
        *self.state.lock()
    }

    pub(crate) fn record_error(&self) {
        self.last_error_at.store(now_ms(), Ordering::SeqCst);
    }

    pub(crate) fn record_dispatch_success_timestamp(&self) {
        self.last_dispatch_at.store(now_ms(), Ordering::SeqCst);
    }

    pub(crate) fn record_dispatch_diagnostic(&self, diagnostic: DispatchDiagnosticSnapshot) {
        *self.last_dispatch_diagnostic.lock() = Some(diagnostic);
    }

    pub(crate) fn last_dispatch_diagnostic(&self) -> Option<DispatchDiagnosticSnapshot> {
        self.last_dispatch_diagnostic.lock().clone()
    }

    pub(crate) fn status(
        &self,
        balancers: &Mutex<IndexMap<String, Arc<Balancer>>>,
        queue_depth: usize,
        queue_by_route: &IndexMap<String, usize>,
    ) -> DispatcherStatus {
        let routes = balancers
            .lock()
            .iter()
            .map(|(channel, balancer)| {
                let balancer_status = balancer.balancer_status();
                let diagnostics = balancer.diagnostics_snapshot();
                (
                    channel.clone(),
                    DispatcherRouteStatus {
                        balancer_id: balancer.id().to_string(),
                        strategy: format!("{:?}", balancer.strategy()),
                        target_count: balancer_status.target_count,
                        available_target_count: balancer_status.available_count,
                        healthy_target_count: balancer_status.healthy_count,
                        open_circuit_count: balancer_status.open_circuit_count,
                        half_open_target_count: balancer_status.half_open_count,
                        pending_queue_depth: queue_by_route.get(channel).copied().unwrap_or(0),
                        last_miss_reason: diagnostics
                            .last_selection
                            .and_then(|selection| selection.miss_reason),
                    },
                )
            })
            .collect();

        DispatcherStatus {
            state: *self.state.lock(),
            last_dispatch_at: non_zero(self.last_dispatch_at.load(Ordering::SeqCst)),
            last_error_at: non_zero(self.last_error_at.load(Ordering::SeqCst)),
            queue_depth,
            routes,
        }
    }
}

fn non_zero(value: u64) -> Option<u64> {
    if value == 0 {
        None
    } else {
        Some(value)
    }
}
