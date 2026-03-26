use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};
use crate::support::now_ms;

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub key: String,
    pub value: Value,
    pub created_at_ms: u64,
    pub accessed_at_ms: u64,
    pub access_count: u64,
    pub ttl_ms: Option<u64>,
    pub size: usize,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub writes: u64,
    pub size_bytes: usize,
    pub entry_count: usize,
    pub hit_rate: f64,
}

pub trait EvictionPolicy: Send + Sync {
    fn select_for_eviction(&self) -> Option<String>;
    fn on_access(&mut self, key: &str);
    fn on_set(&mut self, key: &str);
    fn on_delete(&mut self, key: &str);
}

#[derive(Debug)]
pub struct LruEvictionPolicy {
    access_order: Vec<String>,
}

impl LruEvictionPolicy {
    pub fn new() -> Self {
        Self {
            access_order: Vec::new(),
        }
    }
}

impl Default for LruEvictionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl EvictionPolicy for LruEvictionPolicy {
    fn select_for_eviction(&self) -> Option<String> {
        self.access_order.first().cloned()
    }

    fn on_access(&mut self, key: &str) {
        self.access_order.retain(|k| k != key);
        self.access_order.push(key.to_string());
    }

    fn on_set(&mut self, key: &str) {
        self.access_order.retain(|k| k != key);
        self.access_order.push(key.to_string());
    }

    fn on_delete(&mut self, key: &str) {
        self.access_order.retain(|k| k != key);
    }
}

#[derive(Debug)]
pub struct LruCache {
    entries: Mutex<IndexMap<String, CacheEntry>>,
    access_order: Mutex<Vec<String>>,
    max_entries: usize,
    default_ttl_ms: u64,
    stats: Mutex<CacheStats>,
}

impl LruCache {
    pub fn new(max_entries: Option<usize>, default_ttl_ms: Option<u64>) -> Self {
        Self {
            entries: Mutex::new(IndexMap::new()),
            access_order: Mutex::new(Vec::new()),
            max_entries: max_entries.unwrap_or(1000),
            default_ttl_ms: default_ttl_ms.unwrap_or(5 * 60 * 1000),
            stats: Mutex::new(CacheStats::default()),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut entries = self.entries.lock();
        let expired = entries
            .get(key)
            .map(is_cache_entry_expired)
            .unwrap_or(false);
        if expired {
            entries.shift_remove(key);
            self.access_order.lock().retain(|existing| existing != key);
            self.stats.lock().misses += 1;
            return None;
        }

        let value = if let Some(entry) = entries.get_mut(key) {
            entry.access_count += 1;
            entry.accessed_at_ms = now_ms();
            Some(entry.value.clone())
        } else {
            None
        };
        drop(entries);

        if value.is_some() {
            let mut order = self.access_order.lock();
            order.retain(|existing| existing != key);
            order.push(key.to_string());
            self.stats.lock().hits += 1;
        } else {
            self.stats.lock().misses += 1;
        }
        value
    }

    pub fn set(&self, key: &str, value: Value, ttl_ms: Option<u64>) -> Value {
        loop {
            let entries = self.entries.lock();
            if entries.len() < self.max_entries || entries.contains_key(key) {
                break;
            }
            drop(entries);
            if let Some(evict_key) = self.access_order.lock().first().cloned() {
                self.delete(&evict_key);
                self.stats.lock().evictions += 1;
            } else {
                break;
            }
        }

        let now = now_ms();
        let entry = CacheEntry {
            key: key.to_string(),
            size: estimate_size(&value),
            value: value.clone(),
            created_at_ms: now,
            accessed_at_ms: now,
            access_count: 0,
            ttl_ms: ttl_ms.or(Some(self.default_ttl_ms)),
            metadata: json!({}),
        };
        self.entries.lock().insert(key.to_string(), entry);
        let mut order = self.access_order.lock();
        order.retain(|existing| existing != key);
        order.push(key.to_string());
        self.stats.lock().writes += 1;
        drop(order);
        value
    }

    pub fn delete(&self, key: &str) -> bool {
        let deleted = self.entries.lock().shift_remove(key).is_some();
        if deleted {
            self.access_order.lock().retain(|existing| existing != key);
        }
        deleted
    }

    pub fn get_or_compute<F>(&self, key: &str, compute_fn: F) -> Value
    where
        F: FnOnce() -> Value,
    {
        if let Some(value) = self.get(key) {
            return value;
        }
        let value = compute_fn();
        self.set(key, value.clone(), None);
        value
    }

    pub fn stats(&self) -> CacheStats {
        let mut stats = self.stats.lock().clone();
        let entries = self.entries.lock();
        stats.entry_count = entries.len();
        stats.size_bytes = entries.values().map(|entry| entry.size).sum();
        let total = stats.hits + stats.misses;
        stats.hit_rate = if total > 0 {
            stats.hits as f64 / total as f64
        } else {
            0.0
        };
        stats
    }
}

pub fn make_api_cache_key(session_id: &str, content: &str) -> String {
    format!("api:{session_id}:{}", stable_hash(content))
}

pub fn make_task_cache_key(task_type: &str, payload: &str) -> String {
    format!("task:{task_type}:{}", stable_hash(payload))
}

pub fn make_session_cache_key(session_id: &str, data_type: &str) -> String {
    format!("session:{session_id}:{}", stable_hash(data_type))
}

fn is_cache_entry_expired(entry: &CacheEntry) -> bool {
    match entry.ttl_ms {
        Some(ttl_ms) => now_ms().saturating_sub(entry.created_at_ms) > ttl_ms,
        None => false,
    }
}

fn estimate_size(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 8,
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(estimate_size).sum(),
        Value::Object(map) => map.iter().map(|(k, v)| k.len() + estimate_size(v)).sum(),
    }
}

fn stable_hash(input: &str) -> u64 {
    let mut hash = 1469598103934665603u64;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}
