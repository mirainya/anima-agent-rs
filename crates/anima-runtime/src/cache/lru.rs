//! LRU（最近最少使用）缓存模块
//!
//! 提供基于 LRU 淘汰策略的键值缓存，支持：
//! - 可选的 TTL 过期机制
//! - 自动淘汰最久未访问的条目
//! - 缓存命中率统计
//! - 多种缓存键生成工具函数（API/任务/会话）
//!
//! 内部使用 IndexMap 保持插入顺序，配合独立的 access_order 向量追踪访问顺序。

use crate::support::now_ms;
use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};

/// 缓存条目，存储值及其元数据（创建时间、访问次数、TTL 等）
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

/// 缓存统计信息
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

/// 淘汰策略 trait：定义缓存满时如何选择被淘汰的条目
pub trait EvictionPolicy: Send + Sync {
    /// 选择一个待淘汰的 key
    fn select_for_eviction(&self) -> Option<String>;
    /// 通知策略某个 key 被访问了（用于更新访问顺序）
    fn on_access(&mut self, key: &str);
    /// 通知策略某个 key 被写入了
    fn on_set(&mut self, key: &str);
    /// 通知策略某个 key 被删除了
    fn on_delete(&mut self, key: &str);
}

/// LRU 淘汰策略实现
///
/// 通过维护一个访问顺序向量来追踪 key 的使用情况，
/// 向量头部是最久未访问的 key，尾部是最近访问的。
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
    /// 淘汰最久未访问的 key（向量头部）
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

/// LRU 缓存
///
/// 线程安全的 LRU 缓存实现，支持 TTL 过期和容量上限淘汰。
/// 使用 parking_lot::Mutex 保证并发安全。
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

    /// 读取缓存，自动处理过期淘汰和访问顺序更新
    pub fn get(&self, key: &str) -> Option<Value> {
        let mut entries = self.entries.lock();
        let expired = entries
            .get(key)
            .map(is_cache_entry_expired)
            .unwrap_or(false);
        if expired {
            entries.shift_remove(key);
            drop(entries);
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

    /// 写入缓存，容量满时自动淘汰最久未访问的条目
    pub fn set(&self, key: &str, value: Value, ttl_ms: Option<u64>) -> Value {
        // 容量已满且不是更新已有 key 时，循环淘汰直到有空间
        loop {
            let entries = self.entries.lock();
            if entries.len() < self.max_entries || entries.contains_key(key) {
                break;
            }
            drop(entries);
            // 注意：必须先 drop access_order 锁再调 delete，否则死锁
            let evict_key = self.access_order.lock().first().cloned();
            if let Some(evict_key) = evict_key {
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

    /// 缓存穿透保护：命中则返回缓存值，未命中则计算并写入
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

// ── 缓存键生成工具函数 ───────────────────────────────────────────

/// 生成 API 调用的缓存键（基于会话 ID 和内容哈希）
pub fn make_api_cache_key(session_id: &str, content: &str) -> String {
    format!("api:{session_id}:{}", stable_hash(content))
}

/// 生成任务的缓存键（基于任务类型和负载哈希）
pub fn make_task_cache_key(task_type: &str, payload: &str) -> String {
    format!("task:{task_type}:{}", stable_hash(payload))
}

/// 生成会话数据的缓存键
pub fn make_session_cache_key(session_id: &str, data_type: &str) -> String {
    format!("session:{session_id}:{}", stable_hash(data_type))
}

fn is_cache_entry_expired(entry: &CacheEntry) -> bool {
    match entry.ttl_ms {
        Some(ttl_ms) => now_ms().saturating_sub(entry.created_at_ms) > ttl_ms,
        None => false,
    }
}

/// 粗略估算 JSON 值的内存占用（字节数）
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

/// FNV-1a 哈希：用于生成稳定的缓存键，避免直接暴露原始内容
fn stable_hash(input: &str) -> u64 {
    let mut hash = 1469598103934665603u64;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_miss_on_empty_cache() {
        let cache = LruCache::new(Some(10), Some(60_000));
        assert!(cache.get("nonexistent").is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_set_and_get() {
        let cache = LruCache::new(Some(10), Some(60_000));
        cache.set("key1", json!("value1"), None);
        assert_eq!(cache.get("key1"), Some(json!("value1")));
        let stats = cache.stats();
        assert_eq!(stats.writes, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.entry_count, 1);
    }

    #[test]
    fn test_overwrite_same_key() {
        let cache = LruCache::new(Some(10), Some(60_000));
        cache.set("k", json!(1), None);
        cache.set("k", json!(2), None);
        assert_eq!(cache.get("k"), Some(json!(2)));
        assert_eq!(cache.stats().entry_count, 1);
        assert_eq!(cache.stats().writes, 2);
    }

    #[test]
    fn test_delete() {
        let cache = LruCache::new(Some(10), Some(60_000));
        cache.set("k", json!("v"), None);
        assert!(cache.delete("k"));
        assert!(cache.get("k").is_none());
        assert!(!cache.delete("k")); // already deleted
    }

    #[test]
    fn test_eviction_on_capacity() {
        let cache = LruCache::new(Some(3), Some(60_000));
        cache.set("a", json!(1), None);
        cache.set("b", json!(2), None);
        cache.set("c", json!(3), None);
        // 访问 a 使其变为最近使用
        cache.get("a");
        // 插入 d 应淘汰 b（最久未使用）
        cache.set("d", json!(4), None);
        assert!(cache.get("b").is_none(), "b should have been evicted");
        assert_eq!(cache.get("a"), Some(json!(1)));
        assert_eq!(cache.get("d"), Some(json!(4)));
        assert!(cache.stats().evictions >= 1);
    }

    #[test]
    fn test_ttl_expiration() {
        let cache = LruCache::new(Some(10), Some(60_000));
        // 设置一个极短的 TTL（1ms）
        cache.set("ephemeral", json!("gone"), Some(1));
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.get("ephemeral").is_none(), "should have expired");
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_no_ttl_never_expires() {
        let cache = LruCache::new(Some(10), None);
        // 手动构造无 TTL 条目
        {
            let mut entries = cache.entries.lock();
            entries.insert(
                "forever".into(),
                CacheEntry {
                    key: "forever".into(),
                    value: json!("alive"),
                    created_at_ms: 0, // 很久以前
                    accessed_at_ms: 0,
                    access_count: 0,
                    ttl_ms: None,
                    size: 5,
                    metadata: json!({}),
                },
            );
            cache.access_order.lock().push("forever".into());
        }
        assert_eq!(cache.get("forever"), Some(json!("alive")));
    }

    #[test]
    fn test_get_or_compute() {
        let cache = LruCache::new(Some(10), Some(60_000));
        let v = cache.get_or_compute("computed", || json!(42));
        assert_eq!(v, json!(42));
        // 第二次应命中缓存，不调用 compute
        let v2 = cache.get_or_compute("computed", || json!(999));
        assert_eq!(v2, json!(42));
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_stats_hit_rate() {
        let cache = LruCache::new(Some(10), Some(60_000));
        cache.set("k", json!(1), None);
        cache.get("k"); // hit
        cache.get("missing"); // miss
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_key_generators() {
        let k1 = make_api_cache_key("session1", "content");
        let k2 = make_api_cache_key("session1", "content");
        assert_eq!(k1, k2); // 相同输入产生相同 key
        let k3 = make_api_cache_key("session1", "different");
        assert_ne!(k1, k3); // 不同输入产生不同 key

        let tk = make_task_cache_key("classify", "payload");
        assert!(tk.starts_with("task:classify:"));

        let sk = make_session_cache_key("s1", "history");
        assert!(sk.starts_with("session:s1:"));
    }

    #[test]
    fn test_lru_eviction_policy() {
        let mut policy = LruEvictionPolicy::new();
        policy.on_set("a");
        policy.on_set("b");
        policy.on_set("c");
        // a 是最旧的
        assert_eq!(policy.select_for_eviction(), Some("a".into()));
        // 访问 a 后，b 变为最旧
        policy.on_access("a");
        assert_eq!(policy.select_for_eviction(), Some("b".into()));
        // 删除 b
        policy.on_delete("b");
        assert_eq!(policy.select_for_eviction(), Some("c".into()));
    }
}
