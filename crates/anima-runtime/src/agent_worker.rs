//! # Worker 与 WorkerPool
//!
//! 本模块实现了任务执行的核心工作单元：
//! - `WorkerAgent`：单个工作者，负责接收并执行一个任务（线程安全、支持并发提交）
//! - `WorkerPool`：工作者池，管理多个 WorkerAgent 的生命周期，提供任务分发与负载均衡
//!
//! 设计要点：
//! - 每个 WorkerAgent 同一时刻只处理一个任务（通过 `busy` 原子标志保证）
//! - WorkerPool 使用 Condvar 等待可用 Worker，避免忙轮询
//! - 任务提交后通过 crossbeam channel 异步返回结果

use anima_sdk::facade::Client as SdkClient;
use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

use crate::agent_executor::TaskExecutor;
use crate::agent_types::{make_task_result, MakeTaskResult, Task, TaskResult};
use crate::support::now_ms;

/// Worker 当前正在执行的任务信息
#[derive(Debug, Clone, PartialEq)]
pub struct CurrentTaskInfo {
    pub task_id: String,
    pub trace_id: String,
    pub task_type: String,
    pub started_ms: u64,
    pub content_preview: String,
}

/// 单个工作者智能体，封装了 SDK 客户端和任务执行器。
/// 通过原子标志 `running` / `busy` 实现无锁的状态管理。
pub struct WorkerAgent {
    id: String,
    client: SdkClient,
    executor: Arc<dyn TaskExecutor>,
    running: AtomicBool,
    busy: AtomicBool,
    metrics: Mutex<WorkerMetrics>,
    current_task: Mutex<Option<CurrentTaskInfo>>,
    #[allow(dead_code)] // stored for future task timeout enforcement
    timeout_ms: u64,
}

/// Worker 的运行指标统计
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkerMetrics {
    pub tasks_completed: u64,
    pub timeouts: u64,
    pub errors: u64,
    pub total_duration_ms: u64,
}

/// Worker 的当前状态快照（stopped / busy / idle）
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerStatus {
    pub id: String,
    pub status: String,
    pub metrics: WorkerMetrics,
    pub current_task: Option<CurrentTaskInfo>,
}

impl WorkerAgent {
    /// 创建新的 WorkerAgent，自动生成 UUID 作为唯一标识
    pub fn new(
        client: SdkClient,
        executor: Arc<dyn TaskExecutor>,
        timeout_ms: Option<u64>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            client,
            executor,
            running: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            metrics: Mutex::new(WorkerMetrics::default()),
            current_task: Mutex::new(None),
            timeout_ms: timeout_ms.unwrap_or(60_000),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> WorkerStatus {
        let status = if !self.is_running() {
            "stopped"
        } else if self.is_busy() {
            "busy"
        } else {
            "idle"
        };
        WorkerStatus {
            id: self.id.clone(),
            status: status.to_string(),
            metrics: self.metrics.lock().clone(),
            current_task: self.current_task.lock().clone(),
        }
    }

    /// 提交任务给此 Worker 执行。
    /// 返回一个 Receiver，调用方可通过它异步获取执行结果。
    /// 如果 Worker 未运行或正忙，会立即返回失败结果。
    pub fn submit_task(
        self: &Arc<Self>,
        task: Task,
        notify: Option<Arc<Condvar>>,
    ) -> crossbeam_channel::Receiver<TaskResult> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if !self.is_running() {
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Worker is not running".into()),
                duration_ms: 0,
                worker_id: Some(self.id.clone()),
                result: None,
            }));
            return rx;
        }

        // 用 swap 原子地检查并设置 busy 标志，防止并发提交
        if self.busy.swap(true, Ordering::SeqCst) {
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Worker is busy".into()),
                duration_ms: 0,
                worker_id: Some(self.id.clone()),
                result: None,
            }));
            return rx;
        }

        let worker = Arc::clone(self);
        thread::spawn(move || {
            // Panic guard: ensure busy is cleared even if execute_task panics
            let _guard = PanicGuard {
                busy: &worker.busy,
                notify: notify.as_deref(),
            };

            // 记录当前任务信息
            let content_preview = task
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>();
            *worker.current_task.lock() = Some(CurrentTaskInfo {
                task_id: task.id.clone(),
                trace_id: task.trace_id.clone(),
                task_type: task.task_type.clone(),
                started_ms: now_ms(),
                content_preview,
            });

            let start = now_ms();
            let result = worker.execute_task(&task);
            let duration_ms = now_ms().saturating_sub(start);

            // 清除当前任务
            *worker.current_task.lock() = None;
            {
                let mut metrics = worker.metrics.lock();
                metrics.total_duration_ms += duration_ms;
                if result.status == "success" {
                    metrics.tasks_completed += 1;
                } else if result.status == "timeout" {
                    metrics.timeouts += 1;
                } else {
                    metrics.errors += 1;
                }
            }
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: result.status,
                result: result.result,
                error: result.error,
                duration_ms,
                worker_id: Some(worker.id.clone()),
            }));
            // _guard drops here: clears busy + notifies condvar
        });

        rx
    }

    /// 根据任务类型分发执行：api-call / session-create / transform / query
    fn execute_task(&self, task: &Task) -> ExecuteResult {
        match task.task_type.as_str() {
            // 调用 SDK 发送 prompt 到已有会话
            "api-call" => {
                let session_id = task
                    .payload
                    .get("opencode-session-id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let content = task.payload.get("content").cloned();
                match (session_id, content) {
                    (Some(session_id), Some(content)) => {
                        match self
                            .executor
                            .send_prompt(&self.client, &session_id, content)
                        {
                            Ok(result) => ExecuteResult::success(result),
                            Err(error) => ExecuteResult::failure(error),
                        }
                    }
                    _ => ExecuteResult::failure(
                        "Missing required fields: opencode-session-id or content",
                    ),
                }
            }
            // 创建新的 SDK 会话
            "session-create" => match self.executor.create_session(&self.client) {
                Ok(result) => {
                    if let Some(session_id) = result.get("id").and_then(Value::as_str) {
                        ExecuteResult::success(json!({"opencode-session-id": session_id}))
                    } else {
                        ExecuteResult::failure("Failed to create session: no ID returned")
                    }
                }
                Err(error) => ExecuteResult::failure(error),
            },
            // 数据透传，直接返回 payload 中的 data
            "transform" => {
                if let Some(data) = task.payload.get("data") {
                    ExecuteResult::success(data.clone())
                } else {
                    ExecuteResult::failure("Missing transform data")
                }
            }
            // 按路径查询 context 中的嵌套字段
            "query" => {
                let query = task.payload.get("query").and_then(Value::as_array);
                let context = task.payload.get("context");
                match (query, context) {
                    (Some(path), Some(context)) => {
                        let mut current = context;
                        for segment in path {
                            let Some(key) = segment.as_str() else {
                                return ExecuteResult::failure(
                                    "Query path must contain string keys",
                                );
                            };
                            let Some(next) = current.get(key) else {
                                return ExecuteResult::failure("Query path not found in context");
                            };
                            current = next;
                        }
                        ExecuteResult::success(current.clone())
                    }
                    _ => ExecuteResult::failure("Missing query or context"),
                }
            }
            other => ExecuteResult::failure(format!("Unknown task type: {other}")),
        }
    }
}

/// 任务执行的内部结果，仅在 WorkerAgent 内部使用
struct ExecuteResult {
    status: String,
    result: Option<Value>,
    error: Option<String>,
}

/// Panic 安全守卫：无论任务执行是否 panic，都会在 drop 时清除 busy 标志并唤醒等待者。
/// Guard that clears the busy flag and notifies the condvar on drop (including panics).
struct PanicGuard<'a> {
    busy: &'a AtomicBool,
    notify: Option<&'a Condvar>,
}

impl Drop for PanicGuard<'_> {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::SeqCst);
        if let Some(cv) = self.notify {
            cv.notify_one();
        }
    }
}

impl ExecuteResult {
    fn success(result: Value) -> Self {
        Self {
            status: "success".into(),
            result: Some(result),
            error: None,
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            status: "failure".into(),
            result: None,
            error: Some(error.into()),
        }
    }
}

/// 工作者池，管理一组 WorkerAgent 的生命周期。
/// 使用 round-robin + Condvar 实现负载均衡和等待机制。
pub struct WorkerPool {
    workers: Mutex<Vec<Arc<WorkerAgent>>>,
    worker_available: Arc<Condvar>,
    client: SdkClient,
    executor: Arc<dyn TaskExecutor>,
    worker_timeout_ms: Option<u64>,
    next_index: AtomicU64,
    max_wait_ms: u64,
    running: AtomicBool,
    min_size: usize,
    max_size: usize,
}

/// WorkerPool 的状态快照
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerPoolStatus {
    pub status: String,
    pub size: usize,
    pub workers: Vec<WorkerStatus>,
}

impl WorkerPool {
    /// 创建工作者池，初始化 `initial_size` 个 Worker（默认 2，最少 1）
    pub fn new(
        client: SdkClient,
        executor: Arc<dyn TaskExecutor>,
        initial_size: Option<usize>,
        timeout_ms: Option<u64>,
        max_wait_ms: Option<u64>,
    ) -> Self {
        let size = initial_size.unwrap_or(2).max(1);
        let mut workers = Vec::with_capacity(size);
        for _ in 0..size {
            workers.push(Arc::new(WorkerAgent::new(
                client.clone(),
                executor.clone(),
                timeout_ms,
            )));
        }
        Self {
            workers: Mutex::new(workers),
            worker_available: Arc::new(Condvar::new()),
            client,
            executor,
            worker_timeout_ms: timeout_ms,
            next_index: AtomicU64::new(0),
            max_wait_ms: max_wait_ms.unwrap_or(5_000),
            running: AtomicBool::new(false),
            min_size: 1,
            max_size: 16,
        }
    }

    /// 设置动态伸缩的最小/最大 Worker 数量
    /// Set min/max bounds for dynamic scaling.
    pub fn with_bounds(mut self, min_size: usize, max_size: usize) -> Self {
        self.min_size = min_size.max(1);
        self.max_size = max_size.max(self.min_size);
        self
    }

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
        let workers = self.workers.lock();
        for worker in workers.iter() {
            worker.start();
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let workers = self.workers.lock();
        for worker in workers.iter() {
            worker.stop();
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn size(&self) -> usize {
        self.workers.lock().len()
    }

    pub fn status(&self) -> WorkerPoolStatus {
        let workers = self.workers.lock();
        WorkerPoolStatus {
            status: if self.is_running() {
                "running"
            } else {
                "stopped"
            }
            .to_string(),
            size: workers.len(),
            workers: workers.iter().map(|worker| worker.status()).collect(),
        }
    }

    /// 动态调整池大小到 target（受 min/max 约束），返回调整后的实际大小
    /// Dynamically scale the pool to `target` workers (clamped to min/max bounds).
    pub fn scale_to(&self, target: usize) -> usize {
        let target = target.clamp(self.min_size, self.max_size);
        let mut workers = self.workers.lock();
        let current = workers.len();

        if target > current {
            // Scale up
            for _ in current..target {
                let worker = Arc::new(WorkerAgent::new(
                    self.client.clone(),
                    self.executor.clone(),
                    self.worker_timeout_ms,
                ));
                if self.is_running() {
                    worker.start();
                }
                workers.push(worker);
            }
        } else if target < current {
            // Scale down — stop and remove excess workers from the end
            for _ in target..current {
                if let Some(worker) = workers.pop() {
                    worker.stop();
                }
            }
        }
        workers.len()
    }

    /// 向池中提交任务。会等待可用 Worker（最多 max_wait_ms），超时则返回失败。
    /// 内部使用 Condvar 避免忙轮询，Worker 完成任务后会唤醒等待者。
    pub fn submit_task(&self, task: Task) -> crossbeam_channel::Receiver<TaskResult> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if !self.is_running() {
            let _ = tx.send(make_task_result(MakeTaskResult {
                task_id: task.id,
                trace_id: task.trace_id,
                status: "failure".into(),
                error: Some("Worker pool is not running".into()),
                duration_ms: 0,
                worker_id: None,
                result: None,
            }));
            return rx;
        }

        let started = now_ms();
        let remaining_ms = self.max_wait_ms;

        // Use condvar to wait for an available worker instead of spinning
        let mut dummy = self.workers.lock();
        loop {
            // Drop the lock before scanning workers (next_available_worker takes its own lock)
            drop(dummy);

            if let Some(worker) = self.next_available_worker() {
                return worker.submit_task(task, Some(Arc::clone(&self.worker_available)));
            }

            let elapsed = now_ms().saturating_sub(started);
            if elapsed >= remaining_ms {
                let _ = tx.send(make_task_result(MakeTaskResult {
                    task_id: task.id,
                    trace_id: task.trace_id,
                    status: "failure".into(),
                    error: Some("No available worker".into()),
                    duration_ms: elapsed,
                    worker_id: None,
                    result: None,
                }));
                return rx;
            }

            let wait_ms = (remaining_ms - elapsed).min(50);
            dummy = self.workers.lock();
            self.worker_available
                .wait_for(&mut dummy, Duration::from_millis(wait_ms));
        }
    }

    /// 使用 round-robin 策略查找下一个空闲 Worker
    fn next_available_worker(&self) -> Option<Arc<WorkerAgent>> {
        let workers = self.workers.lock();
        if workers.is_empty() {
            return None;
        }
        let len = workers.len() as u64;
        let start = self.next_index.fetch_add(1, Ordering::SeqCst);
        for offset in 0..len {
            let idx = ((start + offset) % len) as usize;
            let worker = &workers[idx];
            if worker.is_running() && !worker.is_busy() {
                return Some(Arc::clone(worker));
            }
        }
        None
    }
}
