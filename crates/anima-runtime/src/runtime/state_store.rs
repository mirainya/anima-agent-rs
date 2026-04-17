use crate::runtime::events::RuntimeDomainEvent;
use crate::runtime::reducer::reduce_event;
use crate::runtime::snapshot::{PersistedRuntimeState, RuntimeStateSnapshot};
use crate::support::now_ms;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RUNTIME_STATE_PERSISTENCE_VERSION: u32 = 1;

#[derive(Debug, Default)]
struct RuntimeStateStoreInner {
    next_sequence: u64,
    snapshot: RuntimeStateSnapshot,
}

#[derive(Debug)]
pub struct RuntimeStateStore {
    inner: Mutex<RuntimeStateStoreInner>,
    persistence_path: Option<PathBuf>,
}

impl Default for RuntimeStateStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(RuntimeStateStoreInner::default()),
            persistence_path: None,
        }
    }
}

impl RuntimeStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_persistence(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let restored = Self::load_persisted_state(&path).unwrap_or_else(|| PersistedRuntimeState {
            version: RUNTIME_STATE_PERSISTENCE_VERSION,
            next_sequence: 0,
            snapshot: RuntimeStateSnapshot::default(),
        });
        Self {
            inner: Mutex::new(RuntimeStateStoreInner {
                next_sequence: restored.next_sequence,
                snapshot: restored.snapshot,
            }),
            persistence_path: Some(path),
        }
    }

    pub fn append(&self, event: RuntimeDomainEvent) -> u64 {
        let mut inner = self.inner.lock();
        inner.next_sequence += 1;
        let sequence = inner.next_sequence;
        reduce_event(&mut inner.snapshot, event, sequence, now_ms());
        let persisted = PersistedRuntimeState {
            version: RUNTIME_STATE_PERSISTENCE_VERSION,
            next_sequence: inner.next_sequence,
            snapshot: inner.snapshot.clone(),
        };
        if let Some(path) = &self.persistence_path {
            let _ = Self::persist_state(path, &persisted);
        }
        sequence
    }

    pub fn snapshot(&self) -> RuntimeStateSnapshot {
        self.inner.lock().snapshot.clone()
    }

    pub fn next_sequence(&self) -> u64 {
        self.inner.lock().next_sequence
    }

    fn load_persisted_state(path: &Path) -> Option<PersistedRuntimeState> {
        let content = std::fs::read_to_string(path).ok()?;
        let persisted: PersistedRuntimeState = serde_json::from_str(&content).ok()?;
        if persisted.version != RUNTIME_STATE_PERSISTENCE_VERSION {
            return None;
        }
        Some(persisted)
    }

    fn persist_state(path: &Path, persisted: &PersistedRuntimeState) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(persisted)?;
        std::fs::write(&tmp_path, content)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

pub type SharedRuntimeStateStore = Arc<RuntimeStateStore>;
