//! 数据管道核心实现模块
//!
//! `Pipeline` 是数据处理的核心引擎，采用 Builder 模式组装各阶段组件，
//! 数据按 Filter → Transform → Aggregate/Sink 的顺序流经管道。
//! 使用原子计数器和 Mutex 实现线程安全的并发处理和统计。

use crate::pipeline::traits::*;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Pipeline Stats ──────────────────────────────────────────────────

/// 管道处理统计信息
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub items_processed: u64,
    pub items_filtered: u64,
    pub errors: u64,
}

// ── Pipeline ────────────────────────────────────────────────────────

/// 管道运行状态
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineState {
    Idle,
    Running,
    Stopped,
}

/// 数据处理管道，通过 Builder 模式组装 Source/Transform/Filter/Aggregate/Sink
///
/// 各阶段组件通过 Mutex 保护以支持并发访问，
/// 统计计数器使用 AtomicU64 实现无锁更新。
pub struct Pipeline<T: Send + Sync + Clone + 'static = Value> {
    pub id: String,
    source: Mutex<Option<Box<dyn Source>>>,
    transforms: Mutex<Vec<Box<dyn Transform<T>>>>,
    filters: Mutex<Vec<Box<dyn Filter<T>>>>,
    aggregate: Mutex<Option<Box<dyn Aggregate<T>>>>,
    sink: Mutex<Option<Box<dyn Sink<T>>>>,
    state: Mutex<PipelineState>,
    items_processed: AtomicU64,
    items_filtered: AtomicU64,
    errors: AtomicU64,
}

impl Default for Pipeline<Value> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Sync + Clone + 'static> Pipeline<T> {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            source: Mutex::new(None),
            transforms: Mutex::new(Vec::new()),
            filters: Mutex::new(Vec::new()),
            aggregate: Mutex::new(None),
            sink: Mutex::new(None),
            state: Mutex::new(PipelineState::Idle),
            items_processed: AtomicU64::new(0),
            items_filtered: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn add_source(self, source: Box<dyn Source>) -> Self {
        *self.source.lock() = Some(source);
        self
    }

    pub fn add_transform(self, transform: Box<dyn Transform<T>>) -> Self {
        self.transforms.lock().push(transform);
        self
    }

    pub fn add_filter(self, filter: Box<dyn Filter<T>>) -> Self {
        self.filters.lock().push(filter);
        self
    }

    pub fn add_aggregate(self, aggregate: Box<dyn Aggregate<T>>) -> Self {
        *self.aggregate.lock() = Some(aggregate);
        self
    }

    pub fn add_sink(self, sink: Box<dyn Sink<T>>) -> Self {
        *self.sink.lock() = Some(sink);
        self
    }

    /// 处理单个数据项，依次经过 Filter → Transform → Aggregate/Sink 各阶段。
    /// 返回 Ok(None) 表示数据被过滤或被聚合器吸收，Ok(Some) 表示处理完成的输出。
    pub fn process(&self, data: T) -> Result<Option<T>, String> {
        let ctx = PipelineContext::new(&self.id);
        self.items_processed.fetch_add(1, Ordering::Relaxed);

        let filters = self.filters.lock();
        for filter in filters.iter() {
            if !filter.passes(&data, &ctx) {
                self.items_filtered.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        }
        drop(filters);

        let transforms = self.transforms.lock();
        let mut current = data;
        for transform in transforms.iter() {
            match transform.transform(current, &ctx) {
                Ok(Some(next)) => current = next,
                Ok(None) => {
                    self.items_filtered.fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }
                Err(e) => {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
        drop(transforms);

        let agg = self.aggregate.lock();
        if let Some(ref aggregate) = *agg {
            aggregate.add_data(current, &ctx)?;
            return Ok(None);
        }
        drop(agg);

        let sink = self.sink.lock();
        if let Some(ref sink) = *sink {
            sink.write(current.clone(), &ctx)?;
        }

        Ok(Some(current))
    }

    /// Process a batch of items.
    pub fn process_batch(&self, items: Vec<T>) -> Vec<Result<Option<T>, String>> {
        items.into_iter().map(|item| self.process(item)).collect()
    }

    /// 启动管道，将状态设为 Running 并启动数据源
    pub fn start(&self) -> Result<(), String> {
        let ctx = PipelineContext::new(&self.id);
        *self.state.lock() = PipelineState::Running;
        if let Some(ref source) = *self.source.lock() {
            source.start(&ctx)?;
        }
        Ok(())
    }

    /// 停止管道，依次停止数据源、刷新并关闭 Sink
    pub fn stop(&self) -> Result<(), String> {
        *self.state.lock() = PipelineState::Stopped;
        if let Some(ref source) = *self.source.lock() {
            source.stop()?;
        }
        let sink = self.sink.lock();
        if let Some(ref sink) = *sink {
            sink.flush()?;
            sink.close()?;
        }
        Ok(())
    }

    pub fn state(&self) -> PipelineState {
        self.state.lock().clone()
    }

    pub fn stats(&self) -> PipelineStats {
        PipelineStats {
            items_processed: self.items_processed.load(Ordering::Relaxed),
            items_filtered: self.items_filtered.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }

    pub fn aggregation_result(&self) -> Option<T> {
        self.aggregate.lock().as_ref().and_then(|a| a.get_result())
    }
}
