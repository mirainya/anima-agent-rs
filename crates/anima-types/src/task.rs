//! 任务核心类型定义
//!
//! 定义任务（Task）、任务结果（TaskResult）和执行计划（ExecutionPlan）等核心数据结构。

use serde_json::{json, Value};
use std::collections::VecDeque;
use uuid::Uuid;

/// 任务：Agent 运行时的最小调度单元
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub trace_id: String,
    pub task_type: String,
    pub payload: Value,
    pub priority: u8,
    pub timeout_ms: u64,
    pub metadata: Value,
}

/// 创建任务的便捷函数，自动填充默认值（ID、trace_id、优先级等）
pub fn make_task(input: MakeTask) -> Task {
    Task {
        id: Uuid::new_v4().to_string(),
        trace_id: input.trace_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        task_type: input.task_type,
        payload: input.payload.unwrap_or_else(|| json!({})),
        priority: input.priority.unwrap_or(5),
        timeout_ms: input.timeout_ms.unwrap_or(60_000),
        metadata: input.metadata.unwrap_or_else(|| json!({})),
    }
}

/// 创建任务时的可选参数，未提供的字段会使用默认值
#[derive(Debug, Default)]
pub struct MakeTask {
    pub trace_id: Option<String>,
    pub task_type: String,
    pub payload: Option<Value>,
    pub priority: Option<u8>,
    pub timeout_ms: Option<u64>,
    pub metadata: Option<Value>,
}

/// 任务执行结果，包含状态、返回值、耗时等信息
#[derive(Debug, Clone, PartialEq)]
pub struct TaskResult {
    pub task_id: String,
    pub trace_id: String,
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub worker_id: Option<String>,
}

/// 创建任务结果的便捷函数
pub fn make_task_result(input: MakeTaskResult) -> TaskResult {
    TaskResult {
        task_id: input.task_id,
        trace_id: input.trace_id,
        status: input.status,
        result: input.result,
        error: input.error,
        duration_ms: input.duration_ms,
        worker_id: input.worker_id,
    }
}

/// 创建任务结果时的参数
#[derive(Debug, Default)]
pub struct MakeTaskResult {
    pub task_id: String,
    pub trace_id: String,
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub worker_id: Option<String>,
}

/// 执行计划类型，决定任务如何被调度执行
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionPlanKind {
    /// 直接执行，不经过调度
    Direct,
    /// 单任务执行
    Single,
    /// 顺序执行多个任务
    Sequential,
    /// 并行执行多个任务
    Parallel,
    /// 路由到专家 Agent 执行
    SpecialistRoute,
}

/// 执行计划，描述一组任务的执行方式和目标
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    pub kind: ExecutionPlanKind,
    pub plan_type: String,
    /// 待执行的任务队列（使用 VecDeque 支持高效的头部弹出）
    pub tasks: VecDeque<Task>,
    /// 指定的专家 Agent 名称（仅 SpecialistRoute 时使用）
    pub specialist: Option<String>,
}
