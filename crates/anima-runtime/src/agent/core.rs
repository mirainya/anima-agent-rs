//! CoreAgent 核心定义：struct、构造函数、消息处理方法

use anima_types::approval::ApprovalMode;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;

use super::context_types::{
    memory_key, InboundContextPreparation, PlanPreparation, RuntimeErrorInfo, RuntimeTaskPhase,
    SessionPreparation,
};
use super::event_emitter::RuntimeEventEmitter;
use crate::worker::executor::TaskExecutor;
use crate::provider::{Provider, OpenCodeProvider};
use super::requirement::RequirementCoordinator;
use super::runtime_helpers::truncate_preview;
use crate::worker::{WorkerPool, WorkerPoolStatus};
use crate::bus::{
    make_internal, make_outbound, Bus, InboundMessage, MakeInternal, MakeOutbound, OutboundMessage,
};
use crate::channel::SessionStore;
use crate::classifier::rule::AgentClassifier;
use crate::execution::context_assembly::{
    assemble_context, ContextAssemblyMode, ContextAssemblyRequest, ContextAssemblyResult,
};
use crate::hooks::HookRegistry;
use crate::orchestrator::core::{AgentOrchestrator, OrchestratorConfig};
use crate::orchestrator::specialist_pool::SpecialistPool;
use crate::permissions::PermissionChecker;
use crate::runtime::{
    RuntimeStateStore, SharedRuntimeStateStore,
};
use crate::runtime::{
    planning_ready_current_step, preparing_context_current_step, session_ready_current_step,
};
use crate::support::{now_ms, ContextManager, LruCache, MetricsCollector};
use crate::tasks::{RunStatus, SuspensionKind, SuspensionStatus, TaskStatus, TurnStatus};
use crate::tools::registry::ToolRegistry;

pub use anima_types::event::{
    ExecutionStageDurations, ExecutionSummary, RuntimeFailureSnapshot, RuntimeFailureStatus,
    RuntimeTimelineEvent,
};

pub use super::suspension::{
    PendingQuestion, PendingQuestionSourceKind, QuestionAnswerInput, QuestionDecisionMode,
    QuestionKind, QuestionRiskLevel,
};
use super::suspension::SuspensionCoordinator;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionContext {
    pub session_id: Option<String>,
    pub chat_id: String,
    pub channel: String,
    pub history: Vec<Value>,
}

const MAX_SESSIONS: usize = 1000;
const MAX_SESSION_HISTORY: usize = 200;

pub struct CoreAgent {
    pub(crate) bus: Arc<Bus>,
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
    /// Agentic loop 用：Provider 抽象（包装 executor）
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) judge_provider: Mutex<Option<Arc<dyn Provider>>>,
    pub(crate) memory: Mutex<indexmap::IndexMap<String, SessionContext>>,
    pub(crate) emitter: Arc<RuntimeEventEmitter>,
    pub(crate) runtime_state_store: SharedRuntimeStateStore,
    /// 挂起状态协调器
    pub(crate) suspension: Arc<SuspensionCoordinator>,
    pub(crate) requirement: Arc<RequirementCoordinator>,
    pub(crate) approval_mode: Arc<Mutex<ApprovalMode>>,
    pub(crate) auto_approve_tools: Arc<AtomicBool>,
    pub(crate) prompts: parking_lot::RwLock<Arc<anima_types::config::PromptsConfig>>,
    pub(crate) running: AtomicBool,
    pub(crate) loop_handle: Mutex<Option<thread::JoinHandle<()>>>,
    pub(crate) control_handle: Mutex<Option<thread::JoinHandle<()>>>,
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
        session_store: Option<Arc<SessionStore>>,
        executor: Arc<dyn TaskExecutor>,
        pool_size: Option<usize>,
    ) -> Self {
        Self::new_with_runtime_state_store(
            bus,
            session_store,
            executor,
            pool_size,
            Arc::new(RuntimeStateStore::new()),
        )
    }

    pub fn new_with_runtime_state_store(
        bus: Arc<Bus>,
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
                    payload: crate::bus::InternalPayload::RuntimeEvent {
                        event: event.to_string(),
                        message_id: trace_id.to_string(),
                        channel: channel.clone(),
                        chat_id: chat_id.clone(),
                        sender_id: sender_id.clone(),
                        payload,
                    },
                    ..Default::default()
                }));
            });
        let worker_pool = Arc::new(
            WorkerPool::new(executor.clone(), pool_size, None, None)
                .with_runtime_event_publisher(worker_runtime_event_publisher),
        );
        let specialist_pool = Arc::new(SpecialistPool::new(worker_pool.clone()));
        let provider: Arc<dyn Provider> = Arc::new(OpenCodeProvider::new(executor.clone()));
        let orchestrator = Arc::new(AgentOrchestrator::new(
            worker_pool.clone(),
            specialist_pool,
            runtime_state_store.clone(),
            OrchestratorConfig::default(),
        ).with_llm(provider.clone()));
        let emitter = Arc::new(RuntimeEventEmitter::new(
            bus.clone(),
            worker_timeline.clone(),
            runtime_state_store.clone(),
        ));
        Self {
            bus,
            session_store,
            worker_pool,
            orchestrator,
            context_manager: Arc::new(ContextManager::new(Some(true))),
            result_cache: Arc::new(LruCache::new(Some(1000), Some(5 * 60 * 1000))),
            metrics,
            tool_registry: Arc::new(ToolRegistry::new()),
            permission_checker: None,
            hook_registry: None,
            provider: provider.clone(),
            judge_provider: Mutex::new(None),
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
            approval_mode: Arc::new(Mutex::new(ApprovalMode::default())),
            auto_approve_tools: Arc::new(AtomicBool::new(false)),
            prompts: parking_lot::RwLock::new(Arc::new(anima_types::config::PromptsConfig::default())),
            running: AtomicBool::new(false),
            loop_handle: Mutex::new(None),
            control_handle: Mutex::new(None),
        }
    }

    pub(crate) fn enable_llm_judge(&self) {
        *self.judge_provider.lock() = Some(Arc::clone(&self.provider));
    }

    pub fn set_provider(&mut self, provider: Arc<dyn Provider>) {
        self.provider = provider.clone();
        self.orchestrator.set_provider(provider);
    }

    pub fn set_prompts(&self, prompts: anima_types::config::PromptsConfig) {
        let arc = Arc::new(prompts);
        self.orchestrator.set_prompts(Arc::clone(&arc));
        *self.prompts.write() = arc;
    }

    pub fn prompts(&self) -> Arc<anima_types::config::PromptsConfig> {
        Arc::clone(&self.prompts.read())
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
        let user_internal = crate::messages::types::InternalMsg {
            role: crate::messages::types::MessageRole::User,
            blocks: vec![crate::messages::types::ContentBlock::Text {
                text: inbound_msg.content.clone(),
            }],
            message_id: uuid::Uuid::new_v4().to_string(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        };
        self.append_transcript_messages(inbound_msg, &[user_internal]);
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
        let opencode_session_id = if !self.tool_registry.is_empty() {
            // Direct provider mode: skip SDK session, use local session ID
            let local_id = inbound_msg.session_key.clone()
                .unwrap_or_else(|| inbound_msg.id.clone());
            self.store_opencode_session_id(key, &local_id);
            local_id
        } else {
            self.get_or_create_opencode_session(inbound_msg, key)
                .map_err(|error| error.to_error_info())?
        };
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
            .ensure(inbound_msg, super::context_types::DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS);
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
