use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;
use crate::support::now_ms;

#[derive(Debug, Clone, PartialEq)]
pub struct ContextSnapshot {
    pub id: String,
    pub key: String,
    pub value: Value,
    pub source_tier: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContextStats {
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    pub touches: u64,
    pub promotions: u64,
    pub demotions: u64,
    pub gc_runs: u64,
    pub snapshots: usize,
    pub entries: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManagerStatus {
    pub id: String,
    pub status: String,
    pub stats: ContextStats,
}

const MAX_SNAPSHOTS: usize = 500;
const SNAPSHOT_MAX_AGE_MS: u64 = 3_600_000; // 1 hour

#[derive(Debug, Default)]
pub struct ContextManager {
    id: String,
    contexts: Mutex<IndexMap<String, Value>>,
    snapshots: Mutex<IndexMap<String, ContextSnapshot>>,
    stats: Mutex<ContextStats>,
    gc_running: AtomicBool,
}

impl ContextManager {
    pub fn new(enable_gc: Option<bool>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            contexts: Mutex::new(IndexMap::new()),
            snapshots: Mutex::new(IndexMap::new()),
            stats: Mutex::new(ContextStats::default()),
            gc_running: AtomicBool::new(enable_gc.unwrap_or(true)),
        }
    }

    pub fn get_context(&self, key: &str) -> Option<Value> {
        let value = self.contexts.lock().get(key).cloned();
        if value.is_some() {
            self.stats.lock().reads += 1;
        }
        value
    }

    pub fn set_context(&self, key: &str, value: Value) -> Value {
        let len = {
            let mut contexts = self.contexts.lock();
            contexts.insert(key.to_string(), value.clone());
            contexts.len()
        };
        let mut stats = self.stats.lock();
        stats.writes += 1;
        stats.entries = len;
        value
    }

    pub fn delete_context(&self, key: &str) -> bool {
        let (deleted, len) = {
            let mut contexts = self.contexts.lock();
            let deleted = contexts.shift_remove(key).is_some();
            (deleted, contexts.len())
        };
        if deleted {
            let mut stats = self.stats.lock();
            stats.deletes += 1;
            stats.entries = len;
        }
        deleted
    }

    pub fn get_or_create<F>(&self, key: &str, default_fn: F) -> Value
    where
        F: FnOnce() -> Value,
    {
        if let Some(value) = self.get_context(key) {
            return value;
        }
        let value = default_fn();
        self.set_context(key, value.clone());
        value
    }

    pub fn snapshot(&self, key: &str) -> Option<ContextSnapshot> {
        let value = self.get_context(key)?;
        let snapshot = ContextSnapshot {
            id: Uuid::new_v4().to_string(),
            key: key.to_string(),
            value,
            source_tier: "l1".into(),
            timestamp_ms: now_ms(),
        };
        let snapshots_len = {
            let mut snapshots = self.snapshots.lock();
            // Evict if at capacity
            if snapshots.len() >= MAX_SNAPSHOTS {
                // Try removing expired first
                let now = now_ms();
                let expired: Vec<String> = snapshots
                    .iter()
                    .filter(|(_, s)| now.saturating_sub(s.timestamp_ms) >= SNAPSHOT_MAX_AGE_MS)
                    .map(|(k, _)| k.clone())
                    .collect();
                if !expired.is_empty() {
                    for k in &expired {
                        snapshots.shift_remove(k);
                    }
                } else if let Some(oldest_key) = snapshots.keys().next().cloned() {
                    snapshots.shift_remove(&oldest_key);
                }
            }
            snapshots.insert(snapshot.id.clone(), snapshot.clone());
            snapshots.len()
        };
        self.stats.lock().snapshots = snapshots_len;
        Some(snapshot)
    }

    pub fn restore(&self, snapshot_id: &str) -> Option<ContextSnapshot> {
        let snapshot = self.snapshots.lock().get(snapshot_id).cloned()?;
        self.set_context(&snapshot.key, snapshot.value.clone());
        Some(snapshot)
    }

    pub fn list_keys(&self, pattern: &str) -> Vec<String> {
        let prefix = pattern.strip_suffix('*').unwrap_or(pattern);
        self.contexts
            .lock()
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }

    pub fn add_to_session_history(&self, session_id: &str, entry: Value) {
        let key = format!("session:{session_id}:history");
        let mut current = self.get_context(&key).unwrap_or_else(|| json!([]));
        if !current.is_array() {
            current = json!([]);
        }
        current.as_array_mut().unwrap().push(entry);
        self.set_context(&key, current);
    }

    pub fn get_session_history(&self, session_id: &str) -> Vec<Value> {
        self.get_context(&format!("session:{session_id}:history"))
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
    }

    pub fn clear_session_context(&self, session_id: &str) {
        let pattern = format!("session:{session_id}*");
        for key in self.list_keys(&pattern) {
            let _ = self.delete_context(&key);
        }
    }

    pub fn close(&self) {
        self.gc_running.store(false, Ordering::SeqCst);
        self.contexts.lock().clear();
        self.snapshots.lock().clear();
        let mut stats = self.stats.lock();
        stats.entries = 0;
        stats.snapshots = 0;
    }

    /// Remove snapshots older than `SNAPSHOT_MAX_AGE_MS`. Returns count removed.
    pub fn gc_old_snapshots(&self) -> usize {
        let now = now_ms();
        let mut snapshots = self.snapshots.lock();
        let before = snapshots.len();
        snapshots.retain(|_, s| now.saturating_sub(s.timestamp_ms) < SNAPSHOT_MAX_AGE_MS);
        let removed = before - snapshots.len();
        if removed > 0 {
            let mut stats = self.stats.lock();
            stats.gc_runs += 1;
            stats.snapshots = snapshots.len();
        }
        removed
    }

    pub fn status(&self) -> ManagerStatus {
        let mut stats = self.stats.lock().clone();
        stats.entries = self.contexts.lock().len();
        stats.snapshots = self.snapshots.lock().len();
        ManagerStatus {
            id: self.id.clone(),
            status: if self.gc_running.load(Ordering::SeqCst) {
                "running"
            } else {
                "stopped"
            }
            .into(),
            stats,
        }
    }
}
