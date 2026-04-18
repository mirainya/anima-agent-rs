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
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::event_emitter::RuntimeEventEmitter;
use super::executor::{SdkTaskExecutor, TaskExecutor};
use super::requirement::RequirementCoordinator;
use super::runtime_helpers::truncate_preview;
use super::types::{ExecutionPlan, TaskResult};
use super::worker::{WorkerPool, WorkerPoolStatus};
use crate::bus::ControlSignal;
use crate::bus::{
    make_internal, make_outbound, Bus, InboundMessage, MakeInternal, MakeOutbound, OutboundMessage,
};
use crate::channel::SessionStore;
use crate::classifier::rule::AgentClassifier;
use crate::execution::agentic_loop::AgenticLoopConfig;
use crate::execution::context_assembly::{
    assemble_context, ContextAssemblyMode, ContextAssemblyRequest, ContextAssemblyResult,
};
use crate::execution::driver::ExecutionKind;
use crate::execution::turn_coordinator::{TurnOutcomePlan, TurnSource};
use crate::hooks::HookRegistry;
use crate::orchestrator::core::{AgentOrchestrator, OrchestratorConfig};
use crate::orchestrator::specialist_pool::SpecialistPool;
use crate::permissions::PermissionChecker;
use crate::runtime::{
    build_projection, RuntimeProjectionView, RuntimeStateSnapshot, RuntimeStateStore,
    SharedRuntimeStateStore,
};
use crate::runtime::{
    planning_ready_current_step, preparing_context_current_step, session_ready_current_step,
};
use crate::support::{now_ms, ContextManager, LruCache, MetricsCollector};
use crate::tasks::{RunStatus, SuspensionKind, SuspensionStatus, TaskStatus, TurnStatus};
use crate::tools::registry::ToolRegistry;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeErrorInfo {
    pub(crate) code: &'static str,
    pub(crate) stage: &'static str,
    pub(crate) user_message: String,
    pub(crate) internal_message: String,
}

// 事件类型已下沉到 anima-types::event，此处 re-export
pub use anima_types::event::{
    ExecutionStageDurations, ExecutionSummary, RuntimeFailureSnapshot, RuntimeFailureStatus,
    RuntimeTimelineEvent,
};

// 挂起相关类型从 suspension 模块导入
pub use super::suspension::{
    PendingQuestion, PendingQuestionSourceKind, QuestionAnswerInput, QuestionDecisionMode,
    QuestionKind, QuestionRiskLevel,
};
use super::suspension::{SuspendedToolInvocationState, SuspensionCoordinator};

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
pub(crate) const DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExecutionContext {
    pub(crate) memory_key: String,
    pub(crate) history_session_id: String,
    pub(crate) opencode_session_id: String,
    pub(crate) plan_type: String,
    pub(crate) context_ms: u64,
    pub(crate) session_ms: u64,
    pub(crate) classify_ms: u64,
    pub(crate) execute_ms: u64,
    pub(crate) total_ms: u64,
    pub(crate) cache_hit: bool,
}

pub(crate) struct ToolPermissionResumePreparation {
    pub(crate) suspended: SuspendedToolInvocationState,
    pub(crate) config: AgenticLoopConfig,
    pub(crate) resumed_messages: Vec<crate::messages::types::InternalMsg>,
}

pub(crate) struct InitialAgenticLoopRunPreparation {
    pub(crate) initial_messages: Vec<crate::messages::types::InternalMsg>,
    pub(crate) config: AgenticLoopConfig,
}

pub(crate) struct InitialPlanDispatchContext<'a> {
    pub(crate) inbound_msg: &'a InboundMessage,
    pub(crate) plan: &'a ExecutionPlan,
    pub(crate) opencode_session_id: &'a str,
    pub(crate) memory_key: &'a str,
    pub(crate) execution_kind: ExecutionKind,
}

pub(crate) struct InitialExecutionOutcome {
    pub(crate) plan_type: String,
    pub(crate) cache_hit: bool,
    pub(crate) execute_ms: u64,
    pub(crate) result: TaskResult,
}

pub(crate) struct ProcessInboundResolutionContext<'a> {
    pub(crate) inbound_msg: &'a InboundMessage,
    pub(crate) key: &'a str,
    pub(crate) opencode_session_id: &'a str,
    pub(crate) plan_type: &'a str,
    pub(crate) context_ms: u64,
    pub(crate) session_ms: u64,
    pub(crate) classify_ms: u64,
    pub(crate) execute_ms: u64,
    pub(crate) total_ms: u64,
    pub(crate) cache_hit: bool,
}

pub(crate) struct InboundContextPreparation {
    pub(crate) key: String,
    pub(crate) context_ms: u64,
}

pub(crate) struct SessionPreparation {
    pub(crate) opencode_session_id: String,
    pub(crate) session_ms: u64,
}

pub(crate) struct PlanPreparation {
    pub(crate) plan: ExecutionPlan,
    pub(crate) classify_ms: u64,
}

pub(crate) struct AgentFollowupPreparation {
    pub(crate) progress: crate::execution::requirement_judge::RequirementProgressState,
    pub(crate) next_round: usize,
    pub(crate) fingerprint: String,
    pub(crate) followup_branch: crate::execution::turn_coordinator::FollowupBranchData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeTaskPhase {
    Main,
    Question,
    ToolPermission,
    Followup,
    Requirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuccessSource {
    Initial,
    QuestionContinuation,
    AgentFollowup,
}

pub(crate) struct UpstreamTurnContext<'a> {
    pub(crate) inbound_msg: &'a InboundMessage,
    pub(crate) exec_ctx: &'a ExecutionContext,
    pub(crate) result: &'a TaskResult,
    pub(crate) resolved_question: Option<&'a PendingQuestion>,
    pub(crate) turn_source_label: &'a str,
}

pub(crate) struct UpstreamRequirementContext {
    pub(crate) turn_plan: TurnOutcomePlan,
    pub(crate) turn_source_label: &'static str,
}

pub(crate) fn turn_source_from_success(source: SuccessSource) -> TurnSource {
    match source {
        SuccessSource::Initial => TurnSource::Initial,
        SuccessSource::QuestionContinuation => TurnSource::QuestionContinuation,
        SuccessSource::AgentFollowup => TurnSource::AgentFollowup,
    }
}

/// 核心智能体，承载消息循环、会话管理、任务调度等核心逻辑。
/// 通过 Bus 接收入站消息，经分类和编排后交由 WorkerPool 执行。
pub struct CoreAgent {
    pub(crate) bus: Arc<Bus>,
    pub(crate) _client: SdkClient,
    session_store: Arc<SessionStore>,
    pub(crate) worker_pool: Arc<WorkerPool>,
    pub(crate) orchestrator: Arc<AgentOrchestrator>,
    pub(crate) context_manager: Arc<ContextManager>,
    pub(crate) result_cache: Arc<LruCache>,
    pub(crate) metrics: Arc<MetricsCollector>,
    /// Agentic loop 用：工具注册中心
    pub(crate) tool_registry: Arc<ToolRegistry>,
    /// Agentic loop 用：权限检查器
    pub(crate) permission_checker: Option<Arc<PermissionChecker>>,
    /// Agentic loop 用：钩子注册中心
    pub(crate) hook_registry: Option<Arc<HookRegistry>>,
    /// Agentic loop 用：保存 executor 引用以便在循环中直接使用
    pub(crate) executor: Arc<dyn TaskExecutor>,
    pub(crate) memory: Mutex<indexmap::IndexMap<String, SessionContext>>,
    pub(crate) emitter: Arc<RuntimeEventEmitter>,
    pub(crate) runtime_state_store: SharedRuntimeStateStore,
    /// 挂起状态协调器
    pub(crate) suspension: Arc<SuspensionCoordinator>,
    pub(crate) requirement: Arc<RequirementCoordinator>,
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
    pub tool_count: usize,
    pub pre_hook_count: usize,
    pub post_hook_count: usize,
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
        Self::new_with_runtime_state_store(
            bus,
            client,
            session_store,
            executor,
            pool_size,
            Arc::new(RuntimeStateStore::with_persistence(
                std::path::PathBuf::from(".opencode/runtime/state.json"),
            )),
        )
    }

    pub fn new_with_runtime_state_store(
        bus: Arc<Bus>,
        client: SdkClient,
        session_store: Option<Arc<SessionStore>>,
        executor: Arc<dyn TaskExecutor>,
        pool_size: Option<usize>,
        runtime_state_store: SharedRuntimeStateStore,
    ) -> Self {
        let metrics = Arc::new(MetricsCollector::new(Some("anima")));
        metrics.register_agent_metrics();
        let session_store = session_store.unwrap_or_else(|| Arc::new(SessionStore::new()));
        let worker_timeline = Arc::new(Mutex::new(Vec::<RuntimeTimelineEvent>::new()));
        let bus_for_worker_events = Arc::clone(&bus);
        let worker_timeline_for_events = Arc::clone(&worker_timeline);
        let worker_runtime_event_publisher =
            Arc::new(move |trace_id: &str, event: &str, payload: Value| {
                let message_id = payload
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(trace_id)
                    .to_string();
                let channel = payload
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let chat_id = payload
                    .get("chat_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let sender_id = payload
                    .get("sender_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                {
                    let mut timeline = worker_timeline_for_events.lock();
                    timeline.push(RuntimeTimelineEvent {
                        event: event.to_string(),
                        trace_id: trace_id.to_string(),
                        message_id,
                        channel: channel.clone(),
                        chat_id: chat_id.clone(),
                        sender_id: sender_id.clone(),
                        recorded_at_ms: now_ms(),
                        payload: payload.clone(),
                    });
                    if timeline.len() > 50 {
                        let excess = timeline.len() - 50;
                        timeline.drain(..excess);
                    }
                }
                let _ = bus_for_worker_events.publish_internal(make_internal(MakeInternal {
                    source: "worker-agent".into(),
                    trace_id: Some(trace_id.to_string()),
                    payload: json!({
                        "event": event,
                        "message_id": trace_id,
                        "channel": channel,
                        "chat_id": chat_id,
                        "sender_id": sender_id,
                        "payload": payload,
                    }),
                    ..Default::default()
                }));
            });
        let worker_pool = Arc::new(
            WorkerPool::new(client.clone(), executor.clone(), pool_size, None, None)
                .with_runtime_event_publisher(worker_runtime_event_publisher),
        );
        let specialist_pool = Arc::new(SpecialistPool::new(worker_pool.clone()));
        let orchestrator = Arc::new(AgentOrchestrator::new(
            worker_pool.clone(),
            specialist_pool,
            runtime_state_store.clone(),
            OrchestratorConfig::default(),
        ));
        let emitter = Arc::new(RuntimeEventEmitter::new(
            bus.clone(),
            worker_timeline.clone(),
            runtime_state_store.clone(),
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
            emitter: emitter.clone(),
            runtime_state_store: runtime_state_store.clone(),
            suspension: Arc::new(SuspensionCoordinator::new(
                runtime_state_store.clone(),
                emitter.clone(),
            )),
            requirement: Arc::new(RequirementCoordinator::new(
                runtime_state_store,
                emitter.clone(),
            )),
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

    /// 注册所有内建工具（启用 agentic loop 客户端工具执行）
    ///
    /// 调用方按需决定是否启用；不自动注册，保留 orchestrator 路径作为默认行为。
    pub fn register_builtin_tools(&mut self) {
        let registry = Arc::get_mut(&mut self.tool_registry)
            .expect("cannot mutate shared tool_registry — ensure no other Arc references exist");
        crate::tools::builtins::register_all(registry);
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
            failures: self.emitter.failures_snapshot(),
            runtime_timeline: self.emitter.timeline_snapshot(),
            recent_execution_summaries: self.emitter.execution_summaries_snapshot(),
            tool_count: self.tool_registry.len(),
            pre_hook_count: self
                .hook_registry
                .as_ref()
                .map(|registry| registry.pre_hook_count())
                .unwrap_or(0),
            post_hook_count: self
                .hook_registry
                .as_ref()
                .map(|registry| registry.post_hook_count())
                .unwrap_or(0),
        }
    }

    pub fn runtime_state_snapshot(&self) -> RuntimeStateSnapshot {
        self.runtime_state_store.snapshot()
    }

    pub fn runtime_projection_snapshot(&self) -> RuntimeProjectionView {
        build_projection(&self.runtime_state_store.snapshot())
    }

    #[doc(hidden)]
    pub fn evict_resume_state_cache_for_testing(&self, job_id: &str) {
        self.suspension.evict_cache_for_testing(job_id);
    }

    pub(crate) fn upsert_runtime_run(
        &self,
        inbound_msg: &InboundMessage,
        status: RunStatus,
        current_turn_id: Option<String>,
    ) {
        self.emitter
            .upsert_run(inbound_msg, status, current_turn_id);
    }

    pub(crate) fn upsert_runtime_turn(
        &self,
        inbound_msg: &InboundMessage,
        source: &str,
        status: TurnStatus,
    ) -> String {
        self.emitter.upsert_turn(inbound_msg, source, status)
    }

    pub(crate) fn upsert_runtime_task(
        &self,
        inbound_msg: &InboundMessage,
        phase: RuntimeTaskPhase,
        status: TaskStatus,
        description: impl Into<String>,
        error: Option<String>,
    ) -> String {
        self.emitter
            .upsert_task(inbound_msg, phase, status, description, error)
    }

    pub(crate) fn upsert_runtime_suspension(
        &self,
        inbound_msg: &InboundMessage,
        question: &PendingQuestion,
        task_id: Option<String>,
        invocation_id: Option<String>,
        kind: SuspensionKind,
        status: SuspensionStatus,
    ) -> String {
        self.emitter
            .upsert_suspension(inbound_msg, question, task_id, invocation_id, kind, status)
    }

    pub(crate) fn append_transcript_messages(
        &self,
        inbound_msg: &InboundMessage,
        messages: &[crate::messages::types::InternalMsg],
    ) {
        self.emitter
            .append_transcript_messages(inbound_msg, messages);
    }

    pub(crate) fn prepare_inbound_context(
        &self,
        inbound_msg: &InboundMessage,
    ) -> InboundContextPreparation {
        let key = memory_key(inbound_msg);
        let context_started = now_ms();
        self.emitter.publish(
            "message_received",
            inbound_msg,
            json!({
                "memory_key": key,
                "content_preview": truncate_preview(&inbound_msg.content, 120),
            }),
        );
        self.emitter.record_job_lifecycle_hint(
            &inbound_msg.id,
            "preparing_context",
            preparing_context_current_step(),
        );
        self.ensure_context(inbound_msg, &key);
        let user_entry = json!({"role": "user", "content": inbound_msg.content.clone()});
        self.append_history(&key, user_entry.clone());
        let history_session_id = inbound_msg
            .chat_id
            .clone()
            .unwrap_or_else(|| inbound_msg.id.clone());
        self.append_session_store_history(&history_session_id, user_entry.clone());
        self.context_manager
            .add_to_session_history(&history_session_id, user_entry);
        self.metrics.update_session_gauge(self.memory.lock().len());

        InboundContextPreparation {
            key,
            context_ms: now_ms().saturating_sub(context_started),
        }
    }

    pub(crate) fn prepare_inbound_session(
        &self,
        inbound_msg: &InboundMessage,
        key: &str,
    ) -> Result<SessionPreparation, RuntimeErrorInfo> {
        let session_started = now_ms();
        let opencode_session_id = self
            .get_or_create_opencode_session(inbound_msg, key)
            .map_err(|error| error.to_error_info())?;
        let session_ms = now_ms().saturating_sub(session_started);
        self.emitter.publish(
            "session_ready",
            inbound_msg,
            json!({
                "memory_key": key,
                "opencode_session_id": opencode_session_id.clone(),
            }),
        );
        self.emitter.record_job_lifecycle_hint(
            &inbound_msg.id,
            "planning",
            session_ready_current_step(),
        );

        Ok(SessionPreparation {
            opencode_session_id,
            session_ms,
        })
    }

    pub(crate) fn prepare_inbound_plan(
        &self,
        inbound_msg: &InboundMessage,
        key: &str,
    ) -> PlanPreparation {
        let classify_started = now_ms();
        let plan = AgentClassifier::build_plan(inbound_msg);
        let classify_ms = now_ms().saturating_sub(classify_started);
        self.emitter.publish(
            "plan_built",
            inbound_msg,
            json!({
                "memory_key": key,
                "plan_type": plan.plan_type.clone(),
                "plan_kind": format!("{:?}", plan.kind),
                "task_count": plan.tasks.len(),
                "specialist": plan.specialist.clone(),
            }),
        );
        self.emitter.record_job_lifecycle_hint(
            &inbound_msg.id,
            "planning",
            planning_ready_current_step(&plan.plan_type),
        );

        PlanPreparation { plan, classify_ms }
    }

    /// 处理一条入站消息的完整流程：
    /// 上下文初始化 → 获取/创建 SDK 会话 → 分类 → 缓存检查 → 编排执行 → 发送响应
    pub fn process_inbound_message(&self, inbound_msg: InboundMessage) {
        self.process_inbound_message_flow(inbound_msg);
    }

    /// 确保消息对应的会话上下文存在。
    /// 若内存中会话数达到上限，淘汰最早插入的会话（LRU 策略）。
    pub(crate) fn ensure_context(&self, inbound_msg: &InboundMessage, key: &str) {
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
    pub(crate) fn append_history(&self, key: &str, message: Value) {
        if let Some(ctx) = self.memory.lock().get_mut(key) {
            ctx.history.push(message);
            if ctx.history.len() > MAX_SESSION_HISTORY {
                let excess = ctx.history.len() - MAX_SESSION_HISTORY;
                ctx.history.drain(..excess);
            }
        }
    }

    pub(crate) fn append_session_store_history(&self, session_id: &str, message: Value) {
        let _ = self.session_store.add_to_history(session_id, message);
    }

    pub fn session_context_history(&self, session_id: &str) -> Vec<Value> {
        self.context_manager.get_session_history(session_id)
    }

    pub(crate) fn send_response(
        &self,
        inbound_msg: &InboundMessage,
        response_text: &str,
    ) -> OutboundMessage {
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

    pub(crate) fn assemble_turn_context(
        &self,
        inbound_msg: &InboundMessage,
        mode: ContextAssemblyMode,
        opencode_session_id: String,
        pending_question: Option<PendingQuestion>,
        followup_prompt: Option<String>,
    ) -> ContextAssemblyResult {
        let progress = self
            .requirement
            .ensure(inbound_msg, DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS);
        assemble_context(ContextAssemblyRequest {
            mode,
            inbound_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            original_user_request: progress.original_user_request.clone(),
            opencode_session_id,
            pending_question,
            latest_summary: self.emitter.latest_summary(inbound_msg),
            attempted_rounds: progress.attempted_rounds,
            max_rounds: progress.max_rounds,
            previous_fingerprint: progress.last_result_fingerprint.clone(),
            followup_prompt,
        })
    }

    pub(crate) fn send_error_response(
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

    pub fn pending_question_for(&self, job_id: &str) -> Option<PendingQuestion> {
        self.suspension.question_state(job_id, false).or_else(|| {
            self.runtime_projection_snapshot()
                .pending_questions
                .get(job_id)
                .cloned()
        })
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
        let opencode_client = client.unwrap_or_else(|| {
            SdkClient::with_options("http://127.0.0.1:9711", anima_sdk::ClientOptions::default())
        });
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

    pub fn with_runtime_state_store(
        bus: Arc<Bus>,
        client: Option<SdkClient>,
        session_manager: Option<Arc<SessionStore>>,
        executor: Option<Arc<dyn TaskExecutor>>,
        runtime_state_store: SharedRuntimeStateStore,
    ) -> Self {
        let opencode_client = client.unwrap_or_else(|| {
            SdkClient::with_options("http://127.0.0.1:9711", anima_sdk::ClientOptions::default())
        });
        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionStore::new()));
        let core_agent = Arc::new(CoreAgent::new_with_runtime_state_store(
            bus.clone(),
            opencode_client.clone(),
            Some(session_manager.clone()),
            executor.unwrap_or_else(|| Arc::new(SdkTaskExecutor)),
            None,
            runtime_state_store,
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

    pub fn register_builtin_tools(&mut self) {
        let core = Arc::get_mut(&mut self.core_agent)
            .expect("cannot mutate shared core_agent — ensure no other Arc references exist");
        core.register_builtin_tools();
    }

    pub fn set_hook_registry(&mut self, registry: HookRegistry) {
        let core = Arc::get_mut(&mut self.core_agent)
            .expect("cannot mutate shared core_agent — ensure no other Arc references exist");
        core.set_hook_registry(registry);
    }

    pub fn set_permission_checker(&mut self, checker: PermissionChecker) {
        let core = Arc::get_mut(&mut self.core_agent)
            .expect("cannot mutate shared core_agent — ensure no other Arc references exist");
        core.set_permission_checker(checker);
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

    pub fn runtime_projection_snapshot(&self) -> RuntimeProjectionView {
        self.core_agent.runtime_projection_snapshot()
    }

    #[doc(hidden)]
    pub fn evict_resume_state_cache_for_testing(&self, job_id: &str) {
        self.core_agent.evict_resume_state_cache_for_testing(job_id);
    }
}

/// 生成会话内存的 key，格式为 "channel:chat_id"
pub(crate) fn memory_key(inbound_msg: &InboundMessage) -> String {
    format!(
        "{}:{}",
        inbound_msg.channel,
        inbound_msg
            .chat_id
            .clone()
            .unwrap_or_else(|| inbound_msg.id.clone())
    )
}
