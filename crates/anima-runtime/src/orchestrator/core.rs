//! 任务编排器模块
//!
//! 负责将复杂请求分解为子任务并协调执行。核心概念：
//! - 通过正则匹配的分解规则（DecompositionRule）将请求拆分为子任务
//! - 子任务之间可声明依赖关系，编排器自动进行拓扑排序和并行分组
//! - 同一组内无依赖的子任务可并行执行，不同组按依赖顺序串行
//! - 同时提供静态方法 `execute_plan` / `execute_single_task` 用于直接执行 ExecutionPlan

use crate::agent::detect_pending_question;
use crate::agent::runtime_error::AgentError;
use crate::provider::Provider;
use crate::agent::types::{
    make_task, make_task_result, ExecutionPlan, ExecutionPlanKind, MakeTask, MakeTaskResult, Task,
    TaskResult,
};
use crate::worker::WorkerPool;
use crate::orchestrator::parallel_pool::ParallelPool;
use crate::orchestrator::specialist_pool::SpecialistPool;
use crate::runtime::{RuntimeDomainEvent, SharedRuntimeStateStore};
use crate::support::now_ms;
use crate::tasks::query::{plan_task, subtasks_for_plan};
use crate::tasks::{TaskKind, TaskRecord, TaskStatus};
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::Mutex;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct WaitDiagnosticContext {
    plan_id: String,
    parent_job_id: String,
    subtask_id: String,
    subtask_name: String,
    task_id: String,
    task_type: String,
    execution_mode: String,
    parallel_group_index: usize,
    parallel_group_size: usize,
    wait_reason: String,
}

// ── SubTask ───────────────────────────────────────────────────────────

/// 子任务：编排计划中的最小执行单元
///
/// 每个子任务声明自己的依赖（dependencies）、所需专家类型（specialist_type），
/// 运行时状态和结果通过 Mutex 保护以支持并发更新。
pub struct SubTask {
    pub id: String,
    pub parent_id: String,
    pub parent_job_id: String,
    pub trace_id: String,
    pub name: String,
    pub task_type: String,
    pub description: String,
    pub dependencies: HashSet<String>,
    pub priority: u32,
    pub specialist_type: String,
    pub payload: Value,
    pub result: Mutex<Option<TaskResult>>,
    pub started_at: Mutex<Option<u64>>,
    pub completed_at: Mutex<Option<u64>>,
}

// ── OrchestrationPlan ─────────────────────────────────────────────────

/// 编排计划的执行进度
#[derive(Debug, Clone, Default)]
pub struct PlanProgress {
    pub completed_count: u32,
    pub total_count: u32,
    pub failed_count: u32,
}

/// 编排计划：包含子任务集合、拓扑执行顺序和并行分组
///
/// `execution_order` 是拓扑排序后的线性顺序，
/// `parallel_groups` 将其进一步划分为可并行执行的批次。
pub struct OrchestrationPlan {
    pub id: String,
    pub trace_id: String,
    pub parent_job_id: String,
    pub original_request: String,
    pub matched_rule: Option<String>,
    pub subtasks: IndexMap<String, Arc<SubTask>>,
    pub execution_order: Vec<String>,
    pub parallel_groups: Vec<Vec<String>>,
    pub progress: Mutex<PlanProgress>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringPrimitive {
    ApiCall,
    Query,
    Transform,
}

impl LoweringPrimitive {
    fn as_task_type(self) -> &'static str {
        match self {
            Self::ApiCall => "api-call",
            Self::Query => "query",
            Self::Transform => "transform",
        }
    }

    fn result_kind(self) -> &'static str {
        match self {
            Self::ApiCall => "upstream",
            Self::Query => "query",
            Self::Transform => "transform",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredTask {
    pub name: String,
    pub original_task_type: String,
    pub lowered_task_type: String,
    pub primitive: LoweringPrimitive,
    pub parallel_safe: bool,
    pub task: Task,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationExecutionResult {
    pub result: TaskResult,
    pub lowered_tasks: Vec<LoweredTask>,
    pub was_decomposed: bool,
}

#[derive(Debug, Clone)]
struct OrchestrationExecutionContext {
    request: Value,
    parent_job_id: String,
    plan_id: String,
    session_id: String,
    subtask_results: Value,
}

impl OrchestrationExecutionContext {
    fn new(request: &str, parent_job_id: &str, plan_id: &str, session_id: &str) -> Self {
        Self {
            request: Value::String(request.to_string()),
            parent_job_id: parent_job_id.to_string(),
            plan_id: plan_id.to_string(),
            session_id: session_id.to_string(),
            subtask_results: json!({}),
        }
    }

    fn as_value(&self) -> Value {
        json!({
            "request": self.request.clone(),
            "parent_job_id": self.parent_job_id.clone(),
            "plan_id": self.plan_id.clone(),
            "session_id": self.session_id.clone(),
            "subtask_results": self.subtask_results.clone(),
        })
    }
}

// ── Config / Metrics ──────────────────────────────────────────────────

/// 编排器配置
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub default_timeout_ms: u64,
    pub max_retries: u32,
    pub enable_parallel: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 60_000,
            max_retries: 0,
            enable_parallel: true,
        }
    }
}

/// 编排器累计指标
#[derive(Debug, Clone, Default)]
pub struct OrchestratorMetrics {
    pub plans_created: u64,
    pub plans_completed: u64,
    pub plans_failed: u64,
    pub subtasks_executed: u64,
    pub subtasks_failed: u64,
    pub total_duration_ms: u64,
}

// ── AgentOrchestrator ─────────────────────────────────────────────────

/// 任务编排器：将复杂请求分解为子任务，按依赖关系编排执行
///
/// 核心流程：decompose_task → topological_sort → compute_parallel_groups → execute
pub struct AgentOrchestrator {
    id: String,
    specialist_pool: Arc<SpecialistPool>,
    worker_pool: Arc<WorkerPool>,
    runtime_state_store: SharedRuntimeStateStore,
    config: OrchestratorConfig,
    running: AtomicBool,
    metrics: Mutex<OrchestratorMetrics>,
    pub(crate) provider: parking_lot::RwLock<Option<Arc<dyn Provider>>>,
    pub(crate) prompts: parking_lot::RwLock<Arc<anima_types::config::PromptsConfig>>,
}

impl AgentOrchestrator {
    pub fn new(
        worker_pool: Arc<WorkerPool>,
        specialist_pool: Arc<SpecialistPool>,
        runtime_state_store: SharedRuntimeStateStore,
        config: OrchestratorConfig,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            specialist_pool,
            worker_pool,
            runtime_state_store,
            config,
            running: AtomicBool::new(false),
            metrics: Mutex::new(OrchestratorMetrics::default()),
            provider: parking_lot::RwLock::new(None),
            prompts: parking_lot::RwLock::new(Arc::new(anima_types::config::PromptsConfig::default())),
        }
    }

    pub fn with_llm(self, provider: Arc<dyn Provider>) -> Self {
        *self.provider.write() = Some(provider);
        self
    }

    pub fn set_provider(&self, provider: Arc<dyn Provider>) {
        *self.provider.write() = Some(provider);
    }

    pub fn set_prompts(&self, prompts: Arc<anima_types::config::PromptsConfig>) {
        *self.prompts.write() = prompts;
    }

    fn runtime_run_id(job_id: &str) -> String {
        format!("run_{job_id}")
    }

    fn runtime_plan_task_id(plan_id: &str) -> String {
        format!("task_plan_{plan_id}")
    }

    fn runtime_subtask_task_id(subtask_id: &str) -> String {
        format!("task_subtask_{subtask_id}")
    }

    fn current_turn_id_for_job(&self, job_id: &str) -> Option<String> {
        self.runtime_state_store
            .snapshot()
            .runs
            .get(&Self::runtime_run_id(job_id))
            .and_then(|run| run.current_turn_id.clone())
    }

    fn upsert_runtime_plan_task(
        &self,
        plan: &OrchestrationPlan,
        status: TaskStatus,
        error: Option<String>,
    ) {
        let updated_at_ms = now_ms();
        let snapshot = self.runtime_state_store.snapshot();
        let task_id = Self::runtime_plan_task_id(&plan.id);
        let existing = snapshot.tasks.get(&task_id);
        self.runtime_state_store
            .append(RuntimeDomainEvent::TaskUpserted {
                task: TaskRecord {
                    task_id: task_id.clone(),
                    run_id: Self::runtime_run_id(&plan.parent_job_id),
                    turn_id: self.current_turn_id_for_job(&plan.parent_job_id),
                    parent_task_id: None,
                    trace_id: plan.trace_id.clone(),
                    job_id: plan.parent_job_id.clone(),
                    parent_job_id: None,
                    plan_id: Some(plan.id.clone()),
                    kind: TaskKind::Plan,
                    name: "orchestration_plan".into(),
                    task_type: "orchestration_plan".into(),
                    description: plan.original_request.clone(),
                    status: status.clone(),
                    execution_mode: None,
                    result_kind: Some("orchestration_plan".into()),
                    specialist_type: plan.matched_rule.clone(),
                    dependencies: vec![],
                    metadata: json!({
                        "matched_rule": plan.matched_rule.clone(),
                        "execution_order": plan.execution_order.clone(),
                        "parallel_groups": plan.parallel_groups.clone(),
                    }),
                    started_at_ms: existing
                        .and_then(|task| task.started_at_ms)
                        .or(Some(plan.created_at)),
                    updated_at_ms,
                    completed_at_ms: match status {
                        TaskStatus::Completed | TaskStatus::Failed => Some(updated_at_ms),
                        _ => existing.and_then(|task| task.completed_at_ms),
                    },
                    error,
                },
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_runtime_subtask_task(
        &self,
        subtask: &SubTask,
        lowered: &LoweredTask,
        status: TaskStatus,
        execution_mode: Option<&str>,
        parallel_group_index: Option<usize>,
        parallel_group_size: Option<usize>,
        error: Option<String>,
    ) {
        let updated_at_ms = now_ms();
        let snapshot = self.runtime_state_store.snapshot();
        let task_id = Self::runtime_subtask_task_id(&subtask.id);
        let existing = snapshot.tasks.get(&task_id);
        self.runtime_state_store
            .append(RuntimeDomainEvent::TaskUpserted {
                task: TaskRecord {
                    task_id: task_id.clone(),
                    run_id: Self::runtime_run_id(&subtask.parent_job_id),
                    turn_id: self.current_turn_id_for_job(&subtask.parent_job_id),
                    parent_task_id: Some(Self::runtime_plan_task_id(&subtask.parent_id)),
                    trace_id: subtask.trace_id.clone(),
                    job_id: subtask.parent_job_id.clone(),
                    parent_job_id: Some(subtask.parent_job_id.clone()),
                    plan_id: Some(subtask.parent_id.clone()),
                    kind: TaskKind::Subtask,
                    name: subtask.name.clone(),
                    task_type: lowered.original_task_type.clone(),
                    description: subtask.description.clone(),
                    status: status.clone(),
                    execution_mode: execution_mode.map(ToString::to_string),
                    result_kind: Some(lowered.primitive.result_kind().to_string()),
                    specialist_type: if subtask.specialist_type.is_empty() {
                        None
                    } else {
                        Some(subtask.specialist_type.clone())
                    },
                    dependencies: subtask.dependencies.iter().cloned().collect(),
                    metadata: json!({
                        "subtask_id": subtask.id.clone(),
                        "parallel_group_index": parallel_group_index,
                        "parallel_group_size": parallel_group_size,
                        "original_task_type": lowered.original_task_type.clone(),
                        "lowered_task_type": lowered.lowered_task_type.clone(),
                        "parallel_safe": lowered.parallel_safe,
                        "worker_task_id": lowered.task.id.clone(),
                    }),
                    started_at_ms: existing
                        .and_then(|task| task.started_at_ms)
                        .or(*subtask.started_at.lock()),
                    updated_at_ms,
                    completed_at_ms: match status {
                        TaskStatus::Completed | TaskStatus::Failed => Some(updated_at_ms),
                        _ => existing.and_then(|task| task.completed_at_ms),
                    },
                    error,
                },
            });
    }

    fn runtime_active_subtask_ids(&self, plan_id: &str) -> Vec<String> {
        let snapshot = self.runtime_state_store.snapshot();
        subtasks_for_plan(&snapshot, plan_id)
            .into_iter()
            .filter(|task| {
                matches!(
                    task.status,
                    TaskStatus::Pending | TaskStatus::Running | TaskStatus::Suspended
                )
            })
            .map(|task| task.task_id.clone())
            .collect()
    }

    fn runtime_plan_progress_counts(&self, plan_id: &str) -> (usize, usize, usize) {
        let snapshot = self.runtime_state_store.snapshot();
        let subtasks = subtasks_for_plan(&snapshot, plan_id);
        let total = subtasks.len();
        let completed = subtasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Completed))
            .count();
        let failed = subtasks
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::Failed))
            .count();
        (total, completed, failed)
    }

    fn runtime_plan_status(&self, plan_id: &str) -> Option<TaskStatus> {
        let snapshot = self.runtime_state_store.snapshot();
        plan_task(&snapshot, plan_id).map(|task| task.status.clone())
    }

    fn runtime_active_plan_count(&self) -> usize {
        let snapshot = self.runtime_state_store.snapshot();
        snapshot
            .tasks
            .values()
            .filter(|task| {
                task.kind == TaskKind::Plan
                    && matches!(
                        task.status,
                        TaskStatus::Pending | TaskStatus::Running | TaskStatus::Suspended
                    )
            })
            .count()
    }

    fn runtime_plan_is_active(&self, plan_id: &str) -> bool {
        matches!(
            self.runtime_plan_status(plan_id),
            Some(TaskStatus::Pending | TaskStatus::Running | TaskStatus::Suspended)
        )
    }

    fn update_plan_runtime_status(
        &self,
        plan: &OrchestrationPlan,
        status: TaskStatus,
        error: Option<String>,
    ) {
        self.upsert_runtime_plan_task(plan, status, error);
    }

    #[allow(clippy::too_many_arguments)]
    fn update_subtask_runtime_status(
        &self,
        subtask: &SubTask,
        lowered: &LoweredTask,
        status: TaskStatus,
        execution_mode: Option<&str>,
        parallel_group_index: Option<usize>,
        parallel_group_size: Option<usize>,
        error: Option<String>,
    ) {
        self.upsert_runtime_subtask_task(
            subtask,
            lowered,
            status,
            execution_mode,
            parallel_group_index,
            parallel_group_size,
            error,
        );
    }

    // ── Lifecycle ─────────────────────────────────────────────────

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    // ── Status / Metrics ──────────────────────────────────────────

    pub fn status(&self) -> Value {
        let m = self.metrics.lock();
        json!({
            "id": self.id,
            "running": self.is_running(),
            "active_plans": self.runtime_active_plan_count(),
            "config": {
                "default_timeout_ms": self.config.default_timeout_ms,
                "max_retries": self.config.max_retries,
                "enable_parallel": self.config.enable_parallel,
            },
            "metrics": {
                "plans_created": m.plans_created,
                "plans_completed": m.plans_completed,
                "plans_failed": m.plans_failed,
                "subtasks_executed": m.subtasks_executed,
                "subtasks_failed": m.subtasks_failed,
                "total_duration_ms": m.total_duration_ms,
            }
        })
    }

    pub fn metrics(&self) -> OrchestratorMetrics {
        self.metrics
            .lock()
            .clone()
    }

    // ── Task Decomposition ────────────────────────────────────────

    /// 将请求文本分解为编排计划
    ///
    /// LLM 分解优先，失败时创建单个 generic 子任务兜底。
    pub fn decompose_task(
        &self,
        request: &str,
        trace_id: &str,
        parent_job_id: &str,
        session_id: Option<&str>,
    ) -> OrchestrationPlan {
        if let Some(sid) = session_id {
            if let Some(specs) = self.try_llm_decompose(request, sid) {
                return self.build_plan_from_llm_specs(
                    specs,
                    request,
                    trace_id,
                    parent_job_id,
                );
            }
        }

        let plan_id = Uuid::new_v4().to_string();
        let mut subtasks: IndexMap<String, Arc<SubTask>> = IndexMap::new();
        let sub_id = Uuid::new_v4().to_string();
        let subtask = Arc::new(SubTask {
            id: sub_id.clone(),
            parent_id: plan_id.clone(),
            parent_job_id: parent_job_id.to_string(),
            trace_id: trace_id.to_string(),
            name: "generic".into(),
            task_type: "generic".into(),
            description: format!("Generic task: {}", request),
            dependencies: HashSet::new(),
            priority: 5,
            specialist_type: "default".into(),
            payload: json!({ "request": request }),
            result: Mutex::new(None),
            started_at: Mutex::new(None),
            completed_at: Mutex::new(None),
        });
        subtasks.insert("generic".into(), subtask);

        let execution_order = Self::topological_sort(&subtasks);
        let parallel_groups = Self::compute_parallel_groups(&subtasks, &execution_order);

        let total = subtasks.len() as u32;
        OrchestrationPlan {
            id: plan_id,
            trace_id: trace_id.to_string(),
            parent_job_id: parent_job_id.to_string(),
            original_request: request.to_string(),
            matched_rule: None,
            subtasks,
            execution_order,
            parallel_groups,
            progress: Mutex::new(PlanProgress {
                completed_count: 0,
                total_count: total,
                failed_count: 0,
            }),
            created_at: now_ms(),
        }
    }
    /// 拓扑排序：基于子任务依赖关系生成线性执行顺序（DFS 后序）
    pub(crate) fn topological_sort(subtasks: &IndexMap<String, Arc<SubTask>>) -> Vec<String> {
        let mut order: Vec<String> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut visiting: HashSet<String> = HashSet::new();

        fn visit(
            name: &str,
            subtasks: &IndexMap<String, Arc<SubTask>>,
            visited: &mut HashSet<String>,
            visiting: &mut HashSet<String>,
            order: &mut Vec<String>,
        ) {
            if visited.contains(name) || visiting.contains(name) {
                return;
            }
            visiting.insert(name.to_string());
            if let Some(st) = subtasks.get(name) {
                for dep in &st.dependencies {
                    visit(dep, subtasks, visited, visiting, order);
                }
            }
            visiting.remove(name);
            visited.insert(name.to_string());
            order.push(name.to_string());
        }

        for name in subtasks.keys() {
            visit(name, subtasks, &mut visited, &mut visiting, &mut order);
        }
        order
    }

    /// 计算并行分组：将拓扑序列划分为多个批次，同批次内的子任务无相互依赖可并行执行
    pub(crate) fn compute_parallel_groups(
        subtasks: &IndexMap<String, Arc<SubTask>>,
        execution_order: &[String],
    ) -> Vec<Vec<String>> {
        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut completed: HashSet<String> = HashSet::new();
        let mut remaining: Vec<String> = execution_order.to_vec();

        while !remaining.is_empty() {
            let mut group: Vec<String> = Vec::new();
            let mut still_remaining: Vec<String> = Vec::new();

            for name in &remaining {
                if let Some(st) = subtasks.get(name) {
                    let deps_satisfied = st.dependencies.iter().all(|d| completed.contains(d));
                    if deps_satisfied {
                        group.push(name.clone());
                    } else {
                        still_remaining.push(name.clone());
                    }
                }
            }

            if group.is_empty() {
                // 安全兜底：依赖无法解析时（如循环依赖），强制推进避免死循环
                still_remaining.drain(..).for_each(|n| group.push(n));
            }

            for name in &group {
                completed.insert(name.clone());
            }
            groups.push(group);
            remaining = still_remaining;
        }
        groups
    }

    fn lower_subtask(
        &self,
        plan: &OrchestrationPlan,
        subtask: &SubTask,
        session_id: &str,
    ) -> LoweredTask {
        let primitive = self.determine_lowering_primitive(subtask);
        let parallel_safe = matches!(
            primitive,
            LoweringPrimitive::Query | LoweringPrimitive::Transform
        );
        let lowered_task_type = primitive.as_task_type().to_string();
        let content = format!(
            "[orchestration/{plan_id}/{subtask_name}] original_task_type={original_task_type}\nparent_job_id={parent_job_id}\nrequest={request}\nsubtask_description={description}\n\nYou are executing exactly one orchestration subtask inside a larger plan. Focus only on this subtask and return the concrete result needed by downstream steps. Do not re-decompose the whole project. Do not keep planning indefinitely. Do not create or manage todo lists unless the user explicitly asked for that exact output. Do not use tools. Do not spawn subagents. Assume reasonable defaults for minor missing details when you can still produce a useful concrete result. Only ask one focused question if a truly critical detail is missing and you cannot proceed without it. Otherwise, produce a concise final deliverable for this subtask and stop after the result.",
            plan_id = plan.id,
            subtask_name = subtask.name,
            original_task_type = subtask.task_type,
            parent_job_id = subtask.parent_job_id,
            request = plan.original_request,
            description = subtask.description,
        );
        let payload = match primitive {
            LoweringPrimitive::ApiCall => json!({
                "content": content,
                "opencode-session-id": session_id,
                "parent_job_id": subtask.parent_job_id,
                "subtask_id": subtask.id,
                "plan_id": subtask.parent_id,
                "subtask_name": subtask.name,
                "original_task_type": subtask.task_type,
                "orchestration_subtask": true,
                "streaming_observable": false,
            }),
            LoweringPrimitive::Query => json!({
                "query": ["request"],
                "context": Value::Null,
                "parent_job_id": subtask.parent_job_id,
                "subtask_id": subtask.id,
                "plan_id": subtask.parent_id,
                "subtask_name": subtask.name,
                "original_task_type": subtask.task_type,
            }),
            LoweringPrimitive::Transform => json!({
                "data": {
                    "request": plan.original_request,
                    "plan_id": plan.id,
                    "parent_job_id": plan.parent_job_id,
                    "subtask_name": subtask.name,
                    "original_task_type": subtask.task_type,
                    "description": subtask.description,
                },
                "parent_job_id": subtask.parent_job_id,
                "subtask_id": subtask.id,
                "plan_id": subtask.parent_id,
                "subtask_name": subtask.name,
                "original_task_type": subtask.task_type,
            }),
        };
        let task = make_task(MakeTask {
            trace_id: Some(subtask.trace_id.clone()),
            task_type: lowered_task_type.clone(),
            payload: Some(payload),
            priority: Some(subtask.priority as u8),
            timeout_ms: Some(self.config.default_timeout_ms),
            metadata: Some(json!({
                "parent_job_id": subtask.parent_job_id,
                "subtask_id": subtask.id,
                "plan_id": subtask.parent_id,
                "subtask_name": subtask.name,
                "specialist_type": subtask.specialist_type,
                "original_task_type": subtask.task_type,
                "lowered_task_type": lowered_task_type,
                "parallel_safe": parallel_safe,
                "result_kind": primitive.result_kind(),
                "orchestration_rule": plan.matched_rule,
                "orchestration_subtask": true,
                "streaming_observable": false,
            })),
        });
        LoweredTask {
            name: subtask.name.clone(),
            original_task_type: subtask.task_type.clone(),
            lowered_task_type,
            primitive,
            parallel_safe,
            task,
        }
    }

    fn determine_lowering_primitive(&self, subtask: &SubTask) -> LoweringPrimitive {
        match subtask.task_type.as_str() {
            "generic" => LoweringPrimitive::Query,
            "reporting" => LoweringPrimitive::Transform,
            "design" | "frontend" | "backend" | "testing" | "refactoring" | "analysis"
            | "planning" | "data-collection" => LoweringPrimitive::ApiCall,
            _ => LoweringPrimitive::ApiCall,
        }
    }

    pub fn execute_orchestration_for_main_chain<F>(
        &self,
        request: &str,
        trace_id: &str,
        parent_job_id: &str,
        session_id: &str,
        publish_event: F,
    ) -> Result<OrchestrationExecutionResult, AgentError>
    where
        F: FnMut(&str, Value),
    {
        if request.contains("[orchestration-fail]") {
            return Err(AgentError::OrchestrationForcedFallback);
        }

        let plan = Arc::new(self.decompose_task(request, trace_id, parent_job_id, Some(session_id)));
        let was_decomposed = plan.subtasks.len() > 1
            || plan.matched_rule.is_some();
        if !was_decomposed {
            return Err(AgentError::OrchestrationNoDecomposition);
        }
        self.execute_orchestration_plan_impl(plan, request, parent_job_id, session_id, publish_event)
    }

    /// Execute a pre-built OrchestrationPlan (used by agentic loop fallback).
    pub fn execute_existing_plan<F>(
        &self,
        plan: Arc<OrchestrationPlan>,
        request: &str,
        parent_job_id: &str,
        session_id: &str,
        publish_event: F,
    ) -> Result<TaskResult, AgentError>
    where
        F: FnMut(&str, Value),
    {
        self.execute_orchestration_plan_impl(plan, request, parent_job_id, session_id, publish_event)
            .map(|r| r.result)
    }

    fn execute_orchestration_plan_impl<F>(
        &self,
        plan: Arc<OrchestrationPlan>,
        request: &str,
        parent_job_id: &str,
        session_id: &str,
        mut publish_event: F,
    ) -> Result<OrchestrationExecutionResult, AgentError>
    where
        F: FnMut(&str, Value),
    {
        let started = now_ms();
        let lowered_tasks = plan
            .execution_order
            .iter()
            .filter_map(|name| plan.subtasks.get(name))
            .map(|subtask| self.lower_subtask(&plan, subtask, session_id))
            .collect::<Vec<_>>();
        let mut execution_context =
            OrchestrationExecutionContext::new(request, parent_job_id, &plan.id, session_id);

        self.upsert_runtime_plan_task(&plan, TaskStatus::Pending, None);
        for lowered in &lowered_tasks {
            if let Some(subtask) = plan.subtasks.get(&lowered.name) {
                self.upsert_runtime_subtask_task(
                    subtask,
                    lowered,
                    TaskStatus::Pending,
                    None,
                    None,
                    None,
                    None,
                );
            }
        }

        let finalize_plan = |publish_event: &mut F, result_status: &str| {
            let finalize_started_at_ms = now_ms();
            publish_event(
                "orchestration_plan_finalize_started",
                json!({
                    "parent_job_id": plan.parent_job_id,
                    "plan_id": plan.id,
                    "started_at_ms": finalize_started_at_ms,
                    "result_status": result_status,
                }),
            );
            let active_subtasks = self.runtime_active_subtask_ids(&plan.id);
            let active_plan_present_before_cleanup = self.runtime_plan_is_active(&plan.id);
            let active_plan_present_after_cleanup = self.runtime_plan_is_active(&plan.id);
            if !active_subtasks.is_empty() || active_plan_present_after_cleanup {
                publish_event(
                    "orchestration_plan_cleanup_incomplete",
                    json!({
                        "parent_job_id": plan.parent_job_id,
                        "plan_id": plan.id,
                        "result_status": result_status,
                        "active_subtask_ids": active_subtasks,
                        "active_plan_present_before_cleanup": active_plan_present_before_cleanup,
                        "active_plan_present_after_cleanup": active_plan_present_after_cleanup,
                    }),
                );
            }
            let finished_at_ms = now_ms();
            publish_event(
                "orchestration_plan_finalize_finished",
                json!({
                    "parent_job_id": plan.parent_job_id,
                    "plan_id": plan.id,
                    "started_at_ms": finalize_started_at_ms,
                    "finished_at_ms": finished_at_ms,
                    "finalize_duration_ms": finished_at_ms.saturating_sub(finalize_started_at_ms),
                    "result_status": result_status,
                    "active_plan_present_after_cleanup": active_plan_present_after_cleanup,
                }),
            );
        };

        publish_event(
            "orchestration_plan_created",
            json!({
                "parent_job_id": plan.parent_job_id,
                "plan_id": plan.id,
                "trace_id": plan.trace_id,
                "original_request": plan.original_request,
                "plan_rule": plan.matched_rule,
                "created_at_ms": plan.created_at,
                "execution_order": plan.execution_order,
                "subtask_ids": plan.subtasks.values().map(|subtask| subtask.id.clone()).collect::<Vec<_>>(),
                "subtask_count": lowered_tasks.len(),
                "parallel_groups": plan.parallel_groups,
                "lowered_tasks": lowered_tasks
                    .iter()
                    .map(|task| json!({
                        "name": task.name,
                        "original_task_type": task.original_task_type,
                        "lowered_task_type": task.lowered_task_type,
                        "parallel_safe": task.parallel_safe,
                        "result_kind": task.primitive.result_kind(),
                    }))
                    .collect::<Vec<_>>(),
            }),
        );

        {
            let mut m = self.metrics.lock();
            m.plans_created += 1;
        }

        self.update_plan_runtime_status(&plan, TaskStatus::Running, None);
        let mut last_success: Option<TaskResult> = None;

        for (parallel_group_index, group) in plan.parallel_groups.iter().enumerate() {
            let group_tasks = group
                .iter()
                .filter_map(|name| lowered_tasks.iter().find(|task| task.name == *name))
                .cloned()
                .collect::<Vec<_>>();

            if group_tasks.is_empty() {
                continue;
            }

            let execution_mode = if self.should_execute_group_in_parallel(&group_tasks) {
                "whitelist_parallel"
            } else {
                "serial"
            };

            let group_result = if execution_mode == "whitelist_parallel" {
                self.execute_group_parallel(
                    &plan,
                    &group_tasks,
                    parallel_group_index,
                    execution_mode,
                    &mut execution_context,
                    &mut publish_event,
                )
            } else {
                self.execute_group_serial(
                    &plan,
                    &group_tasks,
                    parallel_group_index,
                    execution_mode,
                    session_id,
                    &mut execution_context,
                    &mut publish_event,
                )
            };

            match group_result? {
                Some(result) => {
                    if result.status == "success" {
                        last_success = Some(result.clone());
                        if detect_pending_question(result.result.as_ref(), session_id).is_some() {
                            self.update_plan_runtime_status(&plan, TaskStatus::Completed, None);
                            break;
                        }
                    } else {
                        self.update_plan_runtime_status(
                            &plan,
                            TaskStatus::Failed,
                            result.error.clone(),
                        );
                        finalize_plan(&mut publish_event, &result.status);
                        {
                            let mut m = self.metrics.lock();
                            m.plans_failed += 1;
                            m.total_duration_ms += now_ms().saturating_sub(started);
                        }
                        return Ok(OrchestrationExecutionResult {
                            result,
                            lowered_tasks,
                            was_decomposed: true,
                        });
                    }
                }
                None => continue,
            }
        }

        let mut final_result = if let Some(result) = last_success.as_ref().filter(|result| {
            detect_pending_question(result.result.as_ref(), session_id).is_some()
        }) {
            result.clone()
        } else {
            self.build_orchestration_final_result(
                &plan,
                &lowered_tasks,
                &execution_context,
                last_success.as_ref(),
            )
        };

        if let Some(result_value) = final_result.result.as_mut() {
            if let Some(result_obj) = result_value.as_object_mut() {
                result_obj.insert("plan_id".into(), Value::String(plan.id.clone()));
                result_obj.insert(
                    "parent_job_id".into(),
                    Value::String(plan.parent_job_id.clone()),
                );
                result_obj.insert(
                    "orchestration".into(),
                    json!({
                        "plan_id": plan.id,
                        "parent_job_id": plan.parent_job_id,
                        "rule": plan.matched_rule,
                        "subtasks": lowered_tasks
                            .iter()
                            .map(|task| json!({
                                "name": task.name,
                                "original_task_type": task.original_task_type,
                                "lowered_task_type": task.lowered_task_type,
                                "parallel_safe": task.parallel_safe,
                                "result_kind": task.primitive.result_kind(),
                            }))
                            .collect::<Vec<_>>(),
                    }),
                );
            }
        }

        self.update_plan_runtime_status(&plan, TaskStatus::Completed, None);
        finalize_plan(&mut publish_event, &final_result.status);
        {
            let mut m = self.metrics.lock();
            m.plans_completed += 1;
            m.total_duration_ms += now_ms().saturating_sub(started);
        }
        Ok(OrchestrationExecutionResult {
            result: final_result,
            lowered_tasks,
            was_decomposed: true,
        })
    }

    fn should_execute_group_in_parallel(&self, group: &[LoweredTask]) -> bool {
        self.config.enable_parallel
            && group.len() > 1
            && group.iter().all(|task| task.parallel_safe)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_group_serial<F>(
        &self,
        plan: &Arc<OrchestrationPlan>,
        group: &[LoweredTask],
        parallel_group_index: usize,
        execution_mode: &str,
        session_id: &str,
        execution_context: &mut OrchestrationExecutionContext,
        publish_event: &mut F,
    ) -> Result<Option<TaskResult>, AgentError>
    where
        F: FnMut(&str, Value),
    {
        let mut last_success = None;
        for lowered in group {
            let result = self.execute_lowered_subtask(
                plan,
                lowered,
                parallel_group_index,
                group.len(),
                execution_mode,
                execution_context,
                publish_event,
            );
            if result.status == "success" {
                last_success = Some(result.clone());
                if result.blocked_reason.is_some() {
                    return Ok(Some(result));
                }
                if matches!(lowered.primitive, LoweringPrimitive::ApiCall)
                    && detect_pending_question(result.result.as_ref(), session_id).is_some()
                {
                    return Ok(Some(result));
                }
            } else {
                return Ok(Some(result));
            }
        }
        Ok(last_success)
    }

    fn execute_group_parallel<F>(
        &self,
        plan: &Arc<OrchestrationPlan>,
        group: &[LoweredTask],
        parallel_group_index: usize,
        execution_mode: &str,
        execution_context: &mut OrchestrationExecutionContext,
        publish_event: &mut F,
    ) -> Result<Option<TaskResult>, AgentError>
    where
        F: FnMut(&str, Value),
    {
        let mut receivers = Vec::new();
        for lowered in group {
            let Some(subtask) = plan.subtasks.get(&lowered.name) else {
                continue;
            };
            let prepared = self.prepare_lowered_task(lowered, execution_context);
            self.mark_subtask_started(
                subtask,
                lowered,
                parallel_group_index,
                group.len(),
                execution_mode,
                publish_event,
            );
            let rx = self.worker_pool.submit_task(prepared);
            receivers.push((lowered.clone(), rx));
        }

        let mut last_success = None;
        for (lowered, rx) in receivers {
            let Some(subtask) = plan.subtasks.get(&lowered.name) else {
                continue;
            };
            let wait_started_at_ms = now_ms();
            publish_event(
                "orchestration_wait_started",
                json!({
                    "plan_id": subtask.parent_id,
                    "parent_job_id": subtask.parent_job_id,
                    "subtask_id": subtask.id,
                    "subtask_name": subtask.name,
                    "task_id": lowered.task.id,
                    "task_type": lowered.task.task_type,
                    "execution_mode": execution_mode,
                    "parallel_group_index": parallel_group_index,
                    "parallel_group_size": group.len(),
                    "started_at_ms": wait_started_at_ms,
                    "wait_reason": "parallel_group",
                }),
            );
            let result = rx.recv().unwrap_or_else(|_| {
                make_task_result(MakeTaskResult {
                    task_id: lowered.task.id.clone(),
                    trace_id: lowered.task.trace_id.clone(),
                    status: "failure".into(),
                    error: Some("Task result channel closed".into()),
                    duration_ms: 0,
                    worker_id: None,
                    result: None,
                })
            });
            let wait_finished_at_ms = now_ms();
            publish_event(
                "orchestration_wait_finished",
                json!({
                    "plan_id": subtask.parent_id,
                    "parent_job_id": subtask.parent_job_id,
                    "subtask_id": subtask.id,
                    "subtask_name": subtask.name,
                    "task_id": lowered.task.id,
                    "task_type": lowered.task.task_type,
                    "execution_mode": execution_mode,
                    "parallel_group_index": parallel_group_index,
                    "parallel_group_size": group.len(),
                    "started_at_ms": wait_started_at_ms,
                    "finished_at_ms": wait_finished_at_ms,
                    "wait_duration_ms": wait_finished_at_ms.saturating_sub(wait_started_at_ms),
                    "wait_reason": "parallel_group",
                    "result_status": result.status,
                    "worker_id": result.worker_id,
                    "error": result.error,
                }),
            );
            self.finalize_subtask_result(
                plan,
                subtask,
                &lowered,
                &result,
                parallel_group_index,
                group.len(),
                execution_mode,
                publish_event,
            );
            if result.status == "success" {
                self.apply_subtask_result_to_context(execution_context, &lowered, &result);
                last_success = Some(result.clone());
            } else {
                return Ok(Some(result));
            }
        }
        Ok(last_success)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_lowered_subtask<F>(
        &self,
        plan: &Arc<OrchestrationPlan>,
        lowered: &LoweredTask,
        parallel_group_index: usize,
        parallel_group_size: usize,
        execution_mode: &str,
        execution_context: &mut OrchestrationExecutionContext,
        publish_event: &mut F,
    ) -> TaskResult
    where
        F: FnMut(&str, Value),
    {
        let Some(subtask) = plan.subtasks.get(&lowered.name) else {
            return Self::failure(format!("Subtask not found: {}", lowered.name));
        };
        let task = self.prepare_lowered_task(lowered, execution_context);
        self.mark_subtask_started(
            subtask,
            lowered,
            parallel_group_index,
            parallel_group_size,
            execution_mode,
            publish_event,
        );
        let wait_context = WaitDiagnosticContext {
            plan_id: subtask.parent_id.clone(),
            parent_job_id: subtask.parent_job_id.clone(),
            subtask_id: subtask.id.clone(),
            subtask_name: subtask.name.clone(),
            task_id: task.id.clone(),
            task_type: task.task_type.clone(),
            execution_mode: execution_mode.to_string(),
            parallel_group_index,
            parallel_group_size,
            wait_reason: "single_task".into(),
        };
        let result =
            Self::wait_for_task(&self.worker_pool, task, Some(wait_context), publish_event);
        self.finalize_subtask_result(
            plan,
            subtask,
            lowered,
            &result,
            parallel_group_index,
            parallel_group_size,
            execution_mode,
            publish_event,
        );
        if result.status == "success" {
            self.apply_subtask_result_to_context(execution_context, lowered, &result);
        }
        result
    }

    fn prepare_lowered_task(
        &self,
        lowered: &LoweredTask,
        execution_context: &OrchestrationExecutionContext,
    ) -> Task {
        let mut task = lowered.task.clone();
        if let Some(payload) = task.payload.as_object_mut() {
            match lowered.primitive {
                LoweringPrimitive::Query => {
                    payload.insert("context".into(), execution_context.as_value());
                }
                LoweringPrimitive::Transform => {
                    payload.insert(
                        "data".into(),
                        json!({
                            "request": execution_context.request.clone(),
                            "plan_id": execution_context.plan_id.clone(),
                            "parent_job_id": execution_context.parent_job_id.clone(),
                            "session_id": execution_context.session_id.clone(),
                            "subtask_name": lowered.name,
                            "source": execution_context
                                .subtask_results
                                .get(&lowered.name)
                                .cloned()
                                .unwrap_or(Value::Null),
                        }),
                    );
                }
                LoweringPrimitive::ApiCall => {}
            }
        }
        task
    }

    fn mark_subtask_started<F>(
        &self,
        subtask: &Arc<SubTask>,
        lowered: &LoweredTask,
        parallel_group_index: usize,
        parallel_group_size: usize,
        execution_mode: &str,
        publish_event: &mut F,
    ) where
        F: FnMut(&str, Value),
    {
        *subtask.started_at.lock() = Some(now_ms());
        self.update_subtask_runtime_status(
            subtask,
            lowered,
            TaskStatus::Running,
            Some(execution_mode),
            Some(parallel_group_index),
            Some(parallel_group_size),
            None,
        );
        publish_event(
            "orchestration_subtask_started",
            json!({
                "parent_job_id": subtask.parent_job_id,
                "subtask_id": subtask.id,
                "plan_id": subtask.parent_id,
                "subtask_name": subtask.name,
                "original_task_type": lowered.original_task_type,
                "lowered_task_type": lowered.lowered_task_type,
                "parallel_safe": lowered.parallel_safe,
                "parallel_group_index": parallel_group_index,
                "parallel_group_size": parallel_group_size,
                "execution_mode": execution_mode,
                "result_kind": lowered.primitive.result_kind(),
                "task_id": lowered.task.id,
            }),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_subtask_result<F>(
        &self,
        _plan: &Arc<OrchestrationPlan>,
        subtask: &Arc<SubTask>,
        lowered: &LoweredTask,
        result: &TaskResult,
        parallel_group_index: usize,
        parallel_group_size: usize,
        execution_mode: &str,
        publish_event: &mut F,
    ) where
        F: FnMut(&str, Value),
    {
        *subtask
            .completed_at
            .lock() = Some(now_ms());
        *subtask.result.lock() = Some(result.clone());
        {
            let mut m = self.metrics.lock();
            m.subtasks_executed += 1;
            if result.status != "success" {
                m.subtasks_failed += 1;
            }
        }
        let mut event_payload = json!({
            "parent_job_id": subtask.parent_job_id,
            "subtask_id": subtask.id,
            "plan_id": subtask.parent_id,
            "subtask_name": subtask.name,
            "original_task_type": lowered.original_task_type,
            "lowered_task_type": lowered.lowered_task_type,
            "parallel_safe": lowered.parallel_safe,
            "parallel_group_index": parallel_group_index,
            "parallel_group_size": parallel_group_size,
            "execution_mode": execution_mode,
            "result_kind": lowered.primitive.result_kind(),
            "task_id": lowered.task.id,
            "status": result.status,
            "worker_id": result.worker_id,
            "task_duration_ms": result.duration_ms,
            "result_preview": self.result_preview(result),
        });

        if result.status == "success" {
            self.update_subtask_runtime_status(
                subtask,
                lowered,
                TaskStatus::Completed,
                Some(execution_mode),
                Some(parallel_group_index),
                Some(parallel_group_size),
                None,
            );
            publish_event("orchestration_subtask_completed", event_payload);
        } else {
            let error = result.error.clone();
            self.update_subtask_runtime_status(
                subtask,
                lowered,
                TaskStatus::Failed,
                Some(execution_mode),
                Some(parallel_group_index),
                Some(parallel_group_size),
                error.clone(),
            );
            if let Some(map) = event_payload.as_object_mut() {
                map.insert(
                    "error".into(),
                    error.map(Value::String).unwrap_or(Value::Null),
                );
            }
            publish_event("orchestration_subtask_failed", event_payload);
        }
    }

    fn apply_subtask_result_to_context(
        &self,
        execution_context: &mut OrchestrationExecutionContext,
        lowered: &LoweredTask,
        result: &TaskResult,
    ) {
        if !execution_context.subtask_results.is_object() {
            execution_context.subtask_results = json!({});
        }
        if let Some(results) = execution_context.subtask_results.as_object_mut() {
            results.insert(
                lowered.name.clone(),
                json!({
                    "status": result.status,
                    "result": result.result,
                    "error": result.error,
                    "worker_id": result.worker_id,
                    "duration_ms": result.duration_ms,
                    "lowered_task_type": lowered.lowered_task_type,
                    "original_task_type": lowered.original_task_type,
                    "result_kind": lowered.primitive.result_kind(),
                    "blocked_reason": result.blocked_reason,
                }),
            );
        }
    }

    fn result_preview(&self, result: &TaskResult) -> Option<String> {
        result
            .result
            .as_ref()
            .and_then(|value| {
                value
                    .get("content")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .or_else(|| {
                result.result.as_ref().map(|value| {
                    let rendered = value.to_string();
                    rendered.chars().take(160).collect::<String>()
                })
            })
    }

    pub(crate) fn extract_result_text(value: Option<&Value>) -> String {
        let Some(value) = value else {
            return String::new();
        };

        if let Some(content) = value.get("content").and_then(Value::as_str) {
            return content.to_string();
        }

        if let Some(content) = value.get("content").and_then(Value::as_array) {
            let chunks = content
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>();
            if !chunks.is_empty() {
                return chunks.join("\n");
            }
        }

        value
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| value.to_string())
    }

    fn build_orchestration_summary_text(
        &self,
        plan: &OrchestrationPlan,
        lowered_tasks: &[LoweredTask],
        execution_context: &OrchestrationExecutionContext,
    ) -> String {
        let mut lines = vec![
            format!(
                "Orchestration completed for request: {}",
                plan.original_request
            ),
            String::new(),
            "Subtask outcomes:".to_string(),
        ];

        for lowered in lowered_tasks {
            let entry = execution_context
                .subtask_results
                .get(&lowered.name)
                .cloned()
                .unwrap_or(Value::Null);
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let result_text = Self::extract_result_text(entry.get("result"));
            let preview = if result_text.is_empty() {
                entry
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("(no result)")
                    .to_string()
            } else if result_text.chars().count() > 280 {
                format!("{}…", result_text.chars().take(280).collect::<String>())
            } else {
                result_text
            };

            lines.push(format!(
                "- {} [{} / {}]: {}",
                lowered.name, lowered.original_task_type, status, preview
            ));
        }

        lines.join("\n")
    }

    fn collect_blocked_reasons(
        &self,
        lowered_tasks: &[LoweredTask],
        execution_context: &OrchestrationExecutionContext,
    ) -> Option<Value> {
        let blocked: Vec<_> = lowered_tasks
            .iter()
            .filter_map(|task| {
                let entry = execution_context.subtask_results.get(&task.name)?;
                let reason = entry.get("blocked_reason").filter(|v| !v.is_null())?;
                Some(json!({ "subtask": task.name, "reason": reason }))
            })
            .collect();
        if blocked.is_empty() {
            return None;
        }
        Some(json!({
            "type": "subtask_blocked",
            "blocked_subtasks": blocked,
        }))
    }

    fn infer_missing_context_question(
        &self,
        plan: &OrchestrationPlan,
        lowered_tasks: &[LoweredTask],
        execution_context: &OrchestrationExecutionContext,
    ) -> Option<Value> {
        if let Some(provider) = &*self.provider.read() {
            if let Some(question) = super::llm_context_infer::try_llm_infer_missing_context(
                provider,
                &execution_context.session_id,
                plan,
                lowered_tasks,
                &execution_context.subtask_results,
                &self.prompts.read().context_infer,
            ) {
                return Some(question);
            }
        }

        // fallback: 硬编码关键词匹配
        let mut missing_context_count = 0usize;
        let mut prompts = Vec::new();

        for lowered in lowered_tasks {
            let entry = execution_context.subtask_results.get(&lowered.name)?;
            let result = entry.get("result")?;
            let content = Self::extract_result_text(Some(result));
            let normalized = content.to_ascii_lowercase();
            if normalized.contains("只问一个")
                || normalized.contains("聚焦问题")
                || normalized.contains("关键问题")
                || normalized.contains("what business")
                || normalized.contains("business type")
                || normalized.contains("核心用户流程")
                || normalized.contains("核心页面")
                || normalized.contains("核心实体")
            {
                missing_context_count += 1;
                prompts.push(content);
            }
        }

        if missing_context_count < 2 {
            return None;
        }

        Some(json!({
            "type": "question",
            "question": {
                "id": format!("orchestration-context-{}", plan.id),
                "kind": "input",
                "prompt": "为了继续完成这个复杂 Web App 的整体拆分与最终交付，我需要一个统一的业务上下文：请告诉我产品类型、核心用户，以及 3-5 个核心功能。若有特殊需求，也请补充是否需要登录、支付、文件上传、实时功能或多租户。",
                "options": [
                    "电商平台",
                    "项目管理系统",
                    "AI SaaS",
                    "内容社区"
                ],
                "orchestration": {
                    "reason": "multiple_subtasks_missing_shared_context",
                    "subtasks": lowered_tasks.iter().map(|task| task.name.clone()).collect::<Vec<_>>(),
                    "collected_prompts": prompts,
                }
            }
        }))
    }

    fn build_orchestration_final_result(
        &self,
        plan: &OrchestrationPlan,
        lowered_tasks: &[LoweredTask],
        execution_context: &OrchestrationExecutionContext,
        last_success: Option<&TaskResult>,
    ) -> TaskResult {
        let aggregated_result =
            if let Some(blocked) = self.collect_blocked_reasons(lowered_tasks, execution_context) {
                blocked
            } else if let Some(question) =
                self.infer_missing_context_question(plan, lowered_tasks, execution_context)
            {
                question
            } else {
                let summary_text =
                    self.build_orchestration_summary_text(plan, lowered_tasks, execution_context);
                json!({
                    "content": summary_text,
                    "plan_id": plan.id,
                    "parent_job_id": plan.parent_job_id,
                    "subtask_results": execution_context.subtask_results.clone(),
                })
            };

        let (total_subtasks, completed_subtasks, failed_subtasks) =
            self.runtime_plan_progress_counts(&plan.id);
        let mut result = make_task_result(MakeTaskResult {
            task_id: plan.id.clone(),
            trace_id: plan.trace_id.clone(),
            status: "success".into(),
            result: Some(aggregated_result),
            error: None,
            duration_ms: now_ms().saturating_sub(plan.created_at),
            worker_id: last_success.and_then(|result| result.worker_id.clone()),
        });

        if let Some(result_value) = result.result.as_mut() {
            if let Some(result_obj) = result_value.as_object_mut() {
                result_obj.insert("subtasks_total".into(), Value::from(total_subtasks));
                result_obj.insert("subtasks_completed".into(), Value::from(completed_subtasks));
                result_obj.insert("subtasks_failed".into(), Value::from(failed_subtasks));
            }
        }

        if let Some(result_value) = result.result.as_ref() {
            if result_value.get("type").and_then(Value::as_str) == Some("subtask_blocked") {
                if let Some(first_reason) = result_value
                    .get("blocked_subtasks")
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.first())
                    .and_then(|entry| entry.get("reason"))
                {
                    result.blocked_reason = Some(first_reason.clone());
                }
            }
        }

        if let Some(last_success) = last_success {
            result.duration_ms = result.duration_ms.max(last_success.duration_ms);
        }

        result
    }

    // ── Orchestration ─────────────────────────────────────────────

    /// 编排入口：分解请求 → 存储计划 → 执行 → 清理 → 更新指标
    pub fn orchestrate(&self, request: &str) -> TaskResult {
        let trace_id = Uuid::new_v4().to_string();
        let started = now_ms();

        // Decompose
        let plan = Arc::new(self.decompose_task(request, &trace_id, &trace_id, None));

        {
            let mut m = self.metrics.lock();
            m.plans_created += 1;
        }

        // Execute
        let result = self.execute_orchestration_plan(&plan);

        // Update metrics
        let elapsed = now_ms().saturating_sub(started);
        {
            let mut m = self.metrics.lock();
            m.total_duration_ms += elapsed;
            if result.status == "success" {
                m.plans_completed += 1;
            } else {
                m.plans_failed += 1;
            }
        }

        result
    }

    /// 执行编排计划：按并行分组逐组执行子任务
    ///
    /// 组内只有一个任务或未启用并行时串行执行，否则并发提交后收集结果。
    /// 任一子任务失败则整个计划标记为 Failed 并提前返回。
    pub fn execute_orchestration_plan(&self, plan: &OrchestrationPlan) -> TaskResult {
        for group in &plan.parallel_groups {
            if group.len() == 1 || !self.config.enable_parallel {
                // 单任务组或未启用并行：逐个串行执行
                for name in group {
                    let result = self.execute_subtask(plan, name);
                    if result.status != "success" {
                        return result;
                    }
                }
            } else {
                // 多任务组且启用并行：先全部提交，再统一收集结果
                let mut receivers = Vec::new();
                for name in group {
                    if let Some(subtask) = plan.subtasks.get(name) {
                        *subtask.started_at.lock() =
                            Some(now_ms());

                        let task = make_task(MakeTask {
                            trace_id: Some(subtask.trace_id.clone()),
                            task_type: subtask.task_type.clone(),
                            payload: Some(subtask.payload.clone()),
                            priority: Some(subtask.priority as u8),
                            timeout_ms: Some(self.config.default_timeout_ms),
                            metadata: Some(json!({
                                "parent_job_id": subtask.parent_job_id,
                                "subtask_id": subtask.id,
                                "plan_id": subtask.parent_id,
                                "subtask_name": subtask.name,
                                "specialist_type": subtask.specialist_type,
                            })),
                        });
                        let rx = self.worker_pool.submit_task(task);
                        receivers.push((name.clone(), rx));
                    }
                }
                // 收集并行结果，记录第一个失败结果
                let mut failed = false;
                let mut fail_result: Option<TaskResult> = None;
                for (name, rx) in receivers {
                    let result = rx.recv().unwrap_or_else(|_| {
                        make_task_result(MakeTaskResult {
                            task_id: Uuid::new_v4().to_string(),
                            trace_id: plan.trace_id.clone(),
                            status: "failure".into(),
                            error: Some("Subtask channel closed".into()),
                            duration_ms: 0,
                            worker_id: None,
                            result: None,
                        })
                    });

                    if let Some(subtask) = plan.subtasks.get(&name) {
                        *subtask
                            .completed_at
                            .lock() = Some(now_ms());

                        let mut m = self.metrics.lock();
                        m.subtasks_executed += 1;

                        if result.status == "success" {
                        } else {
                            m.subtasks_failed += 1;
                            failed = true;
                            if fail_result.is_none() {
                                fail_result = Some(result.clone());
                            }
                        }
                        *subtask.result.lock() =
                            Some(result.clone());
                    }
                }

                if failed {
                    if let Some(r) = fail_result {
                        return r;
                    }
                }
            }
        }

        make_task_result(MakeTaskResult {
            task_id: plan.id.clone(),
            trace_id: plan.trace_id.clone(),
            status: "success".into(),
            result: Some({
                let (total_subtasks, completed_subtasks, failed_subtasks) =
                    self.runtime_plan_progress_counts(&plan.id);
                json!({
                    "plan_id": plan.id,
                    "request": plan.original_request,
                    "subtasks_total": total_subtasks,
                    "subtasks_completed": completed_subtasks,
                    "subtasks_failed": failed_subtasks,
                })
            }),
            error: None,
            duration_ms: now_ms().saturating_sub(plan.created_at),
            worker_id: None,
        })
    }

    /// 执行单个子任务：有专家类型时路由到专家池，否则直接提交到 WorkerPool
    fn execute_subtask(&self, plan: &OrchestrationPlan, name: &str) -> TaskResult {
        let subtask = match plan.subtasks.get(name) {
            Some(st) => st,
            None => {
                return Self::failure(format!("Subtask not found: {}", name));
            }
        };

        *subtask.started_at.lock() = Some(now_ms());

        let task = make_task(MakeTask {
            trace_id: Some(subtask.trace_id.clone()),
            task_type: subtask.task_type.clone(),
            payload: Some(subtask.payload.clone()),
            priority: Some(subtask.priority as u8),
            timeout_ms: Some(self.config.default_timeout_ms),
            metadata: Some(json!({
                "parent_job_id": subtask.parent_job_id,
                "subtask_id": subtask.id,
                "plan_id": subtask.parent_id,
                "subtask_name": subtask.name,
                "specialist_type": subtask.specialist_type,
            })),
        });

        let result = if !subtask.specialist_type.is_empty() {
            self.specialist_pool.route_task(task)
        } else {
            let rx = self.worker_pool.submit_task(task);
            rx.recv().unwrap_or_else(|_| {
                make_task_result(MakeTaskResult {
                    task_id: subtask.id.clone(),
                    trace_id: subtask.trace_id.clone(),
                    status: "failure".into(),
                    error: Some("Subtask channel closed".into()),
                    duration_ms: 0,
                    worker_id: None,
                    result: None,
                })
            })
        };

        *subtask
            .completed_at
            .lock() = Some(now_ms());

        {
            let mut m = self.metrics.lock();
            m.subtasks_executed += 1;
            if result.status != "success" {
                m.subtasks_failed += 1;
            }
        }

        *subtask.result.lock() = Some(result.clone());
        result
    }

    // ── Backward-compatible associated functions ──────────────────

    /// 实例级兼容入口：当前仍复用既有 ExecutionPlan 执行语义，供主链路渐进接入 orchestrator 实例。
    pub fn execute_plan_instance(&self, plan: &ExecutionPlan, session_id: &str) -> TaskResult {
        match plan.kind {
            ExecutionPlanKind::SpecialistRoute => {
                let specialist = plan.specialist.clone().unwrap_or_else(|| "default".into());
                let mut task = match plan.tasks.front().cloned() {
                    Some(t) => t,
                    None => return Self::failure("Empty task list in plan"),
                };
                if !task.payload.is_object() {
                    task.payload = json!({});
                }
                if let Some(obj) = task.payload.as_object_mut() {
                    obj.insert(
                        "opencode-session-id".into(),
                        Value::String(session_id.to_string()),
                    );
                }
                self.specialist_pool.execute(&specialist, task)
            }
            _ => Self::execute_plan(&self.worker_pool, plan, session_id),
        }
    }

    /// 根据 ExecutionPlan 的类型分发执行（Single/Sequential/Parallel/SpecialistRoute/Direct）
    pub fn execute_plan(
        worker_pool: &Arc<WorkerPool>,
        plan: &ExecutionPlan,
        session_id: &str,
    ) -> TaskResult {
        match plan.kind {
            ExecutionPlanKind::Single => {
                let task = match plan.tasks.front().cloned() {
                    Some(t) => t,
                    None => return Self::failure("Empty task list in plan"),
                };
                Self::execute_single_task(worker_pool, task, session_id)
            }
            ExecutionPlanKind::Sequential => {
                // 串行执行：每个任务的结果作为下一个任务的 previous-result 传递
                let mut last_result: Option<Value> = None;
                let mut last_task_result: Option<TaskResult> = None;
                for mut task in plan.tasks.iter().cloned() {
                    if !task.payload.is_object() {
                        task.payload = json!({});
                    }
                    if let Some(obj) = task.payload.as_object_mut() {
                        obj.insert(
                            "opencode-session-id".into(),
                            Value::String(session_id.to_string()),
                        );
                        if let Some(previous) = &last_result {
                            obj.insert("previous-result".into(), previous.clone());
                        }
                    }
                    let result = Self::wait_for_task(worker_pool, task, None, &mut |_, _| {});
                    if result.status != "success" {
                        return result;
                    }
                    last_result = result.result.clone();
                    last_task_result = Some(result);
                }
                last_task_result.unwrap_or_else(|| Self::failure("No tasks executed"))
            }
            ExecutionPlanKind::Parallel => {
                // 并行执行：将所有任务一次性提交到 ParallelPool
                let pool = ParallelPool::new(worker_pool.clone());
                pool.execute(plan.tasks.iter().cloned().collect())
            }
            ExecutionPlanKind::SpecialistRoute => {
                // 专家路由：取第一个任务，注入 session-id 后路由到指定专家
                let specialist = plan.specialist.clone().unwrap_or_else(|| "default".into());
                let mut task = match plan.tasks.front().cloned() {
                    Some(t) => t,
                    None => return Self::failure("Empty task list in plan"),
                };
                if !task.payload.is_object() {
                    task.payload = json!({});
                }
                if let Some(obj) = task.payload.as_object_mut() {
                    obj.insert(
                        "opencode-session-id".into(),
                        Value::String(session_id.to_string()),
                    );
                }
                let pool = SpecialistPool::new(worker_pool.clone());
                pool.execute(&specialist, task)
            }
            ExecutionPlanKind::Direct => make_task_result(MakeTaskResult {
                task_id: Uuid::new_v4().to_string(),
                trace_id: Uuid::new_v4().to_string(),
                status: "success".into(),
                result: Some(json!({"message": "direct"})),
                error: None,
                duration_ms: 0,
                worker_id: None,
            }),
        }
    }

    /// 执行单个任务：注入 session-id 后提交到 WorkerPool
    pub fn execute_single_task(
        worker_pool: &Arc<WorkerPool>,
        mut task: Task,
        session_id: &str,
    ) -> TaskResult {
        if !task.payload.is_object() {
            task.payload = json!({});
        }
        if let Some(obj) = task.payload.as_object_mut() {
            obj.insert(
                "opencode-session-id".into(),
                Value::String(session_id.to_string()),
            );
        }
        Self::wait_for_task(worker_pool, task, None, &mut |_, _| {})
    }

    /// 同步等待任务结果，通道关闭时返回失败
    fn wait_for_task<F>(
        worker_pool: &Arc<WorkerPool>,
        task: Task,
        wait_context: Option<WaitDiagnosticContext>,
        publish_event: &mut F,
    ) -> TaskResult
    where
        F: FnMut(&str, Value),
    {
        let rx = worker_pool.submit_task(task.clone());
        let wait_started_at_ms = now_ms();
        if let Some(context) = wait_context.as_ref() {
            publish_event(
                "orchestration_wait_started",
                json!({
                    "plan_id": context.plan_id,
                    "parent_job_id": context.parent_job_id,
                    "subtask_id": context.subtask_id,
                    "subtask_name": context.subtask_name,
                    "task_id": context.task_id,
                    "task_type": context.task_type,
                    "execution_mode": context.execution_mode,
                    "parallel_group_index": context.parallel_group_index,
                    "parallel_group_size": context.parallel_group_size,
                    "started_at_ms": wait_started_at_ms,
                    "wait_reason": context.wait_reason,
                }),
            );
        }
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
        if let Some(context) = wait_context.as_ref() {
            let wait_finished_at_ms = now_ms();
            publish_event(
                "orchestration_wait_finished",
                json!({
                    "plan_id": context.plan_id,
                    "parent_job_id": context.parent_job_id,
                    "subtask_id": context.subtask_id,
                    "subtask_name": context.subtask_name,
                    "task_id": context.task_id,
                    "task_type": context.task_type,
                    "execution_mode": context.execution_mode,
                    "parallel_group_index": context.parallel_group_index,
                    "parallel_group_size": context.parallel_group_size,
                    "started_at_ms": wait_started_at_ms,
                    "finished_at_ms": wait_finished_at_ms,
                    "wait_duration_ms": wait_finished_at_ms.saturating_sub(wait_started_at_ms),
                    "wait_reason": context.wait_reason,
                    "result_status": result.status,
                    "worker_id": result.worker_id,
                    "error": result.error,
                }),
            );
        }
        result
    }

    /// 构造一个通用失败结果
    fn failure(error: impl Into<String>) -> TaskResult {
        make_task_result(MakeTaskResult {
            task_id: Uuid::new_v4().to_string(),
            trace_id: Uuid::new_v4().to_string(),
            status: "failure".into(),
            error: Some(error.into()),
            duration_ms: 0,
            worker_id: None,
            result: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_subtask(name: &str, deps: &[&str]) -> Arc<SubTask> {
        Arc::new(SubTask {
            id: Uuid::new_v4().to_string(),
            parent_id: "plan-1".into(),
            parent_job_id: "job-1".into(),
            trace_id: "trace-1".into(),
            name: name.into(),
            task_type: "generic".into(),
            description: format!("subtask {name}"),
            dependencies: deps.iter().map(|d| d.to_string()).collect(),
            priority: 5,
            specialist_type: "default".into(),
            payload: json!({}),
            result: Mutex::new(None),
            started_at: Mutex::new(None),
            completed_at: Mutex::new(None),
        })
    }

    fn build_subtasks(specs: &[(&str, &[&str])]) -> IndexMap<String, Arc<SubTask>> {
        let mut map = IndexMap::new();
        for (name, deps) in specs {
            map.insert(name.to_string(), make_subtask(name, deps));
        }
        map
    }

    // ── topological_sort ──

    #[test]
    fn topo_sort_no_deps_preserves_insertion_order() {
        let subtasks = build_subtasks(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn topo_sort_linear_chain() {
        let subtasks = build_subtasks(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        let pos_a = order.iter().position(|x| x == "a").unwrap();
        let pos_b = order.iter().position(|x| x == "b").unwrap();
        let pos_c = order.iter().position(|x| x == "c").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn topo_sort_diamond_dependency() {
        // a → b, a → c, b → d, c → d
        let subtasks = build_subtasks(&[
            ("a", &[]),
            ("b", &["a"]),
            ("c", &["a"]),
            ("d", &["b", "c"]),
        ]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("d"));
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn topo_sort_single_task() {
        let subtasks = build_subtasks(&[("only", &[])]);
        assert_eq!(
            AgentOrchestrator::topological_sort(&subtasks),
            vec!["only"]
        );
    }

    #[test]
    fn topo_sort_cycle_does_not_hang() {
        let subtasks = build_subtasks(&[("a", &["b"]), ("b", &["a"])]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        assert_eq!(order.len(), 2);
    }

    // ── compute_parallel_groups ──

    #[test]
    fn parallel_groups_all_independent() {
        let subtasks = build_subtasks(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        let groups = AgentOrchestrator::compute_parallel_groups(&subtasks, &order);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn parallel_groups_linear_chain() {
        let subtasks = build_subtasks(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        let groups = AgentOrchestrator::compute_parallel_groups(&subtasks, &order);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], vec!["a"]);
        assert_eq!(groups[1], vec!["b"]);
        assert_eq!(groups[2], vec!["c"]);
    }

    #[test]
    fn parallel_groups_diamond() {
        let subtasks = build_subtasks(&[
            ("a", &[]),
            ("b", &["a"]),
            ("c", &["a"]),
            ("d", &["b", "c"]),
        ]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        let groups = AgentOrchestrator::compute_parallel_groups(&subtasks, &order);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], vec!["a"]);
        assert!(groups[1].contains(&"b".to_string()));
        assert!(groups[1].contains(&"c".to_string()));
        assert_eq!(groups[2], vec!["d"]);
    }

    #[test]
    fn parallel_groups_mixed() {
        // a, b independent; c depends on a; d depends on b
        let subtasks = build_subtasks(&[
            ("a", &[]),
            ("b", &[]),
            ("c", &["a"]),
            ("d", &["b"]),
        ]);
        let order = AgentOrchestrator::topological_sort(&subtasks);
        let groups = AgentOrchestrator::compute_parallel_groups(&subtasks, &order);
        assert_eq!(groups.len(), 2);
        assert!(groups[0].contains(&"a".to_string()));
        assert!(groups[0].contains(&"b".to_string()));
        assert!(groups[1].contains(&"c".to_string()));
        assert!(groups[1].contains(&"d".to_string()));
    }

    // ── extract_result_text ──

    #[test]
    fn extract_result_text_string_content() {
        let v = json!({"content": "hello world"});
        assert_eq!(AgentOrchestrator::extract_result_text(Some(&v)), "hello world");
    }

    #[test]
    fn extract_result_text_array_content() {
        let v = json!({"content": [
            {"type": "text", "text": "part1"},
            {"type": "image"},
            {"type": "text", "text": "part2"}
        ]});
        assert_eq!(AgentOrchestrator::extract_result_text(Some(&v)), "part1\npart2");
    }

    #[test]
    fn extract_result_text_plain_string() {
        let v = json!("just a string");
        assert_eq!(AgentOrchestrator::extract_result_text(Some(&v)), "just a string");
    }

    #[test]
    fn extract_result_text_none() {
        assert_eq!(AgentOrchestrator::extract_result_text(None), "");
    }

    #[test]
    fn extract_result_text_object_fallback() {
        let v = json!({"key": "val"});
        let text = AgentOrchestrator::extract_result_text(Some(&v));
        assert!(text.contains("key"));
    }

    // ── failure helper ──

    #[test]
    fn failure_produces_failure_result() {
        let r = AgentOrchestrator::failure("something broke");
        assert_eq!(r.status, "failure");
        assert_eq!(r.error.as_deref(), Some("something broke"));
    }

    // ── PlanProgress ──

    #[test]
    fn plan_progress_default() {
        let p = PlanProgress::default();
        assert_eq!(p.completed_count, 0);
        assert_eq!(p.total_count, 0);
        assert_eq!(p.failed_count, 0);
    }
}
