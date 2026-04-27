use crate::pipeline::traits::*;
use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ── Channel Source ──────────────────────────────────────────────────

pub struct ChannelSource<T: Send + 'static = serde_json::Value> {
    id: String,
    rx: Receiver<T>,
    state: Mutex<StageState>,
    items: AtomicU64,
}

impl<T: Send + 'static> ChannelSource<T> {
    pub fn new(rx: Receiver<T>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            rx,
            state: Mutex::new(StageState::Idle),
            items: AtomicU64::new(0),
        }
    }

    pub fn recv(&self) -> Option<T> {
        self.rx.recv().ok().inspect(|_v| {
            self.items.fetch_add(1, Ordering::Relaxed);
        })
    }

    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok().inspect(|_v| {
            self.items.fetch_add(1, Ordering::Relaxed);
        })
    }
}

impl<T: Send + 'static> Source for ChannelSource<T> {
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

pub struct CollectionSource<T: Send + 'static = serde_json::Value> {
    id: String,
    items: Mutex<Vec<T>>,
    state: Mutex<StageState>,
    emitted: AtomicU64,
}

impl<T: Send + 'static> CollectionSource<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            items: Mutex::new(items),
            state: Mutex::new(StageState::Idle),
            emitted: AtomicU64::new(0),
        }
    }

    pub fn next_item(&self) -> Option<T> {
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

impl<T: Send + 'static> Source for CollectionSource<T> {
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
