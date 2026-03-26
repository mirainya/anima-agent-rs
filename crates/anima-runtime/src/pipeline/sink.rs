use crate::pipeline::traits::*;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Channel Sink ────────────────────────────────────────────────────

pub struct ChannelSink {
    #[allow(dead_code)]
    id: String,
    tx: Sender<Value>,
    state: Mutex<SinkState>,
    items: AtomicU64,
}

#[derive(Debug, Clone, PartialEq)]
enum SinkState {
    Open,
    Closed,
}

impl ChannelSink {
    pub fn new(tx: Sender<Value>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            tx,
            state: Mutex::new(SinkState::Open),
            items: AtomicU64::new(0),
        }
    }
}

impl Sink for ChannelSink {
    fn write(&self, data: Value, _ctx: &PipelineContext) -> Result<(), String> {
        if *self.state.lock() != SinkState::Open {
            return Err("sink closed".into());
        }
        self.tx.send(data).map_err(|e| e.to_string())?;
        self.items.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn flush(&self) -> Result<(), String> {
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        *self.state.lock() = SinkState::Closed;
        Ok(())
    }
}

// ── Collection Sink ─────────────────────────────────────────────────

pub struct CollectionSink {
    items: Mutex<Vec<Value>>,
    state: Mutex<SinkState>,
}

impl Default for CollectionSink {
    fn default() -> Self {
        Self::new()
    }
}

impl CollectionSink {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            state: Mutex::new(SinkState::Open),
        }
    }

    pub fn items(&self) -> Vec<Value> {
        self.items.lock().clone()
    }

    pub fn len(&self) -> usize {
        self.items.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Sink for CollectionSink {
    fn write(&self, data: Value, _ctx: &PipelineContext) -> Result<(), String> {
        if *self.state.lock() != SinkState::Open {
            return Err("sink closed".into());
        }
        self.items.lock().push(data);
        Ok(())
    }
    fn flush(&self) -> Result<(), String> {
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        *self.state.lock() = SinkState::Closed;
        Ok(())
    }
}

// ── Callback Sink ───────────────────────────────────────────────────

pub struct CallbackSink {
    callback: Box<dyn Fn(Value) + Send + Sync>,
    state: Mutex<SinkState>,
    items: AtomicU64,
}

impl CallbackSink {
    pub fn new(callback: impl Fn(Value) + Send + Sync + 'static) -> Self {
        Self {
            callback: Box::new(callback),
            state: Mutex::new(SinkState::Open),
            items: AtomicU64::new(0),
        }
    }
}

impl Sink for CallbackSink {
    fn write(&self, data: Value, _ctx: &PipelineContext) -> Result<(), String> {
        if *self.state.lock() != SinkState::Open {
            return Err("sink closed".into());
        }
        (self.callback)(data);
        self.items.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn flush(&self) -> Result<(), String> {
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        *self.state.lock() = SinkState::Closed;
        Ok(())
    }
}

// ── Null Sink ───────────────────────────────────────────────────────

pub struct NullSink {
    items: AtomicU64,
}

impl Default for NullSink {
    fn default() -> Self {
        Self::new()
    }
}

impl NullSink {
    pub fn new() -> Self {
        Self {
            items: AtomicU64::new(0),
        }
    }

    pub fn count(&self) -> u64 {
        self.items.load(Ordering::Relaxed)
    }
}

impl Sink for NullSink {
    fn write(&self, _data: Value, _ctx: &PipelineContext) -> Result<(), String> {
        self.items.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn flush(&self) -> Result<(), String> {
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}
