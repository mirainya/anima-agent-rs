use crate::runtime::events::RuntimeDomainEvent;
use crate::runtime::reducer::reduce_event;
use crate::runtime::snapshot::RuntimeStateSnapshot;
use crate::support::now_ms;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, Default)]
struct RuntimeStateStoreInner {
    next_sequence: u64,
    snapshot: RuntimeStateSnapshot,
}

#[derive(Debug, Default)]
pub struct RuntimeStateStore {
    inner: Mutex<RuntimeStateStoreInner>,
}

impl RuntimeStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&self, event: RuntimeDomainEvent) -> u64 {
        let mut inner = self.inner.lock();
        inner.next_sequence += 1;
        let sequence = inner.next_sequence;
        reduce_event(&mut inner.snapshot, event, sequence, now_ms());
        sequence
    }

    pub fn snapshot(&self) -> RuntimeStateSnapshot {
        self.inner.lock().snapshot.clone()
    }
}

pub type SharedRuntimeStateStore = Arc<RuntimeStateStore>;
