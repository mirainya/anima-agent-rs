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
use uuid::Uuid;

pub use crate::agent_executor::{SdkTaskExecutor, TaskExecutor};
pub use crate::agent_types::{
    make_task, make_task_result, ExecutionPlan, MakeTask, MakeTaskResult, Task, TaskResult,
};
pub use crate::agent_worker::{CurrentTaskInfo, WorkerAgent, WorkerMetrics, WorkerPool, WorkerPoolStatus, WorkerStatus};
use crate::agent_classifier::AgentClassifier;
use crate::agent_orchestrator::AgentOrchestrator;
use crate::bus::{make_outbound, make_internal, Bus, InboundMessage, MakeInternal, MakeOutbound, OutboundMessage};
use crate::bus::{ControlSignal};
use crate::channel::SessionStore;
use crate::support::{make_api_cache_key, now_ms, ContextManager, LruCache, MetricsCollector};

#[derive(Debug, Clone)]
struct RuntimeErrorInfo {
    code: &'static str,
    stage: &'static str,
    user_message: String,
    internal_message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeFailureSnapshot {
    pub error_code: String,
    pub error_stage: String,
    pub message_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub occurred_at_ms: u64,
    pub internal_message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeFailureStatus {
    pub last_failure: Option<RuntimeFailureSnapshot>,
    pub counts_by_error_code: indexmap::IndexMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeTimelineEvent {
    pub event: String,
    pub trace_id: String,
    pub message_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub sender_id: String,
    pub recorded_at_ms: u64,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionStageDurations {
    pub context_ms: u64,
    pub session_ms: u64,
    pub classify_ms: u64,
    pub execute_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionSummary {
    pub trace_id: String,
    pub message_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub plan_type: String,
    pub status: String,
    pub cache_hit: bool,
    pub worker_id: Option<String>,
    pub error_code: Option<String>,
    pub error_stage: Option<String>,
    pub task_duration_ms: u64,
    pub stages: ExecutionStageDurations,
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

/// 核心智能体，承载消息循环、会话管理、任务调度等核心逻辑。
/// 通过 Bus 接收入站消息，经分类和编排后交由 WorkerPool 执行。
pub struct CoreAgent {
    bus: Arc<Bus>,
    _client: SdkClient,
    session_store: Arc<SessionStore>,
    worker_pool: Arc<WorkerPool>,
    context_manager: Arc<ContextManager>,
    result_cache: Arc<LruCache>,
    metrics: Arc<MetricsCollector>,
    memory: Mutex<indexmap::IndexMap<String, SessionContext>>,
    failures: Mutex<RuntimeFailureStatus>,
    timeline: Mutex<Vec<RuntimeTimelineEvent>>,
    execution_summaries: Mutex<Vec<ExecutionSummary>>,
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
        Self {
            bus,
            _client: client.clone(),
            session_store: session_store.unwrap_or_else(|| Arc::new(SessionStore::new())),
            worker_pool: Arc::new(WorkerPool::new(client, executor, pool_size, None, None)),
            context_manager: Arc::new(ContextManager::new(Some(true))),
            result_cache: Arc::new(LruCache::new(Some(1000), Some(5 * 60 * 1000))),
            metrics,
            memory: Mutex::new(indexmap::IndexMap::new()),
            failures: Mutex::new(RuntimeFailureStatus::default()),
            timeline: Mutex::new(Vec::new()),
            execution_summaries: Mutex::new(Vec::new()),
            running: AtomicBool::new(false),
            loop_handle: Mutex::new(None),
            control_handle: Mutex::new(None),
        }
    }

    /// 启动智能体：启动 WorkerPool，并创建两个后台线程：
    /// 1. 控制信号监听线程（处理 Shutdown / Pause / Resume）
    /// 2. 入站消息循环线程（从 Bus 接收并处理消息）
    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        self.worker_pool.start();

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
        let plan_type = plan.plan_type.clone();
        let execute_started = now_ms();
        let mut cache_hit = false;
        let result = if plan_type == "single" {
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
                self.metrics.counter_inc("tasks_submitted");
                self.publish_worker_event("task_start", &inbound_msg, &plan_type);
                let result = AgentOrchestrator::execute_plan(&self.worker_pool, &plan, &opencode_session_id);
                self.publish_worker_event("task_end", &inbound_msg, &plan_type);
                if result.status == "success" {
                    self.metrics.counter_inc("tasks_completed");
                    if let Some(value) = result.result.clone() {
                        self.result_cache.set(&cache_key, value, None);
                    }
                } else {
                    self.metrics.counter_inc("tasks_failed");
                }
                result
            }
        } else {
            self.metrics.counter_inc("tasks_submitted");
            self.publish_worker_event("task_start", &inbound_msg, &plan_type);
            let result = AgentOrchestrator::execute_plan(&self.worker_pool, &plan, &opencode_session_id);
            self.publish_worker_event("task_end", &inbound_msg, &plan_type);
            if result.status == "success" {
                self.metrics.counter_inc("tasks_completed");
            } else {
                self.metrics.counter_inc("tasks_failed");
            }
            result
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
            let response_text = extract_response_text(result.result.as_ref());
            let assistant_entry = json!({"role": "assistant", "content": response_text.clone()});
            self.append_history(&key, assistant_entry.clone());
            self.context_manager
                .add_to_session_history(&history_session_id, assistant_entry);
            self.metrics.counter_inc("messages_processed");
            self.record_execution_summary(ExecutionSummary {
                trace_id: inbound_msg.id.clone(),
                message_id: inbound_msg.id.clone(),
                channel: inbound_msg.channel.clone(),
                chat_id: inbound_msg.chat_id.clone(),
                plan_type: plan_type.clone(),
                status: result.status.clone(),
                cache_hit,
                worker_id: result.worker_id.clone(),
                error_code: None,
                error_stage: None,
                task_duration_ms: result.duration_ms,
                stages: ExecutionStageDurations {
                    context_ms,
                    session_ms,
                    classify_ms,
                    execute_ms,
                    total_ms,
                },
            });
            self.publish_runtime_event("message_completed", &inbound_msg, json!({
                "memory_key": key,
                "plan_type": plan_type.clone(),
                "status": result.status.clone(),
                "cached": cache_hit,
                "worker_id": result.worker_id.clone(),
                "task_duration_ms": result.duration_ms,
                "message_latency_ms": total_ms,
                "response_preview": truncate_preview(&response_text, 120),
                "response_text": response_text,
            }));
            self.send_response(&inbound_msg, &response_text);
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

        let result = self
            .worker_pool
            .submit_task(make_task(MakeTask {
                trace_id: Some(inbound_msg.id.clone()),
                task_type: "session-create".into(),
                payload: Some(json!({})),
                ..Default::default()
            }))
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
            message_id: inbound_msg.id.clone(),
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
        self.record_timeline_event(event, inbound_msg, payload.clone());
        let _ = self.bus.publish_internal(make_internal(MakeInternal {
            source: "core-agent".into(),
            trace_id: Some(inbound_msg.id.clone()),
            payload: json!({
                "event": event,
                "message_id": inbound_msg.id,
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

fn extract_response_text(response: Option<&Value>) -> String {
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
