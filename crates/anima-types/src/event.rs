//! 运行时事件类型定义
//!
//! 定义时间线事件、执行摘要、失败快照等可观测性数据结构。

use indexmap::IndexMap;
use serde_json::Value;

/// 运行时失败快照
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

/// 运行时失败状态汇总
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeFailureStatus {
    pub last_failure: Option<RuntimeFailureSnapshot>,
    pub counts_by_error_code: IndexMap<String, u64>,
}

/// 运行时时间线事件
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

/// 执行各阶段耗时
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionStageDurations {
    pub context_ms: u64,
    pub session_ms: u64,
    pub classify_ms: u64,
    pub execute_ms: u64,
    pub total_ms: u64,
}

/// 执行摘要
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
