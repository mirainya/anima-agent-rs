//! 任务编排器模块
//!
//! 负责将复杂请求分解为子任务并协调执行。核心概念：
//! - 通过正则匹配的分解规则（DecompositionRule）将请求拆分为子任务
//! - 子任务之间可声明依赖关系，编排器自动进行拓扑排序和并行分组
//! - 同一组内无依赖的子任务可并行执行，不同组按依赖顺序串行
//! - 同时提供静态方法 `execute_plan` / `execute_single_task` 用于直接执行 ExecutionPlan

use crate::agent_parallel_pool::ParallelPool;
use crate::agent_specialist_pool::SpecialistPool;
use crate::agent_types::{
    make_task, make_task_result, ExecutionPlan, ExecutionPlanKind, MakeTask, MakeTaskResult, Task,
    TaskResult,
};
use crate::agent_worker::WorkerPool;
use crate::support::now_ms;
use indexmap::IndexMap;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ── SubTask ───────────────────────────────────────────────────────────

/// 子任务状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// 子任务：编排计划中的最小执行单元
///
/// 每个子任务声明自己的依赖（dependencies）、所需专家类型（specialist_type），
/// 运行时状态和结果通过 Mutex 保护以支持并发更新。
pub struct SubTask {
    pub id: String,
    pub parent_id: String,
    pub trace_id: String,
    pub name: String,
    pub task_type: String,
    pub description: String,
    pub dependencies: HashSet<String>,
    pub priority: u32,
    pub specialist_type: String,
    pub payload: Value,
    pub status: Mutex<SubTaskStatus>,
    pub result: Mutex<Option<TaskResult>>,
    pub started_at: Mutex<Option<u64>>,
    pub completed_at: Mutex<Option<u64>>,
}

// ── OrchestrationPlan ─────────────────────────────────────────────────

/// 编排计划状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStatus {
    Created,
    Running,
    Completed,
    Failed,
}

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
    pub original_request: String,
    pub subtasks: IndexMap<String, Arc<SubTask>>,
    pub execution_order: Vec<String>,
    pub parallel_groups: Vec<Vec<String>>,
    pub status: Mutex<PlanStatus>,
    pub progress: Mutex<PlanProgress>,
    pub created_at: u64,
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

// ── Decomposition Rules ───────────────────────────────────────────────

/// 分解规则：通过正则匹配请求文本，决定如何拆分子任务
struct DecompositionRule {
    pattern: Regex,
    name: String,
    subtask_templates: Vec<SubTaskTemplate>,
}

/// 子任务模板：定义子任务的名称、类型、专家类型和依赖关系
struct SubTaskTemplate {
    name: String,
    task_type: String,
    specialist_type: String,
    dependencies: Vec<String>,
}

// 预定义的任务分解规则表（web-app、api、refactor 等场景）
lazy_static! {
    static ref TASK_DECOMPOSITION_RULES: Vec<DecompositionRule> = vec![
        DecompositionRule {
            pattern: Regex::new(r"(?i)web.?app|website|frontend").unwrap(),
            name: "web-app".into(),
            subtask_templates: vec![
                SubTaskTemplate {
                    name: "design".into(),
                    task_type: "design".into(),
                    specialist_type: "designer".into(),
                    dependencies: vec![],
                },
                SubTaskTemplate {
                    name: "implement-frontend".into(),
                    task_type: "frontend".into(),
                    specialist_type: "frontend-dev".into(),
                    dependencies: vec!["design".into()],
                },
                SubTaskTemplate {
                    name: "implement-backend".into(),
                    task_type: "backend".into(),
                    specialist_type: "backend-dev".into(),
                    dependencies: vec!["design".into()],
                },
                SubTaskTemplate {
                    name: "testing".into(),
                    task_type: "testing".into(),
                    specialist_type: "tester".into(),
                    dependencies: vec!["implement-frontend".into(), "implement-backend".into()],
                },
            ],
        },
        DecompositionRule {
            pattern: Regex::new(r"(?i)api|endpoint|rest").unwrap(),
            name: "api".into(),
            subtask_templates: vec![
                SubTaskTemplate {
                    name: "design-api".into(),
                    task_type: "design".into(),
                    specialist_type: "api-designer".into(),
                    dependencies: vec![],
                },
                SubTaskTemplate {
                    name: "implement-api".into(),
                    task_type: "backend".into(),
                    specialist_type: "backend-dev".into(),
                    dependencies: vec!["design-api".into()],
                },
                SubTaskTemplate {
                    name: "testing-api".into(),
                    task_type: "testing".into(),
                    specialist_type: "tester".into(),
                    dependencies: vec!["implement-api".into()],
                },
            ],
        },
        DecompositionRule {
            pattern: Regex::new(r"(?i)data.?analy|report|dashboard").unwrap(),
            name: "data-analysis".into(),
            subtask_templates: vec![
                SubTaskTemplate {
                    name: "collect-data".into(),
                    task_type: "data-collection".into(),
                    specialist_type: "data-engineer".into(),
                    dependencies: vec![],
                },
                SubTaskTemplate {
                    name: "analyze-data".into(),
                    task_type: "analysis".into(),
                    specialist_type: "data-analyst".into(),
                    dependencies: vec!["collect-data".into()],
                },
                SubTaskTemplate {
                    name: "generate-report".into(),
                    task_type: "reporting".into(),
                    specialist_type: "reporter".into(),
                    dependencies: vec!["analyze-data".into()],
                },
            ],
        },
        DecompositionRule {
            pattern: Regex::new(r"(?i)refactor|restructure|clean.?up").unwrap(),
            name: "refactoring".into(),
            subtask_templates: vec![
                SubTaskTemplate {
                    name: "analyze-code".into(),
                    task_type: "analysis".into(),
                    specialist_type: "code-analyst".into(),
                    dependencies: vec![],
                },
                SubTaskTemplate {
                    name: "plan-refactor".into(),
                    task_type: "planning".into(),
                    specialist_type: "architect".into(),
                    dependencies: vec!["analyze-code".into()],
                },
                SubTaskTemplate {
                    name: "execute-refactor".into(),
                    task_type: "refactoring".into(),
                    specialist_type: "developer".into(),
                    dependencies: vec!["plan-refactor".into()],
                },
                SubTaskTemplate {
                    name: "verify-refactor".into(),
                    task_type: "testing".into(),
                    specialist_type: "tester".into(),
                    dependencies: vec!["execute-refactor".into()],
                },
            ],
        },
    ];
}

// ── AgentOrchestrator ─────────────────────────────────────────────────

/// 任务编排器：将复杂请求分解为子任务，按依赖关系编排执行
///
/// 核心流程：decompose_task → topological_sort → compute_parallel_groups → execute
pub struct AgentOrchestrator {
    id: String,
    specialist_pool: Arc<SpecialistPool>,
    worker_pool: Arc<WorkerPool>,
    active_plans: Mutex<IndexMap<String, Arc<OrchestrationPlan>>>,
    config: OrchestratorConfig,
    running: AtomicBool,
    metrics: Mutex<OrchestratorMetrics>,
}

impl AgentOrchestrator {
    pub fn new(
        worker_pool: Arc<WorkerPool>,
        specialist_pool: Arc<SpecialistPool>,
        config: OrchestratorConfig,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            specialist_pool,
            worker_pool,
            active_plans: Mutex::new(IndexMap::new()),
            config,
            running: AtomicBool::new(false),
            metrics: Mutex::new(OrchestratorMetrics::default()),
        }
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
        let m = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        let plans = self.active_plans.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "id": self.id,
            "running": self.is_running(),
            "active_plans": plans.len(),
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
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    // ── Task Decomposition ────────────────────────────────────────

    /// 将请求文本分解为编排计划
    ///
    /// 匹配预定义规则生成子任务，无匹配时创建单个 generic 子任务。
    /// 自动计算拓扑排序和并行分组。
    pub fn decompose_task(&self, request: &str, trace_id: &str) -> OrchestrationPlan {
        let plan_id = Uuid::new_v4().to_string();
        let matched_rule = TASK_DECOMPOSITION_RULES
            .iter()
            .find(|rule| rule.pattern.is_match(request));

        let mut subtasks: IndexMap<String, Arc<SubTask>> = IndexMap::new();

        match matched_rule {
            Some(rule) => {
                for template in &rule.subtask_templates {
                    let sub_id = Uuid::new_v4().to_string();
                    let subtask = Arc::new(SubTask {
                        id: sub_id.clone(),
                        parent_id: plan_id.clone(),
                        trace_id: trace_id.to_string(),
                        name: template.name.clone(),
                        task_type: template.task_type.clone(),
                        description: format!(
                            "{} for: {}",
                            template.name, request
                        ),
                        dependencies: template
                            .dependencies
                            .iter()
                            .cloned()
                            .collect(),
                        priority: 5,
                        specialist_type: template.specialist_type.clone(),
                        payload: json!({
                            "request": request,
                            "subtask": template.name,
                            "rule": rule.name,
                        }),
                        status: Mutex::new(SubTaskStatus::Pending),
                        result: Mutex::new(None),
                        started_at: Mutex::new(None),
                        completed_at: Mutex::new(None),
                    });
                    subtasks.insert(template.name.clone(), subtask);
                }
            }
            None => {
                let sub_id = Uuid::new_v4().to_string();
                let subtask = Arc::new(SubTask {
                    id: sub_id.clone(),
                    parent_id: plan_id.clone(),
                    trace_id: trace_id.to_string(),
                    name: "generic".into(),
                    task_type: "generic".into(),
                    description: format!("Generic task: {}", request),
                    dependencies: HashSet::new(),
                    priority: 5,
                    specialist_type: "default".into(),
                    payload: json!({ "request": request }),
                    status: Mutex::new(SubTaskStatus::Pending),
                    result: Mutex::new(None),
                    started_at: Mutex::new(None),
                    completed_at: Mutex::new(None),
                });
                subtasks.insert("generic".into(), subtask);
            }
        }

        // Topological sort for execution_order
        let execution_order = Self::topological_sort(&subtasks);

        // Compute parallel groups
        let parallel_groups = Self::compute_parallel_groups(&subtasks, &execution_order);

        let total = subtasks.len() as u32;
        OrchestrationPlan {
            id: plan_id,
            trace_id: trace_id.to_string(),
            original_request: request.to_string(),
            subtasks,
            execution_order,
            parallel_groups,
            status: Mutex::new(PlanStatus::Created),
            progress: Mutex::new(PlanProgress {
                completed_count: 0,
                total_count: total,
                failed_count: 0,
            }),
            created_at: now_ms(),
        }
    }
    /// 拓扑排序：基于子任务依赖关系生成线性执行顺序（DFS 后序）
    fn topological_sort(subtasks: &IndexMap<String, Arc<SubTask>>) -> Vec<String> {
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
    fn compute_parallel_groups(
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

    // ── Orchestration ─────────────────────────────────────────────

    /// 编排入口：分解请求 → 存储计划 → 执行 → 清理 → 更新指标
    pub fn orchestrate(&self, request: &str) -> TaskResult {
        let trace_id = Uuid::new_v4().to_string();
        let started = now_ms();

        // Decompose
        let plan = Arc::new(self.decompose_task(request, &trace_id));

        // Store in active plans
        {
            let mut plans = self.active_plans.lock().unwrap_or_else(|e| e.into_inner());
            plans.insert(plan.id.clone(), Arc::clone(&plan));
        }
        {
            let mut m = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
            m.plans_created += 1;
        }

        // Execute
        let result = self.execute_orchestration_plan(&plan);

        // Clean up completed plan
        {
            let mut plans = self.active_plans.lock().unwrap_or_else(|e| e.into_inner());
            plans.shift_remove(&plan.id);
        }

        // Update metrics
        let elapsed = now_ms().saturating_sub(started);
        {
            let mut m = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
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
        *plan.status.lock().unwrap_or_else(|e| e.into_inner()) = PlanStatus::Running;

        for group in &plan.parallel_groups {
            if group.len() == 1 || !self.config.enable_parallel {
                // 单任务组或未启用并行：逐个串行执行
                for name in group {
                    let result = self.execute_subtask(plan, name);
                    if result.status != "success" {
                        *plan.status.lock().unwrap_or_else(|e| e.into_inner()) =
                            PlanStatus::Failed;
                        return result;
                    }
                }
            } else {
                // 多任务组且启用并行：先全部提交，再统一收集结果
                let mut receivers = Vec::new();
                for name in group {
                    if let Some(subtask) = plan.subtasks.get(name) {
                        *subtask.status.lock().unwrap_or_else(|e| e.into_inner()) =
                            SubTaskStatus::Running;
                        *subtask.started_at.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(now_ms());

                        let task = make_task(MakeTask {
                            trace_id: Some(subtask.trace_id.clone()),
                            task_type: subtask.task_type.clone(),
                            payload: Some(subtask.payload.clone()),
                            priority: Some(subtask.priority as u8),
                            timeout_ms: Some(self.config.default_timeout_ms),
                            metadata: Some(json!({
                                "subtask_id": subtask.id,
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
                        *subtask.completed_at.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(now_ms());

                        let mut m = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
                        m.subtasks_executed += 1;

                        if result.status == "success" {
                            *subtask.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                SubTaskStatus::Completed;
                        } else {
                            *subtask.status.lock().unwrap_or_else(|e| e.into_inner()) =
                                SubTaskStatus::Failed;
                            m.subtasks_failed += 1;
                            failed = true;
                            if fail_result.is_none() {
                                fail_result = Some(result.clone());
                            }
                        }
                        *subtask.result.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(result.clone());
                    }

                    let mut progress =
                        plan.progress.lock().unwrap_or_else(|e| e.into_inner());
                    if result.status == "success" {
                        progress.completed_count += 1;
                    } else {
                        progress.failed_count += 1;
                    }
                }

                if failed {
                    *plan.status.lock().unwrap_or_else(|e| e.into_inner()) = PlanStatus::Failed;
                    return fail_result.unwrap();
                }
            }
        }

        *plan.status.lock().unwrap_or_else(|e| e.into_inner()) = PlanStatus::Completed;

        make_task_result(MakeTaskResult {
            task_id: plan.id.clone(),
            trace_id: plan.trace_id.clone(),
            status: "success".into(),
            result: Some(json!({
                "plan_id": plan.id,
                "request": plan.original_request,
                "subtasks_completed": plan.subtasks.len(),
            })),
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

        *subtask.status.lock().unwrap_or_else(|e| e.into_inner()) = SubTaskStatus::Running;
        *subtask.started_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_ms());

        let task = make_task(MakeTask {
            trace_id: Some(subtask.trace_id.clone()),
            task_type: subtask.task_type.clone(),
            payload: Some(subtask.payload.clone()),
            priority: Some(subtask.priority as u8),
            timeout_ms: Some(self.config.default_timeout_ms),
            metadata: Some(json!({
                "subtask_id": subtask.id,
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

        *subtask.completed_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(now_ms());

        {
            let mut m = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
            m.subtasks_executed += 1;
            if result.status != "success" {
                m.subtasks_failed += 1;
            }
        }

        if result.status == "success" {
            *subtask.status.lock().unwrap_or_else(|e| e.into_inner()) = SubTaskStatus::Completed;
            let mut progress = plan.progress.lock().unwrap_or_else(|e| e.into_inner());
            progress.completed_count += 1;
        } else {
            *subtask.status.lock().unwrap_or_else(|e| e.into_inner()) = SubTaskStatus::Failed;
            let mut progress = plan.progress.lock().unwrap_or_else(|e| e.into_inner());
            progress.failed_count += 1;
        }

        *subtask.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result.clone());
        result
    }

    // ── Backward-compatible associated functions ──────────────────

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
                    let result = Self::wait_for_task(worker_pool, task);
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
        Self::wait_for_task(worker_pool, task)
    }

    /// 同步等待任务结果，通道关闭时返回失败
    pub fn wait_for_task(worker_pool: &Arc<WorkerPool>, task: Task) -> TaskResult {
        let rx = worker_pool.submit_task(task.clone());
        rx.recv().unwrap_or_else(|_| {
            make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Task result channel closed".into()),
                duration_ms: 0,
                worker_id: None,
                result: None,
            })
        })
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
