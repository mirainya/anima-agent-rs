use crate::agent_types::{make_task_result, MakeTaskResult, Task, TaskResult};
use crate::agent_worker::WorkerPool;
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialistStatus {
    Active,
    Inactive,
    Overloaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalanceStrategy {
    LeastLoaded,
    RoundRobin,
    Random,
}

impl Default for LoadBalanceStrategy {
    fn default() -> Self {
        Self::LeastLoaded
    }
}

// ---------------------------------------------------------------------------
// Metrics structs
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SpecialistMetrics {
    pub tasks_completed: AtomicU64,
    pub tasks_failed: AtomicU64,
    pub total_duration_ms: AtomicU64,
}

#[derive(Debug, Clone, Default)]
pub struct SpecialistPoolMetrics {
    pub total_tasks: u64,
    pub successful_tasks: u64,
    pub failed_tasks: u64,
    pub routed_tasks: u64,
    pub fallback_tasks: u64,
}

// ---------------------------------------------------------------------------
// Specialist
// ---------------------------------------------------------------------------

pub struct Specialist {
    pub id: String,
    pub name: String,
    pub capabilities: Mutex<Vec<String>>,
    pub priority: u32,
    pub max_concurrent: usize,
    pub current_load: AtomicUsize,
    pub status: Mutex<SpecialistStatus>,
    pub metrics: SpecialistMetrics,
    pub pool: Arc<WorkerPool>,
}

// ---------------------------------------------------------------------------
// SpecialistInfo — cloneable snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SpecialistInfo {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub priority: u32,
    pub max_concurrent: usize,
    pub current_load: usize,
    pub status: SpecialistStatus,
}

impl Specialist {
    fn info(&self) -> SpecialistInfo {
        SpecialistInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            capabilities: self.capabilities.lock().unwrap().clone(),
            priority: self.priority,
            max_concurrent: self.max_concurrent,
            current_load: self.current_load.load(Ordering::SeqCst),
            status: *self.status.lock().unwrap(),
        }
    }
}

// ---------------------------------------------------------------------------
// Config & registration options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SpecialistPoolConfig {
    pub default_timeout_ms: u64,
    pub max_retries: u32,
    pub fallback_to_worker: bool,
    pub load_balance_strategy: LoadBalanceStrategy,
}

impl Default for SpecialistPoolConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 60_000,
            max_retries: 0,
            fallback_to_worker: true,
            load_balance_strategy: LoadBalanceStrategy::LeastLoaded,
        }
    }
}

pub struct RegisterSpecialistOpts {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub priority: u32,
    pub max_concurrent: usize,
    pub pool: Arc<WorkerPool>,
}

// ---------------------------------------------------------------------------
// SpecialistPool
// ---------------------------------------------------------------------------

pub struct SpecialistPool {
    default_pool: Arc<WorkerPool>,
    config: SpecialistPoolConfig,
    /// Legacy name → WorkerPool mapping (backward compat)
    specialists: Mutex<IndexMap<String, Arc<WorkerPool>>>,
    /// New registry: id → Specialist
    specialist_registry: Mutex<IndexMap<String, Arc<Specialist>>>,
    /// Capability → list of specialist ids
    capabilities: Mutex<IndexMap<String, Vec<String>>>,
    running: AtomicBool,
    round_robin_counter: AtomicU64,
    // Pool-level aggregate metrics
    pool_total_tasks: AtomicU64,
    pool_successful_tasks: AtomicU64,
    pool_failed_tasks: AtomicU64,
    pool_routed_tasks: AtomicU64,
    pool_fallback_tasks: AtomicU64,
}

impl SpecialistPool {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    pub fn new(default_pool: Arc<WorkerPool>) -> Self {
        Self::with_config(default_pool, SpecialistPoolConfig::default())
    }

    pub fn with_config(default_pool: Arc<WorkerPool>, config: SpecialistPoolConfig) -> Self {
        Self {
            default_pool,
            config,
            specialists: Mutex::new(IndexMap::new()),
            specialist_registry: Mutex::new(IndexMap::new()),
            capabilities: Mutex::new(IndexMap::new()),
            running: AtomicBool::new(false),
            round_robin_counter: AtomicU64::new(0),
            pool_total_tasks: AtomicU64::new(0),
            pool_successful_tasks: AtomicU64::new(0),
            pool_failed_tasks: AtomicU64::new(0),
            pool_routed_tasks: AtomicU64::new(0),
            pool_fallback_tasks: AtomicU64::new(0),
        }
    }

    // -----------------------------------------------------------------------
    // Legacy backward-compatible methods
    // -----------------------------------------------------------------------

    pub fn register(&self, specialist: impl Into<String>, worker_pool: Arc<WorkerPool>) {
        self.specialists
            .lock()
            .unwrap()
            .insert(specialist.into(), worker_pool);
    }

    pub fn resolve(&self, specialist: &str) -> Arc<WorkerPool> {
        self.specialists
            .lock()
            .unwrap()
            .get(specialist)
            .cloned()
            .unwrap_or_else(|| self.default_pool.clone())
    }

    pub fn execute(&self, specialist: &str, task: Task) -> TaskResult {
        let pool = self.resolve(specialist);
        let rx = pool.submit_task(task.clone());
        let result = rx.recv().unwrap_or_else(|_| {
            make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Task result channel closed".into()),
                duration_ms: 0,
                worker_id: None,
                result: None,
            })
        });

        if result.status != "success" {
            return result;
        }

        make_task_result(MakeTaskResult {
            task_id: Uuid::new_v4().to_string(),
            trace_id: result.trace_id.clone(),
            status: "success".into(),
            result: Some(json!({
                "specialist": specialist,
                "result": result.result.unwrap_or(Value::Null)
            })),
            error: None,
            duration_ms: result.duration_ms,
            worker_id: result.worker_id,
        })
    }

    // -----------------------------------------------------------------------
    // New specialist registry methods
    // -----------------------------------------------------------------------

    pub fn register_specialist(&self, opts: RegisterSpecialistOpts) {
        let specialist = Arc::new(Specialist {
            id: opts.id.clone(),
            name: opts.name,
            capabilities: Mutex::new(opts.capabilities.clone()),
            priority: opts.priority,
            max_concurrent: opts.max_concurrent,
            current_load: AtomicUsize::new(0),
            status: Mutex::new(SpecialistStatus::Active),
            metrics: SpecialistMetrics::default(),
            pool: opts.pool,
        });

        self.specialist_registry
            .lock()
            .unwrap()
            .insert(opts.id.clone(), specialist);

        // Update capability index
        let mut caps = self.capabilities.lock().unwrap();
        for cap in &opts.capabilities {
            caps.entry(cap.clone())
                .or_insert_with(Vec::new)
                .push(opts.id.clone());
        }
    }

    pub fn unregister_specialist(&self, id: &str) -> bool {
        // Step 1: lock registry to retrieve the specialist and clone its
        // capabilities, then immediately drop the registry lock.
        let spec_caps: Option<Vec<String>> = {
            let registry = self.specialist_registry.lock().unwrap();
            registry
                .get(id)
                .map(|s| s.capabilities.lock().unwrap().clone())
        };

        // Step 2: lock capabilities (no registry lock held) and clean up the
        // capability index.  This ordering — capabilities after registry is
        // released — matches the order used by pick_specialist_for, eliminating
        // the ABBA deadlock risk.
        if let Some(ref caps_list) = spec_caps {
            let mut caps = self.capabilities.lock().unwrap();
            for cap in caps_list {
                if let Some(ids) = caps.get_mut(cap) {
                    ids.retain(|sid| sid != id);
                    if ids.is_empty() {
                        caps.shift_remove(cap);
                    }
                }
            }
        }

        // Step 3: lock registry again to actually remove the specialist.
        self.specialist_registry
            .lock()
            .unwrap()
            .shift_remove(id)
            .is_some()
    }

    pub fn get_specialist(&self, id: &str) -> Option<SpecialistInfo> {
        self.specialist_registry
            .lock()
            .unwrap()
            .get(id)
            .map(|s| s.info())
    }

    pub fn list_specialists(&self) -> Vec<SpecialistInfo> {
        self.specialist_registry
            .lock()
            .unwrap()
            .values()
            .map(|s| s.info())
            .collect()
    }

    pub fn list_specialists_by_capability(&self, cap: &str) -> Vec<SpecialistInfo> {
        let caps = self.capabilities.lock().unwrap();
        let ids = match caps.get(cap) {
            Some(ids) => ids.clone(),
            None => return Vec::new(),
        };
        drop(caps);

        let registry = self.specialist_registry.lock().unwrap();
        ids.iter()
            .filter_map(|id| registry.get(id).map(|s| s.info()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Capability management
    // -----------------------------------------------------------------------

    pub fn list_capabilities(&self) -> Vec<String> {
        self.capabilities.lock().unwrap().keys().cloned().collect()
    }

    pub fn add_capability(&self, specialist_id: &str, capability: &str) {
        let registry = self.specialist_registry.lock().unwrap();
        if let Some(specialist) = registry.get(specialist_id) {
            let mut spec_caps = specialist.capabilities.lock().unwrap();
            if !spec_caps.contains(&capability.to_string()) {
                spec_caps.push(capability.to_string());
            }
        } else {
            return;
        }
        drop(registry);

        let mut caps = self.capabilities.lock().unwrap();
        let ids = caps
            .entry(capability.to_string())
            .or_insert_with(Vec::new);
        if !ids.contains(&specialist_id.to_string()) {
            ids.push(specialist_id.to_string());
        }
    }

    pub fn remove_capability(&self, specialist_id: &str, capability: &str) {
        let registry = self.specialist_registry.lock().unwrap();
        if let Some(specialist) = registry.get(specialist_id) {
            let mut spec_caps = specialist.capabilities.lock().unwrap();
            spec_caps.retain(|c| c != capability);
        }
        drop(registry);

        let mut caps = self.capabilities.lock().unwrap();
        if let Some(ids) = caps.get_mut(capability) {
            ids.retain(|sid| sid != specialist_id);
            if ids.is_empty() {
                caps.shift_remove(capability);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Routing
    // -----------------------------------------------------------------------

    pub fn route_task(&self, task: Task) -> TaskResult {
        self.pool_total_tasks.fetch_add(1, Ordering::SeqCst);

        // Look up specialists by task_type capability
        let specialist = self.pick_specialist_for(&task.task_type);

        let (pool, specialist_arc) = if let Some(spec) = specialist {
            self.pool_routed_tasks.fetch_add(1, Ordering::SeqCst);
            let pool = spec.pool.clone();
            (pool, Some(spec))
        } else if self.config.fallback_to_worker {
            self.pool_fallback_tasks.fetch_add(1, Ordering::SeqCst);
            (self.default_pool.clone(), None)
        } else {
            self.pool_failed_tasks.fetch_add(1, Ordering::SeqCst);
            return make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some(format!(
                    "No specialist found for capability '{}'",
                    task.task_type
                )),
                duration_ms: 0,
                worker_id: None,
                result: None,
            });
        };

        // Track load
        if let Some(ref spec) = specialist_arc {
            spec.current_load.fetch_add(1, Ordering::SeqCst);
        }

        let rx = pool.submit_task(task.clone());
        let result = rx.recv().unwrap_or_else(|_| {
            make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Task result channel closed".into()),
                duration_ms: 0,
                worker_id: None,
                result: None,
            })
        });

        // Update metrics
        if let Some(ref spec) = specialist_arc {
            spec.current_load.fetch_sub(1, Ordering::SeqCst);
            spec.metrics
                .total_duration_ms
                .fetch_add(result.duration_ms, Ordering::SeqCst);
            if result.status == "success" {
                spec.metrics.tasks_completed.fetch_add(1, Ordering::SeqCst);
                self.pool_successful_tasks.fetch_add(1, Ordering::SeqCst);
            } else {
                spec.metrics.tasks_failed.fetch_add(1, Ordering::SeqCst);
                self.pool_failed_tasks.fetch_add(1, Ordering::SeqCst);
            }
        } else if result.status == "success" {
            self.pool_successful_tasks.fetch_add(1, Ordering::SeqCst);
        } else {
            self.pool_failed_tasks.fetch_add(1, Ordering::SeqCst);
        }

        result
    }

    /// Pick a specialist for the given capability using the configured strategy.
    fn pick_specialist_for(&self, capability: &str) -> Option<Arc<Specialist>> {
        let caps = self.capabilities.lock().unwrap();
        let ids = caps.get(capability)?;
        if ids.is_empty() {
            return None;
        }
        let ids = ids.clone();
        drop(caps);

        let registry = self.specialist_registry.lock().unwrap();
        let candidates: Vec<Arc<Specialist>> = ids
            .iter()
            .filter_map(|id| registry.get(id))
            .filter(|s| *s.status.lock().unwrap() == SpecialistStatus::Active)
            .cloned()
            .collect();
        drop(registry);

        if candidates.is_empty() {
            return None;
        }

        match self.config.load_balance_strategy {
            LoadBalanceStrategy::LeastLoaded => {
                candidates
                    .into_iter()
                    .min_by_key(|s| s.current_load.load(Ordering::SeqCst))
            }
            LoadBalanceStrategy::RoundRobin => {
                let idx = self.round_robin_counter.fetch_add(1, Ordering::SeqCst);
                let i = (idx as usize) % candidates.len();
                Some(candidates[i].clone())
            }
            LoadBalanceStrategy::Random => {
                // Simple deterministic hash based on counter to avoid extra deps
                let idx = self.round_robin_counter.fetch_add(1, Ordering::SeqCst);
                let hash = idx.wrapping_mul(6364136223846793005).wrapping_add(1);
                let i = (hash as usize) % candidates.len();
                Some(candidates[i].clone())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    // -----------------------------------------------------------------------
    // Status / metrics / health
    // -----------------------------------------------------------------------

    pub fn status(&self) -> Value {
        let specialists: Vec<Value> = self
            .list_specialists()
            .into_iter()
            .map(|info| {
                json!({
                    "id": info.id,
                    "name": info.name,
                    "capabilities": info.capabilities,
                    "priority": info.priority,
                    "max_concurrent": info.max_concurrent,
                    "current_load": info.current_load,
                    "status": format!("{:?}", info.status),
                })
            })
            .collect();

        let m = self.metrics();
        json!({
            "running": self.is_running(),
            "specialists": specialists,
            "metrics": {
                "total_tasks": m.total_tasks,
                "successful_tasks": m.successful_tasks,
                "failed_tasks": m.failed_tasks,
                "routed_tasks": m.routed_tasks,
                "fallback_tasks": m.fallback_tasks,
            }
        })
    }

    pub fn metrics(&self) -> SpecialistPoolMetrics {
        SpecialistPoolMetrics {
            total_tasks: self.pool_total_tasks.load(Ordering::SeqCst),
            successful_tasks: self.pool_successful_tasks.load(Ordering::SeqCst),
            failed_tasks: self.pool_failed_tasks.load(Ordering::SeqCst),
            routed_tasks: self.pool_routed_tasks.load(Ordering::SeqCst),
            fallback_tasks: self.pool_fallback_tasks.load(Ordering::SeqCst),
        }
    }

    pub fn health_check(&self) -> Value {
        let specialist_count = self.specialist_registry.lock().unwrap().len();
        let capability_count = self.capabilities.lock().unwrap().len();
        let legacy_count = self.specialists.lock().unwrap().len();

        json!({
            "running": self.is_running(),
            "specialist_count": specialist_count,
            "capability_count": capability_count,
            "legacy_specialist_count": legacy_count,
            "load_balance_strategy": format!("{:?}", self.config.load_balance_strategy),
            "fallback_to_worker": self.config.fallback_to_worker,
        })
    }
}