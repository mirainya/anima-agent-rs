use crate::pipeline::traits::*;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Map Transform ───────────────────────────────────────────────────

pub struct MapTransform<T: Send + Sync + 'static = Value> {
    #[allow(dead_code)]
    id: String,
    f: Box<dyn Fn(T) -> T + Send + Sync>,
    items: AtomicU64,
}

impl<T: Send + Sync + 'static> MapTransform<T> {
    pub fn new(f: impl Fn(T) -> T + Send + Sync + 'static) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            f: Box::new(f),
            items: AtomicU64::new(0),
        }
    }
}

impl<T: Send + Sync + 'static> Transform<T> for MapTransform<T> {
    fn transform(&self, data: T, _ctx: &PipelineContext) -> Result<Option<T>, String> {
        self.items.fetch_add(1, Ordering::Relaxed);
        Ok(Some((self.f)(data)))
    }
}

// ── Filter Transform ────────────────────────────────────────────────

pub struct PredicateFilter<T: Send + Sync + 'static = Value> {
    #[allow(dead_code)]
    id: String,
    predicate: Box<dyn Fn(&T) -> bool + Send + Sync>,
}

impl<T: Send + Sync + 'static> PredicateFilter<T> {
    pub fn new(predicate: impl Fn(&T) -> bool + Send + Sync + 'static) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            predicate: Box::new(predicate),
        }
    }
}

impl<T: Send + Sync + 'static> Filter<T> for PredicateFilter<T> {
    fn passes(&self, data: &T, _ctx: &PipelineContext) -> bool {
        (self.predicate)(data)
    }
}

// ── Batch Transform ─────────────────────────────────────────────────

pub struct BatchTransform {
    batch_size: usize,
    buffer: Mutex<Vec<Value>>,
}

impl BatchTransform {
    pub fn new(batch_size: usize) -> Self {
        Self {
            batch_size,
            buffer: Mutex::new(Vec::with_capacity(batch_size)),
        }
    }
}

impl Transform<Value> for BatchTransform {
    fn transform(&self, data: Value, _ctx: &PipelineContext) -> Result<Option<Value>, String> {
        let mut buf = self.buffer.lock();
        buf.push(data);
        if buf.len() >= self.batch_size {
            let batch: Vec<Value> = buf.drain(..).collect();
            Ok(Some(Value::Array(batch)))
        } else {
            Ok(None) // Not yet a full batch
        }
    }
}

// ── Sum Aggregate ───────────────────────────────────────────────────

pub struct SumAggregate {
    state: Mutex<f64>,
    count: AtomicU64,
}

impl Default for SumAggregate {
    fn default() -> Self {
        Self::new()
    }
}

impl SumAggregate {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(0.0),
            count: AtomicU64::new(0),
        }
    }
}

impl Aggregate<Value> for SumAggregate {
    fn add_data(&self, data: Value, _ctx: &PipelineContext) -> Result<(), String> {
        let n = data.as_f64().ok_or("expected number")?;
        *self.state.lock() += n;
        self.count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn get_result(&self) -> Option<Value> {
        Some(serde_json::json!({
            "sum": *self.state.lock(),
            "count": self.count.load(Ordering::Relaxed),
        }))
    }
    fn reset(&self) {
        *self.state.lock() = 0.0;
        self.count.store(0, Ordering::Relaxed);
    }
}

// ── Collect Aggregate ───────────────────────────────────────────────

pub struct CollectAggregate {
    items: Mutex<Vec<Value>>,
}

impl Default for CollectAggregate {
    fn default() -> Self {
        Self::new()
    }
}

impl CollectAggregate {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
        }
    }
}

impl Aggregate<Value> for CollectAggregate {
    fn add_data(&self, data: Value, _ctx: &PipelineContext) -> Result<(), String> {
        self.items.lock().push(data);
        Ok(())
    }
    fn get_result(&self) -> Option<Value> {
        let items = self.items.lock();
        if items.is_empty() {
            None
        } else {
            Some(Value::Array(items.clone()))
        }
    }
    fn reset(&self) {
        self.items.lock().clear();
    }
}
