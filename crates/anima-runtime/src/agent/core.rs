//! # 核心智能体模块
//!
//! 本模块定义了 Anima 运行时的核心智能体架构，分为两层：
//! - `Agent`：对外门面，提供简洁的创建/启停/消息投递接口
//! - `CoreAgent`：内部引擎，负责消息循环、会话管理、任务编排、缓存和指标采集
//!
//! 消息处理流程：
//! 1. 入站消息通过 Bus 到达 CoreAgent 的消息循环
//! 2. 确保会话上下文存在（ensure_context），获取或创建 SDK 会话
//! 3. AgentClassifier 对消息进行分类，生成执行计划（direct / single / multi-step）
//! 4. AgentOrchestrator 将计划分发给 WorkerPool 执行
//! 5. 结果经 extract_response_text 提取后，通过 Bus 发送出站消息

use anima_sdk::facade::Client as SdkClient;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

use super::executor::{SdkTaskExecutor, TaskExecutor};
use super::types::{
    make_task, make_task_result, ExecutionPlan, MakeTask, MakeTaskResult, TaskResult,
};
use super::worker::{WorkerPool, WorkerPoolStatus};
use crate::classifier::rule::AgentClassifier;
use crate::execution::agentic_loop::{run_agentic_loop, AgenticLoopConfig};
use crate::hooks::HookRegistry;
use crate::orchestrator::core::{AgentOrchestrator, OrchestratorConfig};
use crate::orchestrator::specialist_pool::SpecialistPool;
use crate::permissions::PermissionChecker;
use crate::prompt::PromptAssembler;
use crate::tools::registry::ToolRegistry;
use crate::bus::{make_outbound, make_internal, Bus, InboundMessage, MakeInternal, MakeOutbound, OutboundMessage};
use crate::bus::{ControlSignal};
use crate::channel::SessionStore;
use crate::execution::context_assembly::{
    assemble_context, ContextAssemblyMode, ContextAssemblyRequest, ContextAssemblyResult,
};
use crate::execution::driver::{build_api_call_task, execute_api_call, ApiCallExecutionRequest, ExecutionKind};
use crate::execution::turn_coordinator::{
    assembly_mode_for_source, plan_turn_outcome, prepare_completed_branch_data,
    prepare_followup_branch_data, prepare_question_asked_payload,
    prepare_requirement_evaluation, prepare_requirement_evaluation_started_payload,
    prepare_requirement_satisfied_payload, prepare_upstream_response_observed_payload,
    prepare_waiting_user_input_branch_data, RequirementEvaluationPreparation,
    TurnOutcomePlan, TurnSource,
};
use crate::execution::requirement_judge::{judge_requirement, AgentFollowupPlan, RequirementProgressState};
use crate::support::{make_api_cache_key, now_ms, ContextManager, LruCache, MetricsCollector};

#[derive(Debug, Clone)]
struct RuntimeErrorInfo {
    code: &'static str,
    stage: &'static str,
    user_message: String,
    internal_message: String,
}

// 事件类型已下沉到 anima-types::event，此处 re-export
pub use anima_types::event::{
    ExecutionStageDurations, ExecutionSummary, RuntimeFailureSnapshot, RuntimeFailureStatus,
    RuntimeTimelineEvent,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionKind {
    Confirm,
    Choice,
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionDecisionMode {
    AutoAllowed,
    UserRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionRiskLevel {
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub question_id: String,
    pub job_id: String,
    pub opencode_session_id: String,
    pub question_kind: QuestionKind,
    pub prompt: String,
    pub options: Vec<String>,
    pub raw_question: Value,
    pub decision_mode: QuestionDecisionMode,
    pub risk_level: QuestionRiskLevel,
    pub requires_user_confirmation: bool,
    pub asked_at_ms: u64,
    pub answer_submitted: bool,
    pub answer_summary: Option<String>,
    pub resolution_source: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub inbound: Option<InboundMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionAnswerInput {
    pub question_id: String,
    pub source: String,
    pub answer_type: String,
    pub answer: String,
}

/// 单个会话的上下文信息，包含 SDK 会话 ID 和对话历史
#[derive(Debug, Clone, PartialEq)]
pub struct SessionContext {
    pub session_id: Option<String>,
    pub chat_id: String,
    pub channel: String,
    pub history: Vec<Value>,
}

/// 内存中最多保留的会话数，超出时淘汰最早的会话
const MAX_SESSIONS: usize = 1000;
/// 每个会话最多保留的历史消息条数
const MAX_SESSION_HISTORY: usize = 200;
const DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
struct ExecutionContext {
    memory_key: String,
    history_session_id: String,
    opencode_session_id: String,
    plan_type: String,
    context_ms: u64,
    session_ms: u64,
    classify_ms: u64,
    execute_ms: u64,
    total_ms: u64,
    cache_hit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuccessSource {
    Initial,
    QuestionContinuation,
    AgentFollowup,
}

fn turn_source_from_success(source: SuccessSource) -> TurnSource {
    match source {
        SuccessSource::Initial => TurnSource::Initial,
        SuccessSource::QuestionContinuation => TurnSource::QuestionContinuation,
        SuccessSource::AgentFollowup => TurnSource::AgentFollowup,
    }
}

/// 核心智能体，承载消息循环、会话管理、任务调度等核心逻辑。
/// 通过 Bus 接收入站消息，经分类和编排后交由 WorkerPool 执行。
pub struct CoreAgent {
    bus: Arc<Bus>,
    _client: SdkClient,
    session_store: Arc<SessionStore>,
    worker_pool: Arc<WorkerPool>,
    orchestrator: Arc<AgentOrchestrator>,
    context_manager: Arc<ContextManager>,
    result_cache: Arc<LruCache>,
    metrics: Arc<MetricsCollector>,
    /// Agentic loop 用：工具注册中心
    tool_registry: Arc<ToolRegistry>,
    /// Agentic loop 用：权限检查器
    permission_checker: Option<Arc<PermissionChecker>>,
    /// Agentic loop 用：钩子注册中心
    hook_registry: Option<Arc<HookRegistry>>,
    /// Agentic loop 用：保存 executor 引用以便在循环中直接使用
    executor: Arc<dyn TaskExecutor>,
    memory: Mutex<indexmap::IndexMap<String, SessionContext>>,
    failures: Mutex<RuntimeFailureStatus>,
    timeline: Mutex<Vec<RuntimeTimelineEvent>>,
    execution_summaries: Mutex<Vec<ExecutionSummary>>,
    pending_questions: Mutex<HashMap<String, PendingQuestion>>,
    requirement_progress: Mutex<HashMap<String, RequirementProgressState>>,
    running: AtomicBool,
    loop_handle: Mutex<Option<thread::JoinHandle<()>>>,
    control_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

/// CoreAgent 的运行状态快照，用于健康检查和监控
#[derive(Debug, Clone, PartialEq)]
pub struct CoreAgentStatus {
    pub status: String,
    pub sessions_count: usize,
    pub worker_pool: WorkerPoolStatus,
    pub context_status: String,
    pub cache_entries: usize,
    pub metrics: crate::support::MetricsSnapshot,
    pub recent_sessions: Vec<SessionContext>,
    pub failures: RuntimeFailureStatus,
    pub runtime_timeline: Vec<RuntimeTimelineEvent>,
    pub recent_execution_summaries: Vec<ExecutionSummary>,
}

impl CoreAgent {
    /// 创建 CoreAgent，初始化 WorkerPool、缓存、指标采集器等组件
    pub fn new(
        bus: Arc<Bus>,
        client: SdkClient,
        session_store: Option<Arc<SessionStore>>,
        executor: Arc<dyn TaskExecutor>,
        pool_size: Option<usize>,
    ) -> Self {
        let metrics = Arc::new(MetricsCollector::new(Some("anima")));
        metrics.register_agent_metrics();
        let session_store = session_store.unwrap_or_else(|| Arc::new(SessionStore::new()));
        let worker_pool = Arc::new(WorkerPool::new(client.clone(), executor.clone(), pool_size, None, None));
        let specialist_pool = Arc::new(SpecialistPool::new(worker_pool.clone()));
        let orchestrator = Arc::new(AgentOrchestrator::new(
            worker_pool.clone(),
            specialist_pool,
            OrchestratorConfig::default(),
        ));
        Self {
            bus,
            _client: client.clone(),
            session_store,
            worker_pool,
            orchestrator,
            context_manager: Arc::new(ContextManager::new(Some(true))),
            result_cache: Arc::new(LruCache::new(Some(1000), Some(5 * 60 * 1000))),
            metrics,
            tool_registry: Arc::new(ToolRegistry::new()),
            permission_checker: None,
            hook_registry: None,
            executor,
            memory: Mutex::new(indexmap::IndexMap::new()),
            failures: Mutex::new(RuntimeFailureStatus::default()),
            timeline: Mutex::new(Vec::new()),
            execution_summaries: Mutex::new(Vec::new()),
            pending_questions: Mutex::new(HashMap::new()),
            requirement_progress: Mutex::new(HashMap::new()),
            running: AtomicBool::new(false),
            loop_handle: Mutex::new(None),
            control_handle: Mutex::new(None),
        }
    }

    /// 设置工具注册中心（启用 agentic loop）
    pub fn set_tool_registry(&mut self, registry: ToolRegistry) {
        self.tool_registry = Arc::new(registry);
    }

    /// 设置权限检查器
    pub fn set_permission_checker(&mut self, checker: PermissionChecker) {
        self.permission_checker = Some(Arc::new(checker));
    }

    /// 设置钩子注册中心
    pub fn set_hook_registry(&mut self, registry: HookRegistry) {
        self.hook_registry = Some(Arc::new(registry));
    }

    /// 启动智能体：启动 WorkerPool，并创建两个后台线程：
    /// 1. 控制信号监听线程（处理 Shutdown / Pause / Resume）
    /// 2. 入站消息循环线程（从 Bus 接收并处理消息）
    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        self.worker_pool.start();
        self.orchestrator.start();

        // Control signal listener
        let agent_ctrl = Arc::clone(self);
        let ctrl_handle = thread::spawn(move || {
            let control_rx = agent_ctrl.bus.control_receiver();
            while agent_ctrl.running.load(Ordering::SeqCst) {
                match control_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(msg) => match msg.signal {
                        ControlSignal::Shutdown => {
                            agent_ctrl.running.store(false, Ordering::SeqCst);
                            break;
                        }
                        ControlSignal::Pause => {
                            agent_ctrl.metrics.counter_inc("agent.paused");
                        }
                        ControlSignal::Resume => {
                            agent_ctrl.metrics.counter_inc("agent.resumed");
                        }
                        _ => {}
                    },
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if agent_ctrl.bus.is_closed() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        // Inbound message loop
        let agent = Arc::clone(self);
        let handle = thread::spawn(move || {
            let inbound_rx = agent.bus.inbound_receiver();
            while agent.running.load(Ordering::SeqCst) {
                match inbound_rx.recv_timeout(Duration::from_millis(25)) {
                    Ok(msg) => agent.process_inbound_message(msg),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if agent.bus.is_closed() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        *self.loop_handle.lock() = Some(handle);
        *self.control_handle.lock() = Some(ctrl_handle);
    }

    /// 停止智能体，关闭 WorkerPool 并等待后台线程退出
    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        self.worker_pool.stop();
        self.orchestrator.stop();
        if let Some(handle) = self.loop_handle.lock().take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.control_handle.lock().take() {
            let _ = handle.join();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> CoreAgentStatus {
        let memory = self.memory.lock();
        let failures = self.failures.lock().clone();
        let timeline = self.timeline.lock().clone();
        let summaries = self.execution_summaries.lock().clone();
        CoreAgentStatus {
            status: if self.is_running() {
                "running"
            } else {
                "stopped"
            }
            .into(),
            sessions_count: memory.len(),
            worker_pool: self.worker_pool.status(),
            context_status: self.context_manager.status().status,
            cache_entries: self.result_cache.stats().entry_count,
            metrics: self.metrics.snapshot(),
            recent_sessions: memory.values().rev().take(5).cloned().collect(),
            failures,
            runtime_timeline: timeline,
            recent_execution_summaries: summaries,
        }
    }

    /// 处理一条入站消息的完整流程：
    /// 上下文初始化 → 获取/创建 SDK 会话 → 分类 → 缓存检查 → 编排执行 → 发送响应
    pub fn process_inbound_message(&self, inbound_msg: InboundMessage) {
        let started = now_ms();
        self.metrics.counter_inc("messages_received");
        let key = memory_key(&inbound_msg);
        let context_started = now_ms();
        self.publish_runtime_event("message_received", &inbound_msg, json!({
            "memory_key": key,
            "content_preview": truncate_preview(&inbound_msg.content, 120),
        }));
        self.ensure_context(&inbound_msg, &key);
        let user_entry = json!({"role": "user", "content": inbound_msg.content.clone()});
        self.append_history(&key, user_entry.clone());
        let history_session_id = inbound_msg
            .chat_id
            .clone()
            .unwrap_or_else(|| inbound_msg.id.clone());
        self.context_manager
            .add_to_session_history(&history_session_id, user_entry);
        self.metrics.update_session_gauge(self.memory.lock().len());
        let context_ms = now_ms().saturating_sub(context_started);

        let session_started = now_ms();
        let opencode_session_id = match self.get_or_create_opencode_session(&inbound_msg, &key) {
            Ok(session_id) => session_id,
            Err(error_text) => {
                let error_info = classify_runtime_error(Some(error_text.as_str()), Some("session_create"));
                self.record_failure(&inbound_msg, &error_info);
                self.metrics.counter_inc("messages_failed");
                self.publish_runtime_event("session_create_failed", &inbound_msg, json!({
                    "memory_key": key,
                    "error_code": error_info.code,
                    "error_stage": error_info.stage,
                    "error_message": error_info.internal_message,
                }));
                self.send_error_response(&inbound_msg, &error_info);
                return;
            }
        };
        let session_ms = now_ms().saturating_sub(session_started);
        self.publish_runtime_event("session_ready", &inbound_msg, json!({
            "memory_key": key,
            "opencode_session_id": opencode_session_id.clone(),
        }));

        // 分类消息：direct 类型直接响应，无需走 Worker
        let classify_started = now_ms();
        let plan = AgentClassifier::build_plan(&inbound_msg);
        let classify_ms = now_ms().saturating_sub(classify_started);
        self.publish_runtime_event("plan_built", &inbound_msg, json!({
            "memory_key": key,
            "plan_type": plan.plan_type.clone(),
            "plan_kind": format!("{:?}", plan.kind),
            "task_count": plan.tasks.len(),
            "specialist": plan.specialist.clone(),
        }));
        if plan.plan_type == "direct" {
            let duration_ms = now_ms().saturating_sub(started);
            self.metrics.counter_inc("messages_processed");
            self.metrics
                .histogram_record("message_latency", duration_ms);
            self.record_execution_summary(ExecutionSummary {
                trace_id: inbound_msg.id.clone(),
                message_id: inbound_msg.id.clone(),
                channel: inbound_msg.channel.clone(),
                chat_id: inbound_msg.chat_id.clone(),
                plan_type: plan.plan_type.clone(),
                status: "success".into(),
                cache_hit: false,
                worker_id: None,
                error_code: None,
                error_stage: None,
                task_duration_ms: 0,
                stages: ExecutionStageDurations {
                    context_ms,
                    session_ms,
                    classify_ms,
                    execute_ms: 0,
                    total_ms: duration_ms,
                },
            });
            self.publish_runtime_event("message_completed", &inbound_msg, json!({
                "memory_key": key,
                "plan_type": plan.plan_type,
                "status": "success",
                "cached": false,
                "duration_ms": duration_ms,
                "response_preview": "Command processed.",
            }));
            self.send_response(&inbound_msg, "Command processed.");
            return;
        }

        // single 类型先查缓存，命中则直接返回；multi-step 类型不走缓存
        let cache_key = make_api_cache_key(&opencode_session_id, &inbound_msg.content);
        let use_orchestration_v1 = AgentClassifier::should_upgrade_to_orchestration_v1(&inbound_msg);
        let plan_type = if use_orchestration_v1 {
            "orchestration-v1".to_string()
        } else {
            plan.plan_type.clone()
        };
        let execute_started = now_ms();
        let mut cache_hit = false;
        let result = if use_orchestration_v1 {
            self.publish_runtime_event("orchestration_selected", &inbound_msg, json!({
                "memory_key": key,
                "plan_type": plan.plan_type.clone(),
                "selected_plan_type": plan_type.clone(),
                "reason": "web_complex_request",
            }));
            match self.orchestrator.execute_orchestration_for_main_chain(
                &inbound_msg.content,
                &inbound_msg.id,
                &inbound_msg.id,
                &opencode_session_id,
                |event, payload| self.publish_runtime_event(event, &inbound_msg, payload),
            ) {
                Ok(execution) => execution.result,
                Err(error) => {
                    self.publish_runtime_event("orchestration_fallback", &inbound_msg, json!({
                        "memory_key": key,
                        "plan_type": plan_type.clone(),
                        "fallback_plan_type": plan.plan_type.clone(),
                        "reason": error,
                    }));
                    self.execute_initial_plan(
                        &inbound_msg,
                        &plan,
                        &opencode_session_id,
                        &key,
                        ExecutionKind::Initial,
                    )
                }
            }
        } else if plan_type == "single" {
            if let Some(cached) = self.result_cache.get(&cache_key) {
                cache_hit = true;
                self.metrics.counter_inc("cache_hits");
                self.publish_runtime_event("cache_hit", &inbound_msg, json!({
                    "memory_key": key,
                    "cache_key": cache_key.clone(),
                    "plan_type": plan_type.clone(),
                }));
                make_task_result(MakeTaskResult {
                    task_id: Uuid::new_v4().to_string(),
                    trace_id: inbound_msg.id.clone(),
                    status: "success".into(),
                    result: Some(cached),
                    error: None,
                    duration_ms: 0,
                    worker_id: None,
                })
            } else {
                self.metrics.counter_inc("cache_misses");
                self.publish_runtime_event("cache_miss", &inbound_msg, json!({
                    "memory_key": key,
                    "cache_key": cache_key.clone(),
                    "plan_type": plan_type.clone(),
                }));
                let result = self.execute_initial_plan(
                    &inbound_msg,
                    &plan,
                    &opencode_session_id,
                    &key,
                    ExecutionKind::Initial,
                );
                if result.status == "success" {
                    if let Some(value) = result.result.clone() {
                        self.result_cache.set(&cache_key, value, None);
                    }
                }
                result
            }
        } else {
            self.execute_initial_plan(
                &inbound_msg,
                &plan,
                &opencode_session_id,
                &key,
                ExecutionKind::Initial,
            )
        };

        let total_ms = now_ms().saturating_sub(started);
        let execute_ms = now_ms().saturating_sub(execute_started);
        self.metrics
            .histogram_record("message_latency", total_ms);
        let worker_statuses = self.worker_pool.status().workers;
        let active = worker_statuses
            .iter()
            .filter(|worker| worker.status == "busy")
            .count();
        let idle = worker_statuses
            .iter()
            .filter(|worker| worker.status == "idle")
            .count();
        self.metrics.update_worker_gauges(active, idle);

        if result.status == "success" {
            let initial_context = self.assemble_turn_context(
                &inbound_msg,
                ContextAssemblyMode::Initial,
                opencode_session_id.clone(),
                None,
                Some(inbound_msg.content.clone()),
            );
            let exec_ctx = ExecutionContext {
                memory_key: initial_context.metadata.memory_key,
                history_session_id: initial_context.metadata.history_session_id,
                opencode_session_id: initial_context.metadata.opencode_session_id,
                plan_type: plan_type.clone(),
                context_ms,
                session_ms,
                classify_ms,
                execute_ms,
                total_ms,
                cache_hit,
            };
            let _ = self.handle_upstream_result(
                &inbound_msg,
                &exec_ctx,
                result,
                SuccessSource::Initial,
                None,
            );
        } else {
            let error_info = classify_runtime_error(result.error.as_deref(), Some("plan_execute"));
            self.record_failure(&inbound_msg, &error_info);
            self.metrics.counter_inc("messages_failed");
            self.record_execution_summary(ExecutionSummary {
                trace_id: inbound_msg.id.clone(),
                message_id: inbound_msg.id.clone(),
                channel: inbound_msg.channel.clone(),
                chat_id: inbound_msg.chat_id.clone(),
                plan_type: plan_type.clone(),
                status: result.status.clone(),
                cache_hit,
                worker_id: result.worker_id.clone(),
                error_code: Some(error_info.code.to_string()),
                error_stage: Some(error_info.stage.to_string()),
                task_duration_ms: result.duration_ms,
                stages: ExecutionStageDurations {
                    context_ms,
                    session_ms,
                    classify_ms,
                    execute_ms,
                    total_ms,
                },
            });
            self.publish_runtime_event("message_failed", &inbound_msg, json!({
                "memory_key": key,
                "plan_type": plan_type,
                "status": result.status.clone(),
                "cached": cache_hit,
                "worker_id": result.worker_id.clone(),
                "task_duration_ms": result.duration_ms,
                "message_latency_ms": total_ms,
                "error_code": error_info.code,
                "error_stage": error_info.stage,
                "error": error_info.internal_message,
            }));
            self.send_error_response(&inbound_msg, &error_info);
        }
    }

    /// 获取已有的 SDK 会话 ID，若不存在则通过 WorkerPool 创建新会话
    fn get_or_create_opencode_session(
        &self,
        inbound_msg: &InboundMessage,
        key: &str,
    ) -> Result<String, String> {
        if let Some(existing_id) = self
            .memory
            .lock()
            .get(key)
            .and_then(|ctx| ctx.session_id.clone())
        {
            return Ok(existing_id);
        }

        let task = make_task(MakeTask {
            trace_id: Some(inbound_msg.id.clone()),
            task_type: "session-create".into(),
            payload: Some(json!({})),
            ..Default::default()
        });
        self.publish_runtime_event("worker_task_assigned", inbound_msg, json!({
            "memory_key": key,
            "plan_type": "session-create",
            "task_id": task.id,
            "task_type": task.task_type,
            "task_summary": "为当前 job 创建上游会话",
            "task_preview": "创建新的上游会话",
        }));
        let result = self
            .worker_pool
            .submit_task(task)
            .recv()
            .map_err(|error| format!("Failed to receive session-create result: {error}"))?;

        if result.status != "success" {
            let error_text = result
                .error
                .clone()
                .unwrap_or_else(|| format!("Failed to create session: task status={}", result.status));
            return Err(error_text);
        }

        if let Some(error_text) = result.error.clone() {
            return Err(error_text);
        }

        let session_id = result
            .result
            .as_ref()
            .and_then(|value| value.get("opencode-session-id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Failed to create session: no ID returned".to_string())?
            .to_string();

        self.memory.lock().entry(key.to_string()).and_modify(|ctx| {
            ctx.session_id = Some(session_id.clone());
        });

        Ok(session_id)
    }

    /// 确保消息对应的会话上下文存在。
    /// 若内存中会话数达到上限，淘汰最早插入的会话（LRU 策略）。
    fn ensure_context(&self, inbound_msg: &InboundMessage, key: &str) {
        let chat_id = inbound_msg
            .chat_id
            .clone()
            .unwrap_or_else(|| inbound_msg.id.clone());
        {
            let mut memory = self.memory.lock();
            if !memory.contains_key(key) && memory.len() >= MAX_SESSIONS {
                // Evict oldest entry (first in insertion order)
                if let Some(oldest_key) = memory.keys().next().cloned() {
                    memory.shift_remove(&oldest_key);
                }
            }
            memory
                .entry(key.to_string())
                .or_insert_with(|| SessionContext {
                    session_id: None,
                    chat_id,
                    channel: inbound_msg.channel.clone(),
                    history: Vec::new(),
                });
        }
        let _ = self.session_store.find_or_create_session(
            &inbound_msg.channel,
            crate::channel::FindSessionOptions {
                session_id: inbound_msg.chat_id.clone(),
                routing_key: inbound_msg.session_key.clone(),
                account_id: inbound_msg
                    .metadata
                    .get("account-id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            },
        );
    }

    /// 追加消息到会话历史，超过上限时裁剪最早的记录
    fn append_history(&self, key: &str, message: Value) {
        if let Some(ctx) = self.memory.lock().get_mut(key) {
            ctx.history.push(message);
            if ctx.history.len() > MAX_SESSION_HISTORY {
                let excess = ctx.history.len() - MAX_SESSION_HISTORY;
                ctx.history.drain(..excess);
            }
        }
    }

    fn send_response(&self, inbound_msg: &InboundMessage, response_text: &str) -> OutboundMessage {
        let outbound = make_outbound(MakeOutbound {
            channel: inbound_msg.channel.clone(),
            account_id: inbound_msg
                .metadata
                .get("account-id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            chat_id: inbound_msg.chat_id.clone(),
            content: response_text.to_string(),
            reply_target: Some(inbound_msg.sender_id.clone()),
            stage: Some("final".into()),
            ..Default::default()
        });
        let _ = self.bus.publish_outbound(outbound.clone());
        outbound
    }

    fn record_failure(&self, inbound_msg: &InboundMessage, error: &RuntimeErrorInfo) {
        let mut failures = self.failures.lock();
        let count = failures
            .counts_by_error_code
            .entry(error.code.to_string())
            .or_insert(0);
        *count += 1;
        failures.last_failure = Some(RuntimeFailureSnapshot {
            error_code: error.code.to_string(),
            error_stage: error.stage.to_string(),
            message_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            occurred_at_ms: now_ms(),
            internal_message: error.internal_message.clone(),
        });
    }

    fn record_timeline_event(&self, event: &str, inbound_msg: &InboundMessage, payload: Value) {
        let mut timeline = self.timeline.lock();
        timeline.push(RuntimeTimelineEvent {
            event: event.to_string(),
            trace_id: inbound_msg.id.clone(),
            message_id: runtime_message_id(inbound_msg),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            sender_id: inbound_msg.sender_id.clone(),
            recorded_at_ms: now_ms(),
            payload,
        });
        if timeline.len() > 50 {
            let excess = timeline.len() - 50;
            timeline.drain(..excess);
        }
    }

    fn record_execution_summary(&self, summary: ExecutionSummary) {
        let mut summaries = self.execution_summaries.lock();
        summaries.push(summary);
        if summaries.len() > 20 {
            let excess = summaries.len() - 20;
            summaries.drain(..excess);
        }
    }

    fn latest_execution_summary_for(&self, inbound_msg: &InboundMessage) -> Option<ExecutionSummary> {
        self.execution_summaries
            .lock()
            .iter()
            .rev()
            .find(|summary| {
                summary.trace_id == inbound_msg.id
                    || summary.message_id == runtime_message_id(inbound_msg)
            })
            .cloned()
    }

    fn execute_api_call_request(
        &self,
        inbound_msg: &InboundMessage,
        memory_key: &str,
        plan_type: &str,
        request: &ApiCallExecutionRequest,
        task_summary: &str,
        extra_payload: Value,
    ) -> Result<TaskResult, String> {
        let task = build_api_call_task(request);
        let extra_payload_for_started = extra_payload.clone();
        let mut assigned_payload = json!({
            "memory_key": memory_key,
            "plan_type": plan_type,
            "task_id": task.id.clone(),
            "task_type": "api-call",
            "execution_kind": execution_kind_label(request.kind),
            "task_summary": task_summary,
            "task_preview": truncate_preview(&request.content, 160),
            "opencode_session_id": request.session_id.clone(),
        });
        if let Some(map) = assigned_payload.as_object_mut() {
            if let Some(extra) = extra_payload.as_object() {
                for (key, value) in extra {
                    map.insert(key.clone(), value.clone());
                }
            }
        }
        self.publish_runtime_event("worker_task_assigned", inbound_msg, assigned_payload);
        self.publish_runtime_event("api_call_started", inbound_msg, json!({
            "memory_key": memory_key,
            "plan_type": plan_type,
            "task_id": task.id.clone(),
            "task_type": "api-call",
            "execution_kind": execution_kind_label(request.kind),
            "request_preview": truncate_preview(&request.content, 160),
            "opencode_session_id": request.session_id.clone(),
            "question_id": extra_payload_for_started
                .get("question_id")
                .cloned()
                .unwrap_or(Value::Null),
        }));
        execute_api_call(&self.worker_pool, request.clone())
    }

    fn execute_initial_plan(
        &self,
        inbound_msg: &InboundMessage,
        plan: &ExecutionPlan,
        opencode_session_id: &str,
        memory_key: &str,
        execution_kind: ExecutionKind,
    ) -> TaskResult {
        let plan_type = plan.plan_type.clone();
        if plan.tasks.len() == 1 {
            if let Some(task) = plan.tasks.front() {
                let task_preview = task
                    .payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|text| truncate_preview(text, 160));
                self.publish_runtime_event("worker_task_assigned", inbound_msg, json!({
                    "memory_key": memory_key,
                    "plan_type": plan_type.clone(),
                    "task_id": task.id.clone(),
                    "task_type": task.task_type.clone(),
                    "execution_kind": execution_kind_label(execution_kind),
                    "task_summary": "主 agent 已将任务派发给 worker",
                    "task_preview": task_preview,
                    "opencode_session_id": opencode_session_id,
                }));
                if task.task_type == "api-call" {
                    self.publish_runtime_event("api_call_started", inbound_msg, json!({
                        "memory_key": memory_key,
                        "plan_type": plan_type.clone(),
                        "task_id": task.id.clone(),
                        "task_type": task.task_type.clone(),
                        "execution_kind": execution_kind_label(execution_kind),
                        "request_preview": task_preview,
                        "opencode_session_id": opencode_session_id,
                    }));
                }
            }
        }
        self.metrics.counter_inc("tasks_submitted");
        self.publish_worker_event("task_start", inbound_msg, &plan_type);

        // Agentic loop 分支：工具注册中心非空时，走 agentic loop
        let result = if !self.tool_registry.is_empty() {
            self.run_agentic_loop_for_plan(inbound_msg, opencode_session_id)
        } else {
            self.orchestrator
                .execute_plan_instance(plan, opencode_session_id)
        };

        self.publish_worker_event("task_end", inbound_msg, &plan_type);
        if result.status == "success" {
            self.metrics.counter_inc("tasks_completed");
        } else {
            self.metrics.counter_inc("tasks_failed");
        }
        result
    }

    /// 使用 agentic loop 执行当前消息
    fn run_agentic_loop_for_plan(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
    ) -> TaskResult {
        use crate::messages::types::{InternalMsg, MessageRole};
        let started = now_ms();
        let initial_messages = vec![InternalMsg {
            role: MessageRole::User,
            content: json!(inbound_msg.content.clone()),
            message_id: inbound_msg.id.clone(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }];

        let system_prompt = {
            let mut asm = PromptAssembler::new();
            asm.add_text("identity", "你是 Anima 智能助手。", 0);
            Some(asm.build().text)
        };

        let config = AgenticLoopConfig {
            max_iterations: 10,
            session_id: opencode_session_id.to_string(),
            trace_id: inbound_msg.id.clone(),
            compact: None,
            system_prompt,
            tool_definitions: Some(self.tool_registry.tool_definitions()),
        };

        let perm_ref = self.permission_checker.as_deref();
        let hook_ref = self.hook_registry.as_deref();

        match run_agentic_loop(
            &self._client,
            self.executor.as_ref(),
            &self.tool_registry,
            perm_ref,
            hook_ref,
            initial_messages,
            &config,
        ) {
            Ok(loop_result) => {
                let duration_ms = now_ms().saturating_sub(started);
                make_task_result(MakeTaskResult {
                    task_id: Uuid::new_v4().to_string(),
                    trace_id: inbound_msg.id.clone(),
                    status: "success".into(),
                    result: Some(json!({
                        "text": loop_result.final_text,
                        "iterations": loop_result.iterations,
                        "hit_limit": loop_result.hit_limit,
                    })),
                    error: None,
                    duration_ms,
                    worker_id: None,
                })
            }
            Err(err) => {
                let duration_ms = now_ms().saturating_sub(started);
                make_task_result(MakeTaskResult {
                    task_id: Uuid::new_v4().to_string(),
                    trace_id: inbound_msg.id.clone(),
                    status: "error".into(),
                    result: None,
                    error: Some(err.to_string()),
                    duration_ms,
                    worker_id: None,
                })
            }
        }
    }

    fn assemble_turn_context(
        &self,
        inbound_msg: &InboundMessage,
        mode: ContextAssemblyMode,
        opencode_session_id: String,
        pending_question: Option<PendingQuestion>,
        followup_prompt: Option<String>,
    ) -> ContextAssemblyResult {
        let progress = self.ensure_requirement_progress(inbound_msg);
        assemble_context(ContextAssemblyRequest {
            mode,
            inbound_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            original_user_request: progress.original_user_request.clone(),
            opencode_session_id,
            pending_question,
            latest_summary: self.latest_execution_summary_for(inbound_msg),
            attempted_rounds: progress.attempted_rounds,
            max_rounds: progress.max_rounds,
            previous_fingerprint: progress.last_result_fingerprint.clone(),
            followup_prompt,
        })
    }

    /// 发布 Worker 进度事件到 internal bus，供 Web UI 等消费
    fn publish_worker_event(&self, event: &str, inbound_msg: &InboundMessage, task_type: &str) {
        let worker_statuses = self.worker_pool.status().workers;
        for w in &worker_statuses {
            let status = if event == "task_start" && w.status == "busy" {
                "busy"
            } else if event == "task_end" {
                "idle"
            } else {
                &w.status
            };
            let _ = self.bus.publish_internal(make_internal(MakeInternal {
                source: "core-agent".into(),
                trace_id: Some(inbound_msg.id.clone()),
                payload: json!({
                    "event": format!("worker_{}", event),
                    "worker_id": w.id,
                    "status": status,
                    "task_type": task_type,
                    "channel": inbound_msg.channel,
                    "message_id": inbound_msg.id,
                    "chat_id": inbound_msg.chat_id,
                }),
                ..Default::default()
            }));
        }
    }

    fn publish_runtime_event(
        &self,
        event: &str,
        inbound_msg: &InboundMessage,
        payload: Value,
    ) {
        let payload = merge_runtime_metadata(inbound_msg, payload);
        let message_id = runtime_message_id(inbound_msg);
        self.record_timeline_event(event, inbound_msg, payload.clone());
        let _ = self.bus.publish_internal(make_internal(MakeInternal {
            source: "core-agent".into(),
            trace_id: Some(inbound_msg.id.clone()),
            payload: json!({
                "event": event,
                "message_id": message_id,
                "channel": inbound_msg.channel,
                "chat_id": inbound_msg.chat_id,
                "sender_id": inbound_msg.sender_id,
                "payload": payload,
            }),
            ..Default::default()
        }));
    }

    fn send_error_response(
        &self,
        inbound_msg: &InboundMessage,
        error: &RuntimeErrorInfo,
    ) -> OutboundMessage {
        let outbound = make_outbound(MakeOutbound {
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            content: format!("Error [{}]: {}", error.code, error.user_message),
            reply_target: Some(inbound_msg.sender_id.clone()),
            stage: Some("final".into()),
            ..Default::default()
        });
        let _ = self.bus.publish_outbound(outbound.clone());
        outbound
    }

    fn store_pending_question(&self, job_id: String, question: PendingQuestion) {
        self.pending_questions.lock().insert(job_id, question);
    }

    fn clear_pending_question(&self, job_id: &str) {
        self.pending_questions.lock().remove(job_id);
    }

    fn publish_question_asked(
        &self,
        inbound_msg: &InboundMessage,
        key: &str,
        question: &PendingQuestion,
        worker_id: Option<String>,
        total_ms: u64,
        task_duration_ms: u64,
        cache_hit: bool,
        plan_type: &str,
    ) {
        let asked_payload = prepare_question_asked_payload(question);
        if matches!(question.decision_mode, QuestionDecisionMode::UserRequired) {
            self.publish_runtime_event("question_escalated_to_user", inbound_msg, json!({
                "memory_key": key,
                "plan_type": plan_type,
                "cached": cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": total_ms,
                "question_id": asked_payload.question_id,
                "question_kind": question_kind_str(&question.question_kind),
                "prompt": asked_payload.prompt.clone(),
                "options": asked_payload.options.clone(),
                "decision_mode": question_decision_mode_str(&question.decision_mode),
                "risk_level": question_risk_level_str(&question.risk_level),
                "requires_user_confirmation": asked_payload.requires_user_confirmation,
                "opencode_session_id": asked_payload.opencode_session_id.clone(),
                "raw_question": asked_payload.raw_question.clone(),
            }));
        }
        self.publish_runtime_event("question_asked", inbound_msg, json!({
            "memory_key": key,
            "plan_type": plan_type,
            "cached": cache_hit,
            "worker_id": worker_id,
            "task_duration_ms": task_duration_ms,
            "message_latency_ms": total_ms,
            "question_id": asked_payload.question_id,
            "question_kind": question_kind_str(&question.question_kind),
            "prompt": asked_payload.prompt,
            "options": asked_payload.options,
            "decision_mode": question_decision_mode_str(&question.decision_mode),
            "risk_level": question_risk_level_str(&question.risk_level),
            "requires_user_confirmation": asked_payload.requires_user_confirmation,
            "opencode_session_id": asked_payload.opencode_session_id,
            "raw_question": asked_payload.raw_question,
        }));
    }

    fn ensure_requirement_progress(&self, inbound_msg: &InboundMessage) -> RequirementProgressState {
        let mut progress = self.requirement_progress.lock();
        progress
            .entry(inbound_msg.id.clone())
            .or_insert_with(|| RequirementProgressState {
                original_user_request: inbound_msg.content.clone(),
                attempted_rounds: 0,
                max_rounds: DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS,
                last_result_fingerprint: None,
                last_reason: None,
            })
            .clone()
    }

    fn update_requirement_progress(
        &self,
        job_id: &str,
        attempted_rounds: usize,
        last_result_fingerprint: Option<String>,
        last_reason: Option<String>,
    ) {
        if let Some(progress) = self.requirement_progress.lock().get_mut(job_id) {
            progress.attempted_rounds = attempted_rounds;
            progress.last_result_fingerprint = last_result_fingerprint;
            progress.last_reason = last_reason;
        }
    }

    fn clear_requirement_progress(&self, job_id: &str) {
        self.requirement_progress.lock().remove(job_id);
    }

    fn publish_requirement_satisfied(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        response_text: &str,
    ) {
        let satisfied_payload = prepare_requirement_satisfied_payload(response_text);
        self.publish_runtime_event("requirement_satisfied", inbound_msg, json!({
            "memory_key": exec_ctx.memory_key,
            "plan_type": exec_ctx.plan_type,
            "cached": exec_ctx.cache_hit,
            "worker_id": worker_id,
            "task_duration_ms": task_duration_ms,
            "message_latency_ms": exec_ctx.total_ms,
            "response_preview": satisfied_payload.response_preview,
        }));
    }

    fn schedule_agent_followup(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        plan: AgentFollowupPlan,
    ) -> Result<(), String> {
        let progress = self.ensure_requirement_progress(inbound_msg);
        let next_round = progress.attempted_rounds + 1;
        let fingerprint = plan.result_fingerprint.clone();

        let followup_branch = prepare_followup_branch_data(
            plan.reason.clone(),
            plan.missing_requirements.clone(),
            plan.followup_prompt.clone(),
            fingerprint.clone(),
            next_round,
            progress.max_rounds,
            worker_id.clone(),
            task_duration_ms,
            exec_ctx.context_ms,
            exec_ctx.session_ms,
            exec_ctx.classify_ms,
            exec_ctx.execute_ms,
            exec_ctx.total_ms,
        );
        self.publish_runtime_event("requirement_unsatisfied", inbound_msg, json!({
            "memory_key": exec_ctx.memory_key,
            "plan_type": exec_ctx.plan_type,
            "cached": exec_ctx.cache_hit,
            "worker_id": worker_id,
            "task_duration_ms": task_duration_ms,
            "message_latency_ms": exec_ctx.total_ms,
            "attempted_rounds": progress.attempted_rounds,
            "max_rounds": progress.max_rounds,
            "result_fingerprint": fingerprint,
            "payload": followup_branch.unsatisfied.payload,
        }));

        if next_round > progress.max_rounds
            || progress
                .last_result_fingerprint
                .as_ref()
                .map(|previous| previous == &fingerprint)
                .unwrap_or(false)
        {
            self.update_requirement_progress(
                &inbound_msg.id,
                next_round,
                Some(fingerprint.clone()),
                Some(plan.reason.clone()),
            );
            self.record_execution_summary(ExecutionSummary {
                trace_id: inbound_msg.id.clone(),
                message_id: inbound_msg.id.clone(),
                channel: inbound_msg.channel.clone(),
                chat_id: inbound_msg.chat_id.clone(),
                plan_type: exec_ctx.plan_type.clone(),
                status: followup_branch.exhausted_summary.status,
                cache_hit: exec_ctx.cache_hit,
                worker_id: followup_branch.exhausted_summary.worker_id.clone(),
                error_code: followup_branch.exhausted_summary.error_code.clone(),
                error_stage: followup_branch.exhausted_summary.error_stage.clone(),
                task_duration_ms: followup_branch.exhausted_summary.task_duration_ms,
                stages: followup_branch.exhausted_summary.stages.clone(),
            });
            let followup_exhausted_payload = followup_branch.exhausted.clone();
            self.publish_runtime_event("requirement_followup_exhausted", inbound_msg, json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "cached": exec_ctx.cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "attempted_rounds": followup_exhausted_payload.attempted_rounds,
                "max_rounds": followup_exhausted_payload.max_rounds,
                "reason": followup_exhausted_payload.reason,
                "missing_requirements": followup_exhausted_payload.missing_requirements,
                "result_fingerprint": followup_exhausted_payload.result_fingerprint,
            }));
            self.publish_runtime_event("message_failed", inbound_msg, json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "status": "followup_exhausted",
                "cached": exec_ctx.cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "error_code": "requirement_followup_exhausted",
                "error_stage": "requirement_judge",
                "error": plan.reason,
            }));
            self.send_error_response(
                inbound_msg,
                &RuntimeErrorInfo {
                    code: "requirement_followup_exhausted",
                    stage: "requirement_judge",
                    user_message: "主 agent 多轮自动补充后仍未能确认需求已满足，请查看任务事件诊断原因。".into(),
                    internal_message: plan.reason,
                },
            );
            return Ok(());
        }

        self.update_requirement_progress(
            &inbound_msg.id,
            next_round,
            Some(fingerprint),
            Some(plan.reason.clone()),
        );
        self.record_execution_summary(ExecutionSummary {
            trace_id: inbound_msg.id.clone(),
            message_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            plan_type: exec_ctx.plan_type.clone(),
            status: followup_branch.pending_summary.status,
            cache_hit: exec_ctx.cache_hit,
            worker_id: followup_branch.pending_summary.worker_id.clone(),
            error_code: followup_branch.pending_summary.error_code.clone(),
            error_stage: followup_branch.pending_summary.error_stage.clone(),
            task_duration_ms: followup_branch.pending_summary.task_duration_ms,
            stages: followup_branch.pending_summary.stages.clone(),
        });
        let followup_scheduled_payload = followup_branch.scheduled.clone();
        self.publish_runtime_event("requirement_followup_scheduled", inbound_msg, json!({
            "memory_key": exec_ctx.memory_key,
            "plan_type": exec_ctx.plan_type,
            "cached": exec_ctx.cache_hit,
            "worker_id": worker_id,
            "task_duration_ms": task_duration_ms,
            "message_latency_ms": exec_ctx.total_ms,
            "attempted_rounds": followup_scheduled_payload.attempted_rounds,
            "max_rounds": followup_scheduled_payload.max_rounds,
            "reason": followup_scheduled_payload.reason,
            "missing_requirements": followup_scheduled_payload.missing_requirements,
            "followup_prompt": followup_scheduled_payload.followup_prompt,
        }));

        let followup_assembly = self.assemble_turn_context(
            inbound_msg,
            ContextAssemblyMode::Followup,
            exec_ctx.opencode_session_id.clone(),
            None,
            Some(plan.followup_prompt.clone()),
        );
        let followup_request = ApiCallExecutionRequest {
            trace_id: inbound_msg.id.clone(),
            session_id: followup_assembly.metadata.opencode_session_id.clone(),
            content: followup_assembly.prompt_text.clone(),
            kind: ExecutionKind::Followup,
            metadata: Some(json!({})),
        };
        let followup_result = self.execute_api_call_request(
            inbound_msg,
            &exec_ctx.memory_key,
            &exec_ctx.plan_type,
            &followup_request,
            "主 agent 触发自动 followup，派发给 worker 继续推进",
            json!({}),
        )?;

        if followup_result.status != "success" {
            let error_info = classify_runtime_error(followup_result.error.as_deref(), Some("plan_execute"));
            self.record_failure(inbound_msg, &error_info);
            self.publish_runtime_event("message_failed", inbound_msg, json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "status": followup_result.status,
                "cached": false,
                "worker_id": followup_result.worker_id,
                "task_duration_ms": followup_result.duration_ms,
                "message_latency_ms": exec_ctx.total_ms.saturating_add(followup_result.duration_ms),
                "error_code": error_info.code,
                "error_stage": error_info.stage,
                "error": error_info.internal_message,
            }));
            self.send_error_response(inbound_msg, &error_info);
            return Ok(());
        }

        let followup_ctx = ExecutionContext {
            memory_key: exec_ctx.memory_key.clone(),
            history_session_id: exec_ctx.history_session_id.clone(),
            opencode_session_id: exec_ctx.opencode_session_id.clone(),
            plan_type: exec_ctx.plan_type.clone(),
            context_ms: exec_ctx.context_ms,
            session_ms: exec_ctx.session_ms,
            classify_ms: exec_ctx.classify_ms,
            execute_ms: exec_ctx.execute_ms.saturating_add(followup_result.duration_ms),
            total_ms: exec_ctx.total_ms.saturating_add(followup_result.duration_ms),
            cache_hit: false,
        };
        self.handle_upstream_result(
            inbound_msg,
            &followup_ctx,
            followup_result,
            SuccessSource::AgentFollowup,
            None,
        )?;
        Ok(())
    }

    fn handle_upstream_result(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        result: TaskResult,
        source: SuccessSource,
        resolved_question: Option<&PendingQuestion>,
    ) -> Result<Option<PendingQuestion>, String> {
        let task_type = "api-call".to_string();
        let provider = infer_provider(result.result.as_ref());
        let operation = infer_operation(result.result.as_ref());
        let response_text = extract_response_text(result.result.as_ref());
        let observed_payload = prepare_upstream_response_observed_payload(
            result.worker_id.clone(),
            task_type,
            provider,
            operation,
            exec_ctx.opencode_session_id.clone(),
            truncate_preview(&response_text, 200),
            result.result.clone(),
        );
        self.publish_runtime_event("upstream_response_observed", inbound_msg, json!({
            "memory_key": exec_ctx.memory_key,
            "plan_type": exec_ctx.plan_type,
            "worker_id": observed_payload.worker_id,
            "task_type": observed_payload.task_type,
            "provider": observed_payload.provider,
            "operation": observed_payload.operation,
            "task_duration_ms": result.duration_ms,
            "message_latency_ms": exec_ctx.total_ms,
            "opencode_session_id": observed_payload.opencode_session_id,
            "response_preview": observed_payload.response_preview,
            "raw_result": observed_payload.raw_result,
        }));
        let preparation: RequirementEvaluationPreparation = prepare_requirement_evaluation(
            turn_source_from_success(source),
            self.assemble_turn_context(
                inbound_msg,
                assembly_mode_for_source(turn_source_from_success(source)),
                exec_ctx.opencode_session_id.clone(),
                resolved_question.cloned(),
                None,
            ),
            inbound_msg.id.clone(),
            inbound_msg.id.clone(),
            inbound_msg.chat_id.clone(),
            result.result.clone(),
        );
        let evaluation_started_payload =
            prepare_requirement_evaluation_started_payload(preparation.source_label);
        self.publish_runtime_event("requirement_evaluation_started", inbound_msg, json!({
            "memory_key": exec_ctx.memory_key,
            "plan_type": exec_ctx.plan_type,
            "worker_id": result.worker_id.clone(),
            "task_duration_ms": result.duration_ms,
            "message_latency_ms": exec_ctx.total_ms,
            "source": evaluation_started_payload.source,
        }));

        let judgement = judge_requirement(&preparation.judge_context);
        let turn_plan = plan_turn_outcome(judgement, resolved_question.is_some());

        match turn_plan {
            TurnOutcomePlan::Complete {
                should_resolve_question,
            } => {
                let branch = prepare_completed_branch_data(
                    resolved_question,
                    should_resolve_question,
                    extract_response_text(result.result.as_ref()),
                    result.worker_id.clone(),
                    result.duration_ms,
                    exec_ctx.context_ms,
                    exec_ctx.session_ms,
                    exec_ctx.classify_ms,
                    exec_ctx.execute_ms,
                    exec_ctx.total_ms,
                );
                if let Some(resolved_payload) = branch.resolved_question.as_ref() {
                    let pending = resolved_question.expect("resolved question should exist when completed branch includes resolved payload");
                    self.publish_runtime_event("question_resolved", inbound_msg, json!({
                        "memory_key": exec_ctx.memory_key,
                        "question_id": resolved_payload.question_id,
                        "question_kind": question_kind_str(&pending.question_kind),
                        "answer_summary": resolved_payload.answer_summary,
                        "resolution_source": resolved_payload.resolution_source,
                        "opencode_session_id": resolved_payload.opencode_session_id,
                    }));
                }
                self.clear_pending_question(&inbound_msg.id);
                self.clear_requirement_progress(&inbound_msg.id);
                self.append_history(&exec_ctx.memory_key, branch.completion.assistant_entry.clone());
                self.context_manager
                    .add_to_session_history(&exec_ctx.history_session_id, branch.completion.assistant_entry.clone());
                self.metrics.counter_inc("messages_processed");
                self.record_execution_summary(ExecutionSummary {
                    trace_id: inbound_msg.id.clone(),
                    message_id: inbound_msg.id.clone(),
                    channel: inbound_msg.channel.clone(),
                    chat_id: inbound_msg.chat_id.clone(),
                    plan_type: exec_ctx.plan_type.clone(),
                    status: branch.summary.status,
                    cache_hit: exec_ctx.cache_hit,
                    worker_id: branch.summary.worker_id,
                    error_code: branch.summary.error_code,
                    error_stage: branch.summary.error_stage,
                    task_duration_ms: branch.summary.task_duration_ms,
                    stages: branch.summary.stages,
                });
                self.publish_requirement_satisfied(
                    inbound_msg,
                    exec_ctx,
                    result.worker_id.clone(),
                    result.duration_ms,
                    &branch.completion.response_text,
                );
                self.publish_runtime_event("message_completed", inbound_msg, json!({
                    "memory_key": exec_ctx.memory_key,
                    "plan_type": exec_ctx.plan_type,
                    "status": branch.message_completed.status,
                    "cached": exec_ctx.cache_hit,
                    "worker_id": result.worker_id.clone(),
                    "task_duration_ms": result.duration_ms,
                    "message_latency_ms": exec_ctx.total_ms,
                    "response_preview": branch.message_completed.response_preview,
                    "response_text": branch.message_completed.response_text,
                }));
                self.send_response(inbound_msg, &branch.completion.response_text);
                Ok(resolved_question.cloned())
            }
            TurnOutcomePlan::AskUserInput {
                should_resolve_question,
                requirement,
            } => {
                let branch = prepare_waiting_user_input_branch_data(
                    resolved_question,
                    should_resolve_question,
                    requirement,
                    inbound_msg,
                    result.worker_id.clone(),
                    result.duration_ms,
                    exec_ctx.context_ms,
                    exec_ctx.session_ms,
                    exec_ctx.classify_ms,
                    exec_ctx.execute_ms,
                    exec_ctx.total_ms,
                    result.result.as_ref(),
                );
                if let Some(resolved_payload) = branch.resolved_question.as_ref() {
                    let resolved = resolved_question.expect("resolved question should exist when waiting branch includes resolved payload");
                    self.publish_runtime_event("question_resolved", inbound_msg, json!({
                        "memory_key": exec_ctx.memory_key,
                        "question_id": resolved_payload.question_id,
                        "question_kind": question_kind_str(&resolved.question_kind),
                        "answer_summary": resolved_payload.answer_summary,
                        "resolution_source": resolved_payload.resolution_source,
                        "opencode_session_id": resolved_payload.opencode_session_id,
                    }));
                }
                self.publish_runtime_event("requirement_unsatisfied", inbound_msg, json!({
                    "memory_key": exec_ctx.memory_key,
                    "plan_type": exec_ctx.plan_type,
                    "cached": exec_ctx.cache_hit,
                    "worker_id": result.worker_id.clone(),
                    "task_duration_ms": result.duration_ms,
                    "message_latency_ms": exec_ctx.total_ms,
                    "payload": branch.unsatisfied.payload,
                }));
                self.publish_runtime_event("user_input_required", inbound_msg, json!({
                    "memory_key": exec_ctx.memory_key,
                    "plan_type": exec_ctx.plan_type,
                    "reason": branch.user_input_required.reason,
                    "missing_requirements": branch.user_input_required.missing_requirements,
                    "question_id": branch.user_input_required.question_id,
                    "question_kind": question_kind_str(&branch.waiting.question.question_kind),
                    "prompt": branch.user_input_required.prompt,
                }));
                let question = branch.waiting.question;
                self.store_pending_question(inbound_msg.id.clone(), question.clone());
                self.record_execution_summary(ExecutionSummary {
                    trace_id: inbound_msg.id.clone(),
                    message_id: inbound_msg.id.clone(),
                    channel: inbound_msg.channel.clone(),
                    chat_id: inbound_msg.chat_id.clone(),
                    plan_type: exec_ctx.plan_type.clone(),
                    status: branch.summary.status,
                    cache_hit: exec_ctx.cache_hit,
                    worker_id: branch.summary.worker_id,
                    error_code: branch.summary.error_code,
                    error_stage: branch.summary.error_stage,
                    task_duration_ms: branch.summary.task_duration_ms,
                    stages: branch.summary.stages,
                });
                self.publish_question_asked(
                    inbound_msg,
                    &exec_ctx.memory_key,
                    &question,
                    result.worker_id,
                    exec_ctx.total_ms,
                    result.duration_ms,
                    exec_ctx.cache_hit,
                    &exec_ctx.plan_type,
                );
                Ok(Some(question))
            }
            TurnOutcomePlan::ScheduleFollowup { plan } => {
                self.schedule_agent_followup(
                    inbound_msg,
                    exec_ctx,
                    result.worker_id,
                    result.duration_ms,
                    plan,
                )?;
                Ok(resolved_question.cloned())
            }
        }
    }

    pub fn pending_question_for(&self, job_id: &str) -> Option<PendingQuestion> {
        self.pending_questions.lock().get(job_id).cloned()
    }

    pub fn submit_question_answer(
        &self,
        job_id: &str,
        answer: QuestionAnswerInput,
    ) -> Result<PendingQuestion, String> {
        let pending = {
            let mut pending_questions = self.pending_questions.lock();
            let pending = pending_questions
                .get_mut(job_id)
                .ok_or_else(|| format!("No pending question for job: {job_id}"))?;

            if pending.question_id != answer.question_id {
                return Err(format!(
                    "Question ID mismatch for job {job_id}: expected {}, got {}",
                    pending.question_id, answer.question_id
                ));
            }

            pending.answer_submitted = true;
            pending.answer_summary = Some(truncate_preview(&answer.answer, 120));
            pending.resolution_source = Some(answer.source.clone());
            pending.clone()
        };

        let inbound = pending
            .inbound
            .clone()
            .ok_or_else(|| format!("Missing inbound context for pending question on job: {job_id}"))?;
        let key = memory_key(&inbound);

        self.publish_runtime_event("question_answer_submitted", &inbound, json!({
            "memory_key": key,
            "question_id": pending.question_id,
            "question_kind": question_kind_str(&pending.question_kind),
            "answer_type": answer.answer_type,
            "answer": answer.answer,
            "answer_summary": pending.answer_summary,
            "resolution_source": pending.resolution_source,
            "opencode_session_id": pending.opencode_session_id,
        }));

        let continuation_ctx = self.assemble_turn_context(
            &inbound,
            ContextAssemblyMode::QuestionContinuation,
            pending.opencode_session_id.clone(),
            Some(pending.clone()),
            None,
        );
        let continuation_request = ApiCallExecutionRequest {
            trace_id: inbound.id.clone(),
            session_id: continuation_ctx.metadata.opencode_session_id.clone(),
            content: continuation_ctx.prompt_text.clone(),
            kind: ExecutionKind::QuestionContinuation,
            metadata: Some(json!({})),
        };
        let continuation_result = self.execute_api_call_request(
            &inbound,
            &key,
            "single",
            &continuation_request,
            "主 agent 已收到问题答案，派发给 worker 继续执行",
            json!({
                "question_id": pending.question_id,
            }),
        )?;

        if continuation_result.status != "success" {
            return Err(
                continuation_result
                    .error
                    .unwrap_or_else(|| "Continuation execution failed".to_string()),
            );
        }

        let exec_ctx = ExecutionContext {
            memory_key: continuation_ctx.metadata.memory_key,
            history_session_id: continuation_ctx.metadata.history_session_id,
            opencode_session_id: continuation_ctx.metadata.opencode_session_id,
            plan_type: "single".into(),
            context_ms: 0,
            session_ms: 0,
            classify_ms: 0,
            execute_ms: continuation_result.duration_ms,
            total_ms: continuation_result.duration_ms,
            cache_hit: false,
        };
        let _ = self.handle_upstream_result(
            &inbound,
            &exec_ctx,
            continuation_result,
            SuccessSource::QuestionContinuation,
            Some(&pending),
        )?;

        Ok(pending)
    }
}

fn question_kind_str(kind: &QuestionKind) -> &'static str {
    match kind {
        QuestionKind::Confirm => "confirm",
        QuestionKind::Choice => "choice",
        QuestionKind::Input => "input",
    }
}

fn question_decision_mode_str(mode: &QuestionDecisionMode) -> &'static str {
    match mode {
        QuestionDecisionMode::AutoAllowed => "auto_allowed",
        QuestionDecisionMode::UserRequired => "user_required",
    }
}

fn question_risk_level_str(level: &QuestionRiskLevel) -> &'static str {
    match level {
        QuestionRiskLevel::Low => "low",
        QuestionRiskLevel::High => "high",
    }
}

pub(crate) fn detect_pending_question(result: Option<&Value>, opencode_session_id: &str) -> Option<PendingQuestion> {
    let response = result?;

    if response.get("type").and_then(Value::as_str) == Some("question") {
        let question = response.get("question")?;
        return build_pending_question(question, opencode_session_id);
    }

    if let Some(question) = response.get("question") {
        return build_pending_question(question, opencode_session_id);
    }

    None
}

fn build_pending_question(question: &Value, opencode_session_id: &str) -> Option<PendingQuestion> {
    let prompt = question
        .get("prompt")
        .or_else(|| question.get("message"))
        .or_else(|| question.get("text"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let options = question
        .get("options")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| {
                    value
                        .get("value")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                        .or_else(|| value.get("label").and_then(Value::as_str).map(ToString::to_string))
                        .or_else(|| value.as_str().map(ToString::to_string))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let question_kind = classify_question_kind(question, &options);
    let decision_mode = classify_question_decision_mode(&prompt, &options);
    let risk_level = classify_question_risk_level(&prompt, &options);
    let requires_user_confirmation = matches!(decision_mode, QuestionDecisionMode::UserRequired);

    Some(PendingQuestion {
        question_id: question
            .get("question_id")
            .or_else(|| question.get("id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        job_id: String::new(),
        opencode_session_id: opencode_session_id.to_string(),
        question_kind,
        prompt,
        options,
        raw_question: question.clone(),
        decision_mode,
        risk_level,
        requires_user_confirmation,
        asked_at_ms: now_ms(),
        answer_submitted: false,
        answer_summary: None,
        resolution_source: None,
        inbound: None,
    })
}

fn classify_question_kind(question: &Value, options: &[String]) -> QuestionKind {
    match question.get("kind").and_then(Value::as_str) {
        Some("confirm") => QuestionKind::Confirm,
        Some("choice") => QuestionKind::Choice,
        Some("input") => QuestionKind::Input,
        _ if !options.is_empty() => QuestionKind::Choice,
        _ => QuestionKind::Input,
    }
}

fn classify_question_decision_mode(prompt: &str, options: &[String]) -> QuestionDecisionMode {
    if classify_question_requires_user_confirmation(prompt, options) {
        QuestionDecisionMode::UserRequired
    } else {
        QuestionDecisionMode::AutoAllowed
    }
}

fn classify_question_risk_level(prompt: &str, options: &[String]) -> QuestionRiskLevel {
    if classify_question_requires_user_confirmation(prompt, options) {
        QuestionRiskLevel::High
    } else {
        QuestionRiskLevel::Low
    }
}

pub(crate) fn classify_question_requires_user_confirmation(prompt: &str, options: &[String]) -> bool {
    let normalized = format!("{} {}", prompt.to_ascii_lowercase(), options.join(" ").to_ascii_lowercase());
    [
        "delete",
        "remove",
        "overwrite",
        "rollback",
        "force",
        "deploy",
        "production",
        "database",
        "sql",
        "drop table",
        "grant",
        "permission",
        "credential",
        "secret",
        "token",
        "send email",
        "send message",
        "post",
        "external",
        "write operation",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn classify_runtime_error(error: Option<&str>, fallback_stage: Option<&'static str>) -> RuntimeErrorInfo {
    let raw = error.unwrap_or("Unknown runtime error");
    let raw_lower = raw.to_ascii_lowercase();
    let is_session_create_stage = fallback_stage == Some("session_create");
    let is_plan_execute_stage = fallback_stage == Some("plan_execute");
    let looks_like_session_transport_error = raw_lower.contains("http transport error")
        || raw_lower.contains("error sending request")
        || raw_lower.contains("/session)")
        || raw_lower.contains("/session ")
        || raw_lower.ends_with("/session")
        || raw_lower.contains("/session?");
    let looks_like_upstream_stream_error = raw_lower.contains("empty_stream")
        || raw_lower.contains("upstream stream closed before first payload")
        || raw_lower.contains("stream disconnected before completion")
        || raw_lower.contains("stream closed before response.completed");
    let looks_like_upstream_timeout = raw_lower.contains("request timeout")
        || raw_lower.contains("408 request timeout")
        || raw_lower.contains("timed out")
        || raw_lower.contains("timeout");

    if raw.contains("OpenCode session")
        || raw.contains("no ID returned")
        || raw.contains("Failed to create session")
        || raw_lower.contains("create session")
        || raw_lower.contains("session-create")
        || (is_session_create_stage && looks_like_session_transport_error)
    {
        return RuntimeErrorInfo {
            code: "session_create_failed",
            stage: "session_create",
            user_message: "无法创建上游会话，请确认 opencode-server 是否正常运行。".into(),
            internal_message: raw.to_string(),
        };
    }

    if is_plan_execute_stage && looks_like_upstream_timeout {
        return RuntimeErrorInfo {
            code: "upstream_timeout",
            stage: "plan_execute",
            user_message: "上游模型响应超时，请稍后重试。".into(),
            internal_message: raw.to_string(),
        };
    }

    if is_plan_execute_stage && looks_like_upstream_stream_error {
        return RuntimeErrorInfo {
            code: "upstream_stream_failed",
            stage: "plan_execute",
            user_message: "上游模型流式响应异常中断，请稍后重试或检查代理服务状态。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Worker pool is not running") || raw.contains("Worker is not running") {
        return RuntimeErrorInfo {
            code: "worker_unavailable",
            stage: "worker_pool",
            user_message: "当前执行器未就绪，暂时无法处理请求。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Worker is busy") || raw.contains("No available worker") {
        return RuntimeErrorInfo {
            code: "worker_capacity_exhausted",
            stage: "worker_pool",
            user_message: "当前执行队列繁忙，请稍后再试。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Missing required fields") || raw.contains("Missing query") || raw.contains("Missing transform data") {
        return RuntimeErrorInfo {
            code: "invalid_task_payload",
            stage: fallback_stage.unwrap_or("task_execution"),
            user_message: "运行时生成了无效任务，请检查主链路任务构建逻辑。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Unknown task type") {
        return RuntimeErrorInfo {
            code: "unknown_task_type",
            stage: fallback_stage.unwrap_or("task_execution"),
            user_message: "运行时生成了未支持的任务类型。".into(),
            internal_message: raw.to_string(),
        };
    }

    RuntimeErrorInfo {
        code: "task_execution_failed",
        stage: fallback_stage.unwrap_or("task_execution"),
        user_message: "任务执行失败，请查看运行时事件获取详细原因。".into(),
        internal_message: raw.to_string(),
    }
}

/// 对外门面智能体，封装 CoreAgent 提供简洁的公共 API。
/// 使用者通过 Agent::create 创建实例，调用 start/stop 控制生命周期，
/// 通过 process_message 投递消息。
pub struct Agent {
    pub bus: Arc<Bus>,
    pub opencode_client: SdkClient,
    pub session_manager: Arc<SessionStore>,
    running: AtomicBool,
    core_agent: Arc<CoreAgent>,
}

/// Agent 的运行状态快照
#[derive(Debug, Clone, PartialEq)]
pub struct AgentStatus {
    pub running: bool,
    pub core: CoreAgentStatus,
}

impl Agent {
    /// 创建 Agent 实例，各参数均可选，使用合理默认值：
    /// - client: 默认连接 127.0.0.1:9711
    /// - executor: 默认使用 SdkTaskExecutor
    pub fn create(
        bus: Arc<Bus>,
        client: Option<SdkClient>,
        session_manager: Option<Arc<SessionStore>>,
        executor: Option<Arc<dyn TaskExecutor>>,
    ) -> Self {
        let opencode_client = client.unwrap_or_else(|| SdkClient::new("http://127.0.0.1:9711"));
        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionStore::new()));
        let core_agent = Arc::new(CoreAgent::new(
            bus.clone(),
            opencode_client.clone(),
            Some(session_manager.clone()),
            executor.unwrap_or_else(|| Arc::new(SdkTaskExecutor)),
            None,
        ));
        Self {
            bus,
            opencode_client,
            session_manager,
            running: AtomicBool::new(false),
            core_agent,
        }
    }

    pub fn start(&self) {
        if !self.running.swap(true, Ordering::SeqCst) {
            self.core_agent.start();
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.core_agent.stop();
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 将消息投递到 Bus，由 CoreAgent 的消息循环异步处理
    pub fn process_message(&self, inbound_msg: InboundMessage) {
        let _ = self.bus.publish_inbound(inbound_msg);
    }

    pub fn core_agent(&self) -> Arc<CoreAgent> {
        Arc::clone(&self.core_agent)
    }

    pub fn worker_pool(&self) -> Arc<WorkerPool> {
        Arc::clone(&self.core_agent.worker_pool)
    }

    pub fn status(&self) -> AgentStatus {
        AgentStatus {
            running: self.is_running(),
            core: self.core_agent.status(),
        }
    }

    pub fn pending_question_for(&self, job_id: &str) -> Option<PendingQuestion> {
        self.core_agent.pending_question_for(job_id)
    }

    pub fn submit_question_answer(
        &self,
        job_id: &str,
        answer: QuestionAnswerInput,
    ) -> Result<PendingQuestion, String> {
        self.core_agent.submit_question_answer(job_id, answer)
    }
}

/// 生成会话内存的 key，格式为 "channel:chat_id"
fn memory_key(inbound_msg: &InboundMessage) -> String {
    format!(
        "{}:{}",
        inbound_msg.channel,
        inbound_msg
            .chat_id
            .clone()
            .unwrap_or_else(|| inbound_msg.id.clone())
    )
}

/// 从 SDK 响应中提取文本内容。
/// 支持多种响应格式，按优先级依次尝试：
/// 1. parts 数组（含 reasoning + text 分区）
/// 2. data.messages 嵌套结构
/// 3. content 字符串字段
/// 4. 直接字符串值
/// 5. 兜底：序列化为 JSON 字符串
fn truncate_preview(text: &str, max_chars: usize) -> String {
    let truncated = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        format!("{}…", truncated)
    } else {
        truncated
    }
}

fn execution_kind_label(kind: ExecutionKind) -> &'static str {
    match kind {
        ExecutionKind::Initial => "initial",
        ExecutionKind::QuestionContinuation => "question_continuation",
        ExecutionKind::Followup => "followup",
    }
}

fn infer_provider(response: Option<&Value>) -> Option<String> {
    response
        .and_then(|value| value.get("provider").and_then(Value::as_str))
        .map(ToString::to_string)
        .or_else(|| Some("opencode".to_string()))
}

fn infer_operation(response: Option<&Value>) -> Option<String> {
    response
        .and_then(|value| value.get("operation").and_then(Value::as_str))
        .map(ToString::to_string)
        .or_else(|| {
            response
                .and_then(|value| value.get("type").and_then(Value::as_str))
                .map(ToString::to_string)
        })
        .or_else(|| Some("send_prompt".to_string()))
}

fn subtask_job_id(inbound_msg: &InboundMessage) -> Option<String> {
    inbound_msg
        .metadata
        .get("subtask_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn runtime_message_id(inbound_msg: &InboundMessage) -> String {
    subtask_job_id(inbound_msg).unwrap_or_else(|| inbound_msg.id.clone())
}

fn merge_runtime_metadata(inbound_msg: &InboundMessage, payload: Value) -> Value {
    let mut merged = match payload {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".into(), other);
            map
        }
    };

    for key in ["parent_job_id", "subtask_id", "plan_id"] {
        if let Some(value) = inbound_msg.metadata.get(key).cloned() {
            merged.entry(key.to_string()).or_insert(value);
        }
    }

    Value::Object(merged)
}

pub(crate) fn extract_response_text(response: Option<&Value>) -> String {
    let Some(response) = response else {
        return String::new();
    };

    if let Some(parts) = response.get("parts").and_then(Value::as_array) {
        let mut reasoning = Vec::new();
        let mut text = Vec::new();
        for part in parts {
            match part.get("type").and_then(Value::as_str) {
                Some("reasoning") => {
                    if let Some(content) = part.get("text").and_then(Value::as_str) {
                        reasoning.push(content.to_string());
                    }
                }
                Some("text") => {
                    if let Some(content) = part.get("text").and_then(Value::as_str) {
                        text.push(content.to_string());
                    }
                }
                _ => {}
            }
        }
        if !reasoning.is_empty() {
            return format!(
                "【Reasoning】\n{}\n【End Reasoning】\n\n{}",
                reasoning.join("\n"),
                text.join("\n")
            );
        }
        return text.join("\n");
    }

    if let Some(messages) = response
        .get("data")
        .and_then(|data| data.get("messages"))
        .and_then(Value::as_array)
    {
        let mut chunks = Vec::new();
        for message in messages {
            if let Some(parts) = message.get("parts").and_then(Value::as_array) {
                for part in parts {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("text") | Some("reasoning")
                    ) {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            chunks.push(text.to_string());
                        }
                    }
                }
            }
        }
        return chunks.join("\n");
    }

    if let Some(content) = response.get("content").and_then(Value::as_str) {
        return content.to_string();
    }

    if let Some(content) = response.as_str() {
        return content.to_string();
    }

    response.to_string()
}
