use crate::dispatcher::router::{constant_hashing_index, round_robin_index};
use crate::support::now_ms;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetStatus {
    Available,
    Busy,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub struct TargetLoad {
    pub active: usize,
    pub total: usize,
    pub errors: usize,
    pub last_error_at: Option<u64>,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub weight: usize,
    pub capacity: usize,
    pub metadata: Value,
    pub status: TargetStatus,
    pub load: TargetLoad,
}

impl Target {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            weight: 1,
            capacity: 10,
            metadata: json!({}),
            status: TargetStatus::Available,
            load: TargetLoad::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TargetOptions {
    pub weight: Option<usize>,
    pub capacity: Option<usize>,
    pub metadata: Option<Value>,
    pub status: Option<TargetStatus>,
    pub load: Option<TargetLoad>,
}

pub fn make_target(id: Option<String>, opts: TargetOptions) -> Target {
    Target {
        id: id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        weight: opts.weight.unwrap_or(1).max(1),
        capacity: opts.capacity.unwrap_or(10),
        metadata: opts.metadata.unwrap_or_else(|| json!({})),
        status: opts.status.unwrap_or(TargetStatus::Available),
        load: opts.load.unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BalancerStrategy {
    RoundRobin,
    Weighted,
    LeastConnections,
    Hashing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: usize,
    pub success_threshold: usize,
    pub cooldown_ms: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            success_threshold: 1,
            cooldown_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthPolicy {
    pub heartbeat_ttl_ms: u64,
    pub check_on_select: bool,
}

impl Default for HealthPolicy {
    fn default() -> Self {
        Self {
            heartbeat_ttl_ms: 30_000,
            check_on_select: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BalancerRuntimeConfig {
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    pub health: Option<HealthPolicy>,
}

impl BalancerRuntimeConfig {
    pub fn enabled(&self) -> bool {
        self.circuit_breaker.is_some() || self.health.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetHealth {
    pub healthy: bool,
    pub last_checked_at: Option<u64>,
    pub last_heartbeat_at: Option<u64>,
    pub consecutive_failures: usize,
    pub consecutive_successes: usize,
    pub circuit_state: CircuitState,
    pub circuit_opened_at: Option<u64>,
}

impl Default for TargetHealth {
    fn default() -> Self {
        Self {
            healthy: true,
            last_checked_at: None,
            last_heartbeat_at: None,
            consecutive_failures: 0,
            consecutive_successes: 0,
            circuit_state: CircuitState::Closed,
            circuit_opened_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BalancerOptions {
    pub id: Option<String>,
    pub strategy: BalancerStrategy,
    pub config: Value,
    pub runtime: Option<BalancerRuntimeConfig>,
}

impl Default for BalancerOptions {
    fn default() -> Self {
        Self {
            id: None,
            strategy: BalancerStrategy::RoundRobin,
            config: json!({
                "health_check_interval_ms": 30_000,
                "retry_count": 3,
                "retry_delay_ms": 1_000,
            }),
            runtime: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadUpdate {
    Inc,
    Dec,
    Error,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TargetExcludeReason {
    StatusBusy,
    StatusOffline,
    Unhealthy,
    HeartbeatExpired,
    CircuitOpen,
    PreferClosedOverHalfOpen,
    HashingKeyMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SelectionReason {
    RoundRobin,
    WeightedRoundRobin,
    LeastConnections,
    Hashing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BalancerMissReason {
    NoTargetsConfigured,
    NoAvailableTargets,
    NoHealthyTargets,
    AllCircuitsOpen,
    HashingKeyMissing,
    NoEligibleTargets,
    StrategyReturnedNone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancerTargetDiagnostic {
    pub target_id: String,
    pub eligible: bool,
    pub excluded_reasons: Vec<TargetExcludeReason>,
    pub status: TargetStatus,
    pub healthy: bool,
    pub circuit_state: CircuitState,
    pub active: usize,
    pub weight: usize,
    pub capacity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancerSelectionDiagnostic {
    pub selection_key: Option<String>,
    pub strategy: BalancerStrategy,
    pub candidate_ids: Vec<String>,
    pub selected_target_id: Option<String>,
    pub selected_reason: Option<SelectionReason>,
    pub target_diagnostics: IndexMap<String, BalancerTargetDiagnostic>,
    pub miss_reason: Option<BalancerMissReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BalancerDiagnosticsSnapshot {
    pub last_selection: Option<BalancerSelectionDiagnostic>,
    pub miss_reason_counts: IndexMap<BalancerMissReason, usize>,
    pub exclude_reason_counts: IndexMap<TargetExcludeReason, usize>,
}

#[derive(Debug, Default)]
struct BalancerDiagnosticsState {
    last_selection: Option<BalancerSelectionDiagnostic>,
    miss_reason_counts: IndexMap<BalancerMissReason, usize>,
    exclude_reason_counts: IndexMap<TargetExcludeReason, usize>,
}

impl BalancerDiagnosticsState {
    fn snapshot(&self) -> BalancerDiagnosticsSnapshot {
        BalancerDiagnosticsSnapshot {
            last_selection: self.last_selection.clone(),
            miss_reason_counts: self.miss_reason_counts.clone(),
            exclude_reason_counts: self.exclude_reason_counts.clone(),
        }
    }

    fn record(&mut self, selection: BalancerSelectionDiagnostic) {
        for target in selection.target_diagnostics.values() {
            for reason in &target.excluded_reasons {
                *self.exclude_reason_counts.entry(*reason).or_insert(0) += 1;
            }
        }
        if let Some(reason) = selection.miss_reason {
            *self.miss_reason_counts.entry(reason).or_insert(0) += 1;
        }
        self.last_selection = Some(selection);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancerStatsSnapshot {
    pub selections: usize,
    pub failures: usize,
}

#[derive(Debug, Default)]
struct BalancerStats {
    selections: AtomicUsize,
    failures: AtomicUsize,
}

impl BalancerStats {
    fn snapshot(&self) -> BalancerStatsSnapshot {
        BalancerStatsSnapshot {
            selections: self.selections.load(Ordering::SeqCst),
            failures: self.failures.load(Ordering::SeqCst),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BalancerTargetStatus {
    pub status: TargetStatus,
    pub load: TargetLoad,
    pub weight: usize,
    pub capacity: usize,
    pub healthy: bool,
    pub last_checked_at: Option<u64>,
    pub last_heartbeat_at: Option<u64>,
    pub consecutive_failures: usize,
    pub consecutive_successes: usize,
    pub circuit_state: CircuitState,
    pub circuit_opened_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BalancerStatusSnapshot {
    pub id: String,
    pub strategy: BalancerStrategy,
    pub target_count: usize,
    pub available_count: usize,
    pub healthy_count: usize,
    pub open_circuit_count: usize,
    pub half_open_count: usize,
    pub targets: IndexMap<String, BalancerTargetStatus>,
    pub stats: BalancerStatsSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancerMetrics {
    pub total_targets: usize,
    pub available_targets: usize,
    pub busy_targets: usize,
    pub offline_targets: usize,
    pub healthy_targets: usize,
    pub unhealthy_targets: usize,
    pub open_circuits: usize,
    pub half_open_circuits: usize,
    pub total_active: usize,
    pub total_errors: usize,
}

pub struct Balancer {
    id: String,
    strategy: BalancerStrategy,
    config: Value,
    runtime: Option<BalancerRuntimeConfig>,
    targets: Mutex<IndexMap<String, Target>>,
    runtime_health: Mutex<IndexMap<String, TargetHealth>>,
    round_robin_cursor: Mutex<Option<usize>>,
    stats: BalancerStats,
    diagnostics: Mutex<BalancerDiagnosticsState>,
}

impl Default for Balancer {
    fn default() -> Self {
        Self::new(BalancerOptions::default())
    }
}

impl Balancer {
    pub fn new(opts: BalancerOptions) -> Self {
        let runtime = opts.runtime.filter(|config| config.enabled());
        Self {
            id: opts.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            strategy: opts.strategy,
            config: opts.config,
            runtime,
            targets: Mutex::new(IndexMap::new()),
            runtime_health: Mutex::new(IndexMap::new()),
            round_robin_cursor: Mutex::new(None),
            stats: BalancerStats::default(),
            diagnostics: Mutex::new(BalancerDiagnosticsState::default()),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn strategy(&self) -> BalancerStrategy {
        self.strategy
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn runtime_config(&self) -> Option<&BalancerRuntimeConfig> {
        self.runtime.as_ref()
    }

    pub fn add_target(&self, target: Target) -> Target {
        let mut targets = self.targets.lock().unwrap();
        targets.insert(target.id.clone(), target.clone());
        drop(targets);
        if self.runtime.is_some() {
            self.ensure_target_health(&target.id);
        }
        target
    }

    pub fn remove_target(&self, target_id: &str) -> Option<Target> {
        let mut targets = self.targets.lock().unwrap();
        let removed = targets.shift_remove(target_id);
        if removed.is_some() {
            self.realign_cursor(targets.len());
        }
        drop(targets);
        if removed.is_some() {
            self.runtime_health.lock().unwrap().shift_remove(target_id);
        }
        removed
    }

    pub fn get_target(&self, target_id: &str) -> Option<Target> {
        self.targets.lock().unwrap().get(target_id).cloned()
    }

    pub fn list_targets(&self) -> Vec<Target> {
        self.targets.lock().unwrap().values().cloned().collect()
    }

    pub fn list_available_targets(&self) -> Vec<Target> {
        self.targets
            .lock()
            .unwrap()
            .values()
            .filter(|target| target.status == TargetStatus::Available)
            .cloned()
            .collect()
    }

    pub fn update_load(&self, target_id: &str, op: LoadUpdate) -> Option<TargetLoad> {
        let mut targets = self.targets.lock().unwrap();
        let target = targets.get_mut(target_id)?;
        match op {
            LoadUpdate::Inc => {
                target.load.active += 1;
                target.load.total += 1;
            }
            LoadUpdate::Dec => {
                target.load.active = target.load.active.saturating_sub(1);
            }
            LoadUpdate::Error => {
                target.load.active = target.load.active.saturating_sub(1);
                target.load.errors += 1;
                target.load.last_error_at = Some(now_ms());
            }
            LoadUpdate::Reset => {
                target.load.active = 0;
            }
        }
        Some(target.load.clone())
    }

    pub fn get_load(&self, target_id: &str) -> Option<TargetLoad> {
        self.targets
            .lock()
            .unwrap()
            .get(target_id)
            .map(|target| target.load.clone())
    }

    pub fn get_active_count(&self, target_id: &str) -> Option<usize> {
        self.get_load(target_id).map(|load| load.active)
    }

    pub fn target_health(&self, target_id: &str) -> Option<TargetHealth> {
        if self.runtime.is_none() {
            return self
                .targets
                .lock()
                .unwrap()
                .get(target_id)
                .map(|_| TargetHealth::default());
        }
        self.runtime_health.lock().unwrap().get(target_id).cloned()
    }

    pub fn target_health_snapshot(&self) -> IndexMap<String, TargetHealth> {
        if self.runtime.is_none() {
            return self
                .targets
                .lock()
                .unwrap()
                .keys()
                .map(|id| (id.clone(), TargetHealth::default()))
                .collect();
        }
        self.runtime_health.lock().unwrap().clone()
    }

    pub fn diagnostics_snapshot(&self) -> BalancerDiagnosticsSnapshot {
        self.diagnostics.lock().unwrap().snapshot()
    }

    pub fn record_target_success(&self, target_id: &str) -> Option<TargetHealth> {
        let _ = self.targets.lock().unwrap().get(target_id)?;
        let now = now_ms();
        if self.runtime.is_none() {
            return Some(TargetHealth::default());
        }

        let mut runtime_health = self.runtime_health.lock().unwrap();
        let health = self.health_entry_mut(&mut runtime_health, target_id);
        health.last_checked_at = Some(now);
        health.healthy = true;
        health.consecutive_failures = 0;
        health.consecutive_successes += 1;

        if matches!(health.circuit_state, CircuitState::HalfOpen)
            && health.consecutive_successes >= self.success_threshold()
        {
            self.close_circuit(health);
        }

        Some(health.clone())
    }

    pub fn record_target_failure(&self, target_id: &str) -> Option<TargetHealth> {
        let _ = self.targets.lock().unwrap().get(target_id)?;
        let now = now_ms();
        if self.runtime.is_none() {
            return Some(TargetHealth::default());
        }

        let mut runtime_health = self.runtime_health.lock().unwrap();
        let health = self.health_entry_mut(&mut runtime_health, target_id);
        health.last_checked_at = Some(now);
        health.consecutive_failures += 1;
        health.consecutive_successes = 0;

        match health.circuit_state {
            CircuitState::HalfOpen => self.open_circuit(health, now),
            CircuitState::Closed if health.consecutive_failures >= self.failure_threshold() => {
                self.open_circuit(health, now)
            }
            CircuitState::Open | CircuitState::Closed => {}
        }

        Some(health.clone())
    }

    pub fn record_target_heartbeat(&self, target_id: &str, healthy: bool) -> Option<TargetHealth> {
        let _ = self.targets.lock().unwrap().get(target_id)?;
        let now = now_ms();
        if self.runtime.is_none() {
            return Some(TargetHealth {
                healthy,
                last_checked_at: Some(now),
                last_heartbeat_at: Some(now),
                ..TargetHealth::default()
            });
        }

        let mut runtime_health = self.runtime_health.lock().unwrap();
        let health = self.health_entry_mut(&mut runtime_health, target_id);
        health.healthy = healthy;
        health.last_checked_at = Some(now);
        health.last_heartbeat_at = Some(now);
        if healthy {
            health.consecutive_successes += 1;
        } else {
            health.consecutive_successes = 0;
        }
        Some(health.clone())
    }

    pub fn refresh_target_health(&self, target_id: &str, now_ms: u64) -> Option<TargetHealth> {
        let _ = self.targets.lock().unwrap().get(target_id)?;
        if self.runtime.is_none() {
            return Some(TargetHealth::default());
        }

        let mut runtime_health = self.runtime_health.lock().unwrap();
        let health = self.health_entry_mut(&mut runtime_health, target_id);
        health.last_checked_at = Some(now_ms);

        if matches!(health.circuit_state, CircuitState::Open)
            && self.cooldown_elapsed(health, now_ms)
        {
            health.circuit_state = CircuitState::HalfOpen;
            health.circuit_opened_at = None;
            health.consecutive_successes = 0;
        }

        if self.heartbeat_expired(health, now_ms) {
            health.healthy = false;
        }

        Some(health.clone())
    }

    pub fn refresh_targets(&self, now_ms: u64) {
        if self.runtime.is_none() {
            return;
        }
        let target_ids: Vec<String> = self.targets.lock().unwrap().keys().cloned().collect();
        for target_id in target_ids {
            let _ = self.refresh_target_health(&target_id, now_ms);
        }
    }

    pub fn is_target_routable(&self, target_id: &str, now_ms: u64) -> bool {
        let Some(target) = self.get_target(target_id) else {
            return false;
        };
        if target.status != TargetStatus::Available {
            return false;
        }
        let Some(health) = self.refresh_target_health(target_id, now_ms) else {
            return false;
        };
        health.circuit_state != CircuitState::Open && health.healthy
    }

    pub fn set_target_status(&self, target_id: &str, status: TargetStatus) -> Option<TargetStatus> {
        let mut targets = self.targets.lock().unwrap();
        let target = targets.get_mut(target_id)?;
        target.status = status;
        Some(target.status)
    }

    pub fn mark_target_available(&self, target_id: &str) -> Option<TargetStatus> {
        self.set_target_status(target_id, TargetStatus::Available)
    }

    pub fn mark_target_busy(&self, target_id: &str) -> Option<TargetStatus> {
        self.set_target_status(target_id, TargetStatus::Busy)
    }

    pub fn mark_target_offline(&self, target_id: &str) -> Option<TargetStatus> {
        self.set_target_status(target_id, TargetStatus::Offline)
    }

    pub fn select_target(&self, key: Option<&str>) -> Option<Target> {
        let targets = self.targets.lock().unwrap().clone();
        let selection = self.compute_selection(targets, key);

        if selection.selected_target.is_some() {
            self.stats.selections.fetch_add(1, Ordering::SeqCst);
        } else {
            self.stats.failures.fetch_add(1, Ordering::SeqCst);
        }

        self.diagnostics
            .lock()
            .unwrap()
            .record(selection.diagnostic.clone());

        selection.selected_target
    }

    pub fn balancer_status(&self) -> BalancerStatusSnapshot {
        let targets = self.targets.lock().unwrap().clone();
        let runtime_health = self.target_health_snapshot();
        let snapshot_targets: IndexMap<String, BalancerTargetStatus> = targets
            .iter()
            .map(|(id, target)| {
                let health = runtime_health.get(id).cloned().unwrap_or_default();
                (
                    id.clone(),
                    BalancerTargetStatus {
                        status: target.status,
                        load: target.load.clone(),
                        weight: target.weight,
                        capacity: target.capacity,
                        healthy: health.healthy,
                        last_checked_at: health.last_checked_at,
                        last_heartbeat_at: health.last_heartbeat_at,
                        consecutive_failures: health.consecutive_failures,
                        consecutive_successes: health.consecutive_successes,
                        circuit_state: health.circuit_state,
                        circuit_opened_at: health.circuit_opened_at,
                    },
                )
            })
            .collect();

        let healthy_count = snapshot_targets.values().filter(|target| target.healthy).count();
        let open_circuit_count = snapshot_targets
            .values()
            .filter(|target| target.circuit_state == CircuitState::Open)
            .count();
        let half_open_count = snapshot_targets
            .values()
            .filter(|target| target.circuit_state == CircuitState::HalfOpen)
            .count();

        BalancerStatusSnapshot {
            id: self.id.clone(),
            strategy: self.strategy,
            target_count: targets.len(),
            available_count: targets
                .values()
                .filter(|target| target.status == TargetStatus::Available)
                .count(),
            healthy_count,
            open_circuit_count,
            half_open_count,
            targets: snapshot_targets,
            stats: self.stats.snapshot(),
        }
    }

    pub fn balancer_metrics(&self) -> BalancerMetrics {
        let targets = self.targets.lock().unwrap().clone();
        let runtime_health = self.target_health_snapshot();
        let healthy_targets = targets
            .keys()
            .filter(|target_id| runtime_health.get(*target_id).map(|health| health.healthy).unwrap_or(true))
            .count();
        let open_circuits = runtime_health
            .values()
            .filter(|health| health.circuit_state == CircuitState::Open)
            .count();
        let half_open_circuits = runtime_health
            .values()
            .filter(|health| health.circuit_state == CircuitState::HalfOpen)
            .count();

        BalancerMetrics {
            total_targets: targets.len(),
            available_targets: targets
                .values()
                .filter(|target| target.status == TargetStatus::Available)
                .count(),
            busy_targets: targets
                .values()
                .filter(|target| target.status == TargetStatus::Busy)
                .count(),
            offline_targets: targets
                .values()
                .filter(|target| target.status == TargetStatus::Offline)
                .count(),
            healthy_targets,
            unhealthy_targets: targets.len().saturating_sub(healthy_targets),
            open_circuits,
            half_open_circuits,
            total_active: targets.values().map(|target| target.load.active).sum(),
            total_errors: targets.values().map(|target| target.load.errors).sum(),
        }
    }

    fn compute_selection(
        &self,
        targets: IndexMap<String, Target>,
        key: Option<&str>,
    ) -> BalancerSelectionComputation {
        let now = now_ms();
        if self
            .health_policy()
            .map(|policy| policy.check_on_select)
            .unwrap_or(false)
        {
            self.refresh_targets(now);
        }

        let runtime_health = self.target_health_snapshot();
        let mut available_status_count = 0;
        let mut healthy_available_count = 0;
        let mut open_circuit_available_count = 0;
        let mut closed_candidate_ids = Vec::new();
        let mut half_open_candidate_ids = Vec::new();

        for target in targets.values() {
            if target.status != TargetStatus::Available {
                continue;
            }
            available_status_count += 1;

            let health = runtime_health.get(&target.id).cloned().unwrap_or_default();
            let heartbeat_expired = self.heartbeat_expired(&health, now);
            let healthy = if self.runtime.is_some() {
                health.healthy && !heartbeat_expired
            } else {
                true
            };

            if !healthy {
                continue;
            }

            healthy_available_count += 1;
            match health.circuit_state {
                CircuitState::Closed => closed_candidate_ids.push(target.id.clone()),
                CircuitState::HalfOpen => half_open_candidate_ids.push(target.id.clone()),
                CircuitState::Open => open_circuit_available_count += 1,
            }
        }

        let prefer_closed = !closed_candidate_ids.is_empty();
        let candidate_ids = if prefer_closed {
            closed_candidate_ids.clone()
        } else {
            half_open_candidate_ids.clone()
        };
        let hashing_key_missing =
            matches!(self.strategy, BalancerStrategy::Hashing) && key.is_none() && !candidate_ids.is_empty();

        let candidate_set: std::collections::HashSet<&str> =
            candidate_ids.iter().map(String::as_str).collect();
        let closed_set: std::collections::HashSet<&str> =
            closed_candidate_ids.iter().map(String::as_str).collect();

        let target_diagnostics = targets
            .iter()
            .map(|(target_id, target)| {
                let health = runtime_health.get(target_id).cloned().unwrap_or_default();
                let excluded_reasons = self.target_excluded_reasons(
                    target,
                    &health,
                    now,
                    prefer_closed,
                    &closed_set,
                    &candidate_set,
                    hashing_key_missing,
                );
                let heartbeat_expired = self.heartbeat_expired(&health, now);
                let healthy = if self.runtime.is_some() {
                    health.healthy && !heartbeat_expired
                } else {
                    true
                };
                (
                    target_id.clone(),
                    BalancerTargetDiagnostic {
                        target_id: target_id.clone(),
                        eligible: excluded_reasons.is_empty(),
                        excluded_reasons,
                        status: target.status,
                        healthy,
                        circuit_state: health.circuit_state,
                        active: target.load.active,
                        weight: target.weight,
                        capacity: target.capacity,
                    },
                )
            })
            .collect::<IndexMap<_, _>>();

        let candidates = candidate_ids
            .iter()
            .filter_map(|target_id| targets.get(target_id).cloned())
            .collect::<Vec<_>>();

        let selected_target = match self.strategy {
            BalancerStrategy::RoundRobin => self.select_round_robin(&candidates),
            BalancerStrategy::Weighted => self.select_weighted(&candidates),
            BalancerStrategy::LeastConnections => self.select_least_connections(&candidates),
            BalancerStrategy::Hashing => self.select_hashing(&candidates, key),
        };

        let selected_reason = selected_target
            .as_ref()
            .map(|_| self.selection_reason_for_strategy());
        let miss_reason = if selected_target.is_none() {
            Some(self.determine_miss_reason(
                targets.len(),
                available_status_count,
                healthy_available_count,
                open_circuit_available_count,
                &candidate_ids,
                key,
            ))
        } else {
            None
        };

        let diagnostic = BalancerSelectionDiagnostic {
            selection_key: key.map(ToString::to_string),
            strategy: self.strategy,
            candidate_ids,
            selected_target_id: selected_target.as_ref().map(|target| target.id.clone()),
            selected_reason,
            target_diagnostics,
            miss_reason,
        };

        BalancerSelectionComputation {
            selected_target,
            diagnostic,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn target_excluded_reasons(
        &self,
        target: &Target,
        health: &TargetHealth,
        now: u64,
        prefer_closed: bool,
        closed_set: &std::collections::HashSet<&str>,
        candidate_set: &std::collections::HashSet<&str>,
        hashing_key_missing: bool,
    ) -> Vec<TargetExcludeReason> {
        let mut reasons = Vec::new();

        match target.status {
            TargetStatus::Busy => reasons.push(TargetExcludeReason::StatusBusy),
            TargetStatus::Offline => reasons.push(TargetExcludeReason::StatusOffline),
            TargetStatus::Available => {}
        }

        if target.status == TargetStatus::Available && self.runtime.is_some() {
            let heartbeat_expired = self.heartbeat_expired(health, now);
            if heartbeat_expired {
                reasons.push(TargetExcludeReason::HeartbeatExpired);
            } else if !health.healthy {
                reasons.push(TargetExcludeReason::Unhealthy);
            }

            if health.circuit_state == CircuitState::Open {
                reasons.push(TargetExcludeReason::CircuitOpen);
            }

            if prefer_closed
                && health.circuit_state == CircuitState::HalfOpen
                && !closed_set.contains(target.id.as_str())
            {
                reasons.push(TargetExcludeReason::PreferClosedOverHalfOpen);
            }
        }

        if target.status == TargetStatus::Available
            && hashing_key_missing
            && candidate_set.contains(target.id.as_str())
        {
            reasons.push(TargetExcludeReason::HashingKeyMissing);
        }

        reasons
    }

    fn determine_miss_reason(
        &self,
        total_targets: usize,
        available_status_count: usize,
        healthy_available_count: usize,
        open_circuit_available_count: usize,
        candidate_ids: &[String],
        key: Option<&str>,
    ) -> BalancerMissReason {
        if total_targets == 0 {
            return BalancerMissReason::NoTargetsConfigured;
        }
        if matches!(self.strategy, BalancerStrategy::Hashing) && key.is_none() && !candidate_ids.is_empty() {
            return BalancerMissReason::HashingKeyMissing;
        }
        if available_status_count == 0 {
            return BalancerMissReason::NoAvailableTargets;
        }
        if self.runtime.is_some() {
            if healthy_available_count == 0 {
                return BalancerMissReason::NoHealthyTargets;
            }
            if candidate_ids.is_empty() && open_circuit_available_count > 0 {
                return BalancerMissReason::AllCircuitsOpen;
            }
            if candidate_ids.is_empty() {
                return BalancerMissReason::NoEligibleTargets;
            }
        } else if candidate_ids.is_empty() {
            return BalancerMissReason::NoAvailableTargets;
        }
        BalancerMissReason::StrategyReturnedNone
    }

    fn selection_reason_for_strategy(&self) -> SelectionReason {
        match self.strategy {
            BalancerStrategy::RoundRobin => SelectionReason::RoundRobin,
            BalancerStrategy::Weighted => SelectionReason::WeightedRoundRobin,
            BalancerStrategy::LeastConnections => SelectionReason::LeastConnections,
            BalancerStrategy::Hashing => SelectionReason::Hashing,
        }
    }

    fn select_round_robin(&self, targets: &[Target]) -> Option<Target> {
        let mut cursor = self.round_robin_cursor.lock().unwrap();
        let next = round_robin_index(*cursor, targets.len())?;
        *cursor = Some(next);
        targets.get(next).cloned()
    }

    fn select_weighted(&self, targets: &[Target]) -> Option<Target> {
        let weighted: Vec<Target> = targets
            .iter()
            .flat_map(|target| std::iter::repeat_n(target.clone(), target.weight.max(1)))
            .collect();
        let mut cursor = self.round_robin_cursor.lock().unwrap();
        let next = round_robin_index(*cursor, weighted.len())?;
        *cursor = Some(next);
        weighted.get(next).cloned()
    }

    fn select_least_connections(&self, targets: &[Target]) -> Option<Target> {
        targets
            .iter()
            .min_by_key(|target| target.load.active)
            .cloned()
    }

    fn select_hashing(&self, targets: &[Target], key: Option<&str>) -> Option<Target> {
        let hashing_key = key?;
        let index = constant_hashing_index(hashing_key, targets.len())?;
        targets.get(index).cloned()
    }

    fn realign_cursor(&self, len: usize) {
        let mut cursor = self.round_robin_cursor.lock().unwrap();
        *cursor = if len == 0 {
            None
        } else {
            cursor.map(|index| index.min(len.saturating_sub(1)))
        };
    }

    fn ensure_target_health(&self, target_id: &str) {
        let mut runtime_health = self.runtime_health.lock().unwrap();
        runtime_health
            .entry(target_id.to_string())
            .or_insert_with(|| self.initial_target_health());
    }

    fn initial_target_health(&self) -> TargetHealth {
        let now = now_ms();
        let mut health = TargetHealth {
            last_checked_at: Some(now),
            ..TargetHealth::default()
        };
        if self.health_policy().is_some() {
            health.last_heartbeat_at = Some(now);
        }
        health
    }

    fn health_entry_mut<'a>(
        &self,
        runtime_health: &'a mut IndexMap<String, TargetHealth>,
        target_id: &str,
    ) -> &'a mut TargetHealth {
        runtime_health
            .entry(target_id.to_string())
            .or_insert_with(|| self.initial_target_health())
    }

    fn failure_threshold(&self) -> usize {
        self.runtime
            .as_ref()
            .and_then(|config| config.circuit_breaker.as_ref())
            .map(|config| config.failure_threshold.max(1))
            .unwrap_or(usize::MAX)
    }

    fn success_threshold(&self) -> usize {
        self.runtime
            .as_ref()
            .and_then(|config| config.circuit_breaker.as_ref())
            .map(|config| config.success_threshold.max(1))
            .unwrap_or(usize::MAX)
    }

    fn health_policy(&self) -> Option<&HealthPolicy> {
        self.runtime.as_ref().and_then(|config| config.health.as_ref())
    }

    fn heartbeat_expired(&self, health: &TargetHealth, now_ms: u64) -> bool {
        let Some(policy) = self.health_policy() else {
            return false;
        };
        let Some(last_heartbeat_at) = health.last_heartbeat_at else {
            return false;
        };
        let reference_now = health.last_checked_at.unwrap_or(now_ms).max(now_ms);
        reference_now.saturating_sub(last_heartbeat_at) > policy.heartbeat_ttl_ms
    }

    fn cooldown_elapsed(&self, health: &TargetHealth, now_ms: u64) -> bool {
        let Some(circuit_breaker) = self
            .runtime
            .as_ref()
            .and_then(|config| config.circuit_breaker.as_ref())
        else {
            return false;
        };
        let Some(opened_at) = health.circuit_opened_at else {
            return false;
        };
        now_ms.saturating_sub(opened_at) >= circuit_breaker.cooldown_ms
    }

    fn open_circuit(&self, health: &mut TargetHealth, now_ms: u64) {
        health.circuit_state = CircuitState::Open;
        health.circuit_opened_at = Some(now_ms);
        health.consecutive_successes = 0;
    }

    fn close_circuit(&self, health: &mut TargetHealth) {
        health.circuit_state = CircuitState::Closed;
        health.circuit_opened_at = None;
        health.consecutive_failures = 0;
    }
}

struct BalancerSelectionComputation {
    selected_target: Option<Target>,
    diagnostic: BalancerSelectionDiagnostic,
}

