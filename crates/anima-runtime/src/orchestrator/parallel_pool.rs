//! 并行任务池模块
//!
//! 提供批量任务的并行执行能力。核心概念：
//! - 将一组任务按 `max_concurrent` 分块并发提交给 `WorkerPool`
//! - 支持 fail-fast（任一失败立即中止）和最低成功率阈值两种提前终止策略
//! - 同时保留了向后兼容的简单 `execute` 接口

use crate::agent::types::{make_task_result, MakeTaskResult, Task, TaskResult};
use crate::agent::worker::WorkerPool;
use crate::support::now_ms;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ── Config ─────────────────────────────────────────────────────────

/// 并行池配置：控制并发度、超时、失败策略
#[derive(Debug, Clone)]
pub struct ParallelPoolConfig {
    pub max_concurrent: usize,
    pub default_timeout_ms: u64,
    pub fail_fast: bool,
    pub min_success_ratio: f64,
}

impl Default for ParallelPoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            default_timeout_ms: 60_000,
            fail_fast: false,
            min_success_ratio: 0.5,
        }
    }
}

// ── Parallel Task / Result ─────────────────────────────────────────

/// 单个并行任务的包装，附带独立超时配置
#[derive(Debug, Clone)]
pub struct ParallelTask {
    pub id: String,
    pub task: Task,
    pub timeout_ms: Option<u64>,
}

/// 批量执行的汇总结果，包含成功/失败/超时计数和成功率
#[derive(Debug, Clone)]
pub struct ParallelResult {
    pub batch_id: String,
    pub results: Vec<TaskResult>,
    pub successful: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub duration_ms: u64,
    pub success_ratio: f64,
}

// ── Metrics ────────────────────────────────────────────────────────

/// 并行池累计指标
#[derive(Debug, Clone, Default)]
pub struct ParallelPoolMetrics {
    pub batches_executed: u64,
    pub total_tasks: u64,
    pub successful_tasks: u64,
    pub failed_tasks: u64,
    pub timed_out_tasks: u64,
}

// ── Pool ───────────────────────────────────────────────────────────

/// 并行任务池：将任务分块提交给 WorkerPool 并收集结果
pub struct ParallelPool {
    worker_pool: Arc<WorkerPool>,
    running: AtomicBool,
    config: ParallelPoolConfig,
    metrics: Mutex<ParallelPoolMetrics>,
}

impl ParallelPool {
    pub fn new(worker_pool: Arc<WorkerPool>) -> Self {
        Self {
            worker_pool,
            running: AtomicBool::new(false),
            config: ParallelPoolConfig::default(),
            metrics: Mutex::new(ParallelPoolMetrics::default()),
        }
    }

    pub fn with_config(worker_pool: Arc<WorkerPool>, config: ParallelPoolConfig) -> Self {
        Self {
            worker_pool,
            running: AtomicBool::new(false),
            config,
            metrics: Mutex::new(ParallelPoolMetrics::default()),
        }
    }

    // ── Existing API (backward compatible) ─────────────────────────

    /// 向后兼容接口：批量提交所有任务，按输入顺序收集结果，任一失败则直接返回失败结果
    pub fn execute(&self, tasks: Vec<Task>) -> TaskResult {
        let receivers = tasks
            .into_iter()
            .map(|task| {
                let rx = self.worker_pool.submit_task(task.clone());
                (task, rx)
            })
            .collect::<Vec<_>>();

        let results = receivers
            .into_iter()
            .map(|(task, rx)| {
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
            })
            .collect::<Vec<_>>();

        if let Some(failed) = results.iter().find(|result| result.status != "success") {
            return failed.clone();
        }

        make_task_result(MakeTaskResult {
            task_id: Uuid::new_v4().to_string(),
            trace_id: results
                .first()
                .map(|result| result.trace_id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            status: "success".into(),
            result: Some(json!({
                "results": results
                    .iter()
                    .map(|result| result.result.clone().unwrap_or(Value::Null))
                    .collect::<Vec<_>>()
            })),
            error: None,
            duration_ms: results.iter().map(|result| result.duration_ms).sum(),
            worker_id: None,
        })
    }

    // ── New batch API ──────────────────────────────────────────────

    /// 批量执行任务，支持并发分块、超时、fail-fast 和最低成功率阈值
    ///
    /// 任务按 `max_concurrent` 分块提交，每块内并发执行。
    /// 两种提前终止条件：fail-fast 模式下任一失败立即中止；
    /// 或当剩余任务全部成功也无法达到 `min_success_ratio` 时提前退出。
    pub fn execute_batch(
        &self,
        tasks: Vec<Task>,
        opts: Option<ParallelPoolConfig>,
    ) -> ParallelResult {
        let config = opts.unwrap_or_else(|| self.config.clone());
        let started = now_ms();
        let batch_id = Uuid::new_v4().to_string();

        let mut results: Vec<TaskResult> = Vec::with_capacity(tasks.len());
        let mut successful = 0usize;
        let mut failed = 0usize;
        let mut timed_out = 0usize;

        // Process in chunks of max_concurrent
        for chunk in tasks.chunks(config.max_concurrent) {
            let receivers: Vec<_> = chunk
                .iter()
                .map(|task| {
                    let rx = self.worker_pool.submit_task(task.clone());
                    (task.clone(), rx)
                })
                .collect();

            for (task, rx) in receivers {
                let timeout = std::time::Duration::from_millis(config.default_timeout_ms);
                let result = match rx.recv_timeout(timeout) {
                    Ok(r) => r,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        timed_out += 1;
                        make_task_result(MakeTaskResult {
                            task_id: task.id,
                            trace_id: task.trace_id,
                            status: "failure".into(),
                            error: Some("Task timed out".into()),
                            duration_ms: config.default_timeout_ms,
                            worker_id: None,
                            result: None,
                        })
                    }
                    Err(_) => {
                        make_task_result(MakeTaskResult {
                            task_id: task.id,
                            trace_id: task.trace_id,
                            status: "failure".into(),
                            error: Some("Task result channel closed".into()),
                            duration_ms: 0,
                            worker_id: None,
                            result: None,
                        })
                    }
                };

                if result.status == "success" {
                    successful += 1;
                } else if !result.error.as_deref().is_some_and(|e| e.contains("timed out")) {
                    // 超时任务已在上面的 match 分支中计数，这里只统计非超时的失败
                    failed += 1;
                }

                results.push(result);

                // Fail-fast: abort if any failure
                if config.fail_fast && (failed + timed_out) > 0 {
                    break;
                }
            }

            // 乐观估算：即使剩余任务全部成功，成功率仍低于阈值则提前终止
            let total_so_far = results.len();
            if total_so_far > 0 {
                let _ratio = successful as f64 / total_so_far as f64;
                let remaining = tasks.len() - total_so_far;
                let best_possible = (successful + remaining) as f64 / tasks.len() as f64;
                if best_possible < config.min_success_ratio {
                    break;
                }
            }

            if config.fail_fast && (failed + timed_out) > 0 {
                break;
            }
        }

        let duration_ms = now_ms().saturating_sub(started);
        let total = results.len();
        let success_ratio = if total > 0 {
            successful as f64 / total as f64
        } else {
            0.0
        };

        // Update metrics
        if let Ok(mut m) = self.metrics.lock() {
            m.batches_executed += 1;
            m.total_tasks += total as u64;
            m.successful_tasks += successful as u64;
            m.failed_tasks += failed as u64;
            m.timed_out_tasks += timed_out as u64;
        }

        ParallelResult {
            batch_id,
            results,
            successful,
            failed,
            timed_out,
            duration_ms,
            success_ratio,
        }
    }

    // ── Lifecycle ──────────────────────────────────────────────────

    pub fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    // ── Status / Metrics ───────────────────────────────────────────

    pub fn status(&self) -> Value {
        let m = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "running": self.is_running(),
            "config": {
                "max_concurrent": self.config.max_concurrent,
                "default_timeout_ms": self.config.default_timeout_ms,
                "fail_fast": self.config.fail_fast,
                "min_success_ratio": self.config.min_success_ratio,
            },
            "metrics": {
                "batches_executed": m.batches_executed,
                "total_tasks": m.total_tasks,
                "successful_tasks": m.successful_tasks,
                "failed_tasks": m.failed_tasks,
                "timed_out_tasks": m.timed_out_tasks,
            }
        })
    }

    pub fn metrics(&self) -> ParallelPoolMetrics {
        self.metrics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
