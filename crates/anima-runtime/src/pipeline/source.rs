use crate::pipeline::traits::*;
use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Channel Source ──────────────────────────────────────────────────

pub struct ChannelSource {
    id: String,
    rx: Receiver<Value>,
    state: Mutex<StageState>,
    items: AtomicU64,
}

impl ChannelSource {
    pub fn new(rx: Receiver<Value>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            rx,
            state: Mutex::new(StageState::Idle),
            items: AtomicU64::new(0),
        }
    }

    pub fn recv(&self) -> Option<Value> {
        self.rx.recv().ok().inspect(|_v| {
            self.items.fetch_add(1, Ordering::Relaxed);
        })
    }

    pub fn try_recv(&self) -> Option<Value> {
        self.rx.try_recv().ok().inspect(|_v| {
            self.items.fetch_add(1, Ordering::Relaxed);
        })
    }
}

impl Source for ChannelSource {
    fn start(&self, _ctx: &PipelineContext) -> Result<(), String> {
        *self.state.lock() = StageState::Running;
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        *self.state.lock() = StageState::Stopped;
        Ok(())
    }
    fn status(&self) -> SourceStatus {
        SourceStatus {
            id: self.id.clone(),
            state: self.state.lock().clone(),
            items_emitted: self.items.load(Ordering::Relaxed),
        }
    }
}

// ── Collection Source ───────────────────────────────────────────────

pub struct CollectionSource {
    id: String,
    items: Mutex<Vec<Value>>,
    state: Mutex<StageState>,
    emitted: AtomicU64,
}

impl CollectionSource {
    pub fn new(items: Vec<Value>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            items: Mutex::new(items),
            state: Mutex::new(StageState::Idle),
            emitted: AtomicU64::new(0),
        }
    }

    pub fn next_item(&self) -> Option<Value> {
        let mut items = self.items.lock();
        if items.is_empty() {
            None
        } else {
            self.emitted.fetch_add(1, Ordering::Relaxed);
            Some(items.remove(0))
        }
    }

    pub fn remaining(&self) -> usize {
        self.items.lock().len()
    }
}

impl Source for CollectionSource {
    fn start(&self, _ctx: &PipelineContext) -> Result<(), String> {
        *self.state.lock() = StageState::Running;
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        *self.state.lock() = StageState::Stopped;
        Ok(())
    }
    fn status(&self) -> SourceStatus {
        SourceStatus {
            id: self.id.clone(),
            state: self.state.lock().clone(),
            items_emitted: self.emitted.load(Ordering::Relaxed),
        }
    }
}
