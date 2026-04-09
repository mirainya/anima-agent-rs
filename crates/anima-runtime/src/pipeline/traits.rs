//! 数据管道 trait 定义模块
//!
//! 定义了数据管道各阶段的核心抽象（对齐 Clojure 版本的 ISource/ITransform/IFilter/IAggregate/ISink）。
//! 数据流经管道的顺序为：Source → Filter → Transform → Aggregate/Sink。
//! 同时定义了管道执行上下文 `PipelineContext`，携带执行 ID 和元数据。

use crate::support::now_ms;
use serde_json::Value;
use uuid::Uuid;

// ── Pipeline Traits (对齐 Clojure ISource/ITransform/IFilter/IAggregate/ISink) ──

/// 数据源 trait —— 向管道发射数据项
pub trait Source: Send + Sync {
    fn start(&self, ctx: &PipelineContext) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn status(&self) -> SourceStatus;
}

/// 数据转换器 trait —— 对每个数据项进行映射变换，返回 None 表示丢弃
pub trait Transform: Send + Sync {
    fn transform(&self, data: Value, ctx: &PipelineContext) -> Result<Option<Value>, String>;
}

/// 数据过滤器 trait —— 返回 true 保留，false 丢弃
pub trait Filter: Send + Sync {
    fn passes(&self, data: &Value, ctx: &PipelineContext) -> bool;
}

/// 数据聚合器 trait —— 累积多个数据项，最终输出合并结果
pub trait Aggregate: Send + Sync {
    fn add_data(&self, data: Value, ctx: &PipelineContext) -> Result<(), String>;
    fn get_result(&self) -> Option<Value>;
    fn reset(&self);
}

/// 数据输出端 trait —— 将处理后的数据写出到外部系统
pub trait Sink: Send + Sync {
    fn write(&self, data: Value, ctx: &PipelineContext) -> Result<(), String>;
    fn flush(&self) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
}

// ── Status Types ────────────────────────────────────────────────────

/// 管道阶段的运行状态
#[derive(Debug, Clone, PartialEq)]
pub enum StageState {
    Idle,
    Running,
    Stopped,
    Error(String),
}

/// 数据源的运行状态信息
#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub id: String,
    pub state: StageState,
    pub items_emitted: u64,
}

// ── Pipeline Context ────────────────────────────────────────────────

/// 管道执行上下文，每次 process 调用生成唯一的 execution_id
#[derive(Debug, Clone)]
pub struct PipelineContext {
    pub pipeline_id: String,
    pub execution_id: String,
    pub metadata: Value,
    pub start_time_ms: u64,
}

impl PipelineContext {
    pub fn new(pipeline_id: &str) -> Self {
        Self {
            pipeline_id: pipeline_id.to_string(),
            execution_id: Uuid::new_v4().to_string(),
            metadata: serde_json::json!({}),
            start_time_ms: now_ms(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        now_ms().saturating_sub(self.start_time_ms)
    }
}
