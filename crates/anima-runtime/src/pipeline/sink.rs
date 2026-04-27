use crate::pipeline::traits::*;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Channel Sink ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum SinkState {
    Open,
    Closed,
}

pub struct ChannelSink<T: Send + 'static = serde_json::Value> {
    #[allow(dead_code)]
    id: String,
    tx: Sender<T>,
    state: Mutex<SinkState>,
    items: AtomicU64,
}

impl<T: Send + 'static> ChannelSink<T> {
    pub fn new(tx: Sender<T>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            tx,
            state: Mutex::new(SinkState::Open),
            items: AtomicU64::new(0),
        }
    }
}

impl<T: Send + Sync + 'static> Sink<T> for ChannelSink<T> {
    fn write(&self, data: T, _ctx: &PipelineContext) -> Result<(), String> {
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

pub struct CollectionSink<T: Send + 'static = serde_json::Value> {
    items: Mutex<Vec<T>>,
    state: Mutex<SinkState>,
}

impl<T: Send + 'static> Default for CollectionSink<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> CollectionSink<T> {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            state: Mutex::new(SinkState::Open),
        }
    }

    pub fn items(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.items.lock().clone()
    }

    pub fn len(&self) -> usize {
        self.items.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Send + Sync + 'static> Sink<T> for CollectionSink<T> {
    fn write(&self, data: T, _ctx: &PipelineContext) -> Result<(), String> {
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

pub struct CallbackSink<T: Send + Sync + 'static = serde_json::Value> {
    callback: Box<dyn Fn(T) + Send + Sync>,
    state: Mutex<SinkState>,
    items: AtomicU64,
}

impl<T: Send + Sync + 'static> CallbackSink<T> {
    pub fn new(callback: impl Fn(T) + Send + Sync + 'static) -> Self {
        Self {
            callback: Box::new(callback),
            state: Mutex::new(SinkState::Open),
            items: AtomicU64::new(0),
        }
    }
}

impl<T: Send + Sync + 'static> Sink<T> for CallbackSink<T> {
    fn write(&self, data: T, _ctx: &PipelineContext) -> Result<(), String> {
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

impl<T: Send + Sync + 'static> Sink<T> for NullSink {
    fn write(&self, _data: T, _ctx: &PipelineContext) -> Result<(), String> {
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
