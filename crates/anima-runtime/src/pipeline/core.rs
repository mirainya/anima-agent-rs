use crate::pipeline::traits::*;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Pipeline Stats ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub items_processed: u64,
    pub items_filtered: u64,
    pub errors: u64,
}

// ── Pipeline ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineState {
    Idle,
    Running,
    Stopped,
}

pub struct Pipeline {
    pub id: String,
    source: Mutex<Option<Box<dyn Source>>>,
    transforms: Mutex<Vec<Box<dyn Transform>>>,
    filters: Mutex<Vec<Box<dyn Filter>>>,
    aggregate: Mutex<Option<Box<dyn Aggregate>>>,
    sink: Mutex<Option<Box<dyn Sink>>>,
    state: Mutex<PipelineState>,
    items_processed: AtomicU64,
    items_filtered: AtomicU64,
    errors: AtomicU64,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
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

    pub fn add_transform(self, transform: Box<dyn Transform>) -> Self {
        self.transforms.lock().push(transform);
        self
    }

    pub fn add_filter(self, filter: Box<dyn Filter>) -> Self {
        self.filters.lock().push(filter);
        self
    }

    pub fn add_aggregate(self, aggregate: Box<dyn Aggregate>) -> Self {
        *self.aggregate.lock() = Some(aggregate);
        self
    }

    pub fn add_sink(self, sink: Box<dyn Sink>) -> Self {
        *self.sink.lock() = Some(sink);
        self
    }

    /// Process a single data item through all stages (filter → transform → aggregate/sink).
    pub fn process(&self, data: Value) -> Result<Option<Value>, String> {
        let ctx = PipelineContext::new(&self.id);
        self.items_processed.fetch_add(1, Ordering::Relaxed);

        // 1. Filters
        let filters = self.filters.lock();
        for filter in filters.iter() {
            if !filter.passes(&data, &ctx) {
                self.items_filtered.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        }
        drop(filters);

        // 2. Transforms
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

        // 3. Aggregate (if present, absorbs data — no per-item output)
        let agg = self.aggregate.lock();
        if let Some(ref aggregate) = *agg {
            aggregate.add_data(current, &ctx)?;
            return Ok(None);
        }
        drop(agg);

        // 4. Sink
        let sink = self.sink.lock();
        if let Some(ref sink) = *sink {
            sink.write(current.clone(), &ctx)?;
        }

        Ok(Some(current))
    }

    /// Process a batch of items.
    pub fn process_batch(&self, items: Vec<Value>) -> Vec<Result<Option<Value>, String>> {
        items.into_iter().map(|item| self.process(item)).collect()
    }

    pub fn start(&self) -> Result<(), String> {
        let ctx = PipelineContext::new(&self.id);
        *self.state.lock() = PipelineState::Running;
        if let Some(ref source) = *self.source.lock() {
            source.start(&ctx)?;
        }
        Ok(())
    }

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

    pub fn aggregation_result(&self) -> Option<Value> {
        self.aggregate.lock().as_ref().and_then(|a| a.get_result())
    }
}
