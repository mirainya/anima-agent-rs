use crate::support::now_ms;
use serde_json::Value;
use uuid::Uuid;

// ── Pipeline Traits (对齐 Clojure ISource/ITransform/IFilter/IAggregate/ISink) ──

/// Data source — emits items into the pipeline.
pub trait Source: Send + Sync {
    fn start(&self, ctx: &PipelineContext) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn status(&self) -> SourceStatus;
}

/// Data transformer — maps each item.
pub trait Transform: Send + Sync {
    fn transform(&self, data: Value, ctx: &PipelineContext) -> Result<Option<Value>, String>;
}

/// Data filter — returns true to keep, false to drop.
pub trait Filter: Send + Sync {
    fn passes(&self, data: &Value, ctx: &PipelineContext) -> bool;
}

/// Data aggregator — accumulates items, emits combined result.
pub trait Aggregate: Send + Sync {
    fn add_data(&self, data: Value, ctx: &PipelineContext) -> Result<(), String>;
    fn get_result(&self) -> Option<Value>;
    fn reset(&self);
}

/// Data sink — writes items out.
pub trait Sink: Send + Sync {
    fn write(&self, data: Value, ctx: &PipelineContext) -> Result<(), String>;
    fn flush(&self) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
}

// ── Status Types ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum StageState {
    Idle,
    Running,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub id: String,
    pub state: StageState,
    pub items_emitted: u64,
}

// ── Pipeline Context ────────────────────────────────────────────────

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

