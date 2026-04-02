//! TTL（生存时间）缓存模块
//!
//! 提供基于过期时间的键值缓存，每个条目都有独立的 TTL。
//! 与 LRU 缓存不同，TTL 缓存的淘汰策略纯粹基于时间：
//! - 读取时自动检测并清除过期条目
//! - 支持批量清理过期条目（purge_expired）
//! - 容量满时优先淘汰剩余 TTL 最短的条目

use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::Value;
use crate::support::now_ms;

// ── TTL 缓存条目 ─────────────────────────────────────────────────

/// 缓存条目，记录值及其生命周期信息
#[derive(Debug, Clone)]
pub struct TtlEntry {
    pub key: String,
    pub value: Value,
    pub created_at_ms: u64,
    pub accessed_at_ms: u64,
    pub access_count: u64,
    pub ttl_ms: u64,
    pub size: usize,
}

impl TtlEntry {
    fn is_expired(&self) -> bool {
        now_ms().saturating_sub(self.created_at_ms) > self.ttl_ms
    }

    fn remaining_ttl(&self) -> u64 {
        let elapsed = now_ms().saturating_sub(self.created_at_ms);
        self.ttl_ms.saturating_sub(elapsed)
    }
}

// ── 统计信息 ─────────────────────────────────────────────────────

/// TTL 缓存的运行统计
#[derive(Debug, Clone, Default)]
pub struct TtlCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub entry_count: usize,
    pub size_bytes: usize,
}

impl TtlCacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total > 0 {
            self.hits as f64 / total as f64
        } else {
            0.0
        }
    }
}

// ── TTL 缓存 ─────────────────────────────────────────────────────

/// 基于过期时间的缓存
///
/// 线程安全，默认 TTL 5 分钟，最大容量 5000 条。
/// 容量满时淘汰剩余 TTL 最短的条目。
#[derive(Debug)]
pub struct TtlCache {
    entries: Mutex<IndexMap<String, TtlEntry>>,
    default_ttl_ms: u64,
    max_entries: usize,
    stats: Mutex<TtlCacheStats>,
}

impl TtlCache {
    pub fn new(default_ttl_ms: Option<u64>, max_entries: Option<usize>) -> Self {
        Self {
            entries: Mutex::new(IndexMap::new()),
            default_ttl_ms: default_ttl_ms.unwrap_or(5 * 60 * 1000),
            max_entries: max_entries.unwrap_or(5000),
            stats: Mutex::new(TtlCacheStats::default()),
        }
    }

    /// 读取缓存，过期条目会被自动清除并计入 miss
    pub fn get(&self, key: &str) -> Option<Value> {
        let mut entries = self.entries.lock();
        // 先检查是否过期，过期则立即移除
        if let Some(entry) = entries.get(key) {
            if entry.is_expired() {
                entries.shift_remove(key);
                let mut stats = self.stats.lock();
                stats.expirations += 1;
                stats.misses += 1;
                return None;
            }
        }
        // 未过期则更新访问信息并返回
        if let Some(entry) = entries.get_mut(key) {
            entry.access_count += 1;
            entry.accessed_at_ms = now_ms();
            let value = entry.value.clone();
            self.stats.lock().hits += 1;
            Some(value)
        } else {
            self.stats.lock().misses += 1;
            None
        }
    }

    /// 写入缓存，容量满时先尝试清理过期条目，仍不够则淘汰最早创建的条目
    pub fn set(&self, key: &str, value: Value, ttl_ms: Option<u64>) -> Value {
        let ttl = ttl_ms.unwrap_or(self.default_ttl_ms);
        let mut entries = self.entries.lock();

        // 容量已满且不是更新已有 key 时，需要腾出空间
        if entries.len() >= self.max_entries && !entries.contains_key(key) {
            // 优先批量清理过期条目（最多 10 个，避免单次清理耗时过长）
            let expired_keys: Vec<String> = entries
                .iter()
                .filter(|(_, e)| e.is_expired())
                .map(|(k, _)| k.clone())
                .take(10)
                .collect();

            if !expired_keys.is_empty() {
                for k in &expired_keys {
                    entries.shift_remove(k);
                }
                self.stats.lock().expirations += expired_keys.len() as u64;
            } else {
                // 没有过期条目可清理，则淘汰创建时间最早的条目
                if let Some((oldest_key, _)) = entries
                    .iter()
                    .min_by_key(|(_, e)| e.created_at_ms)
                    .map(|(k, e)| (k.clone(), e.clone()))
                {
                    entries.shift_remove(&oldest_key);
                    self.stats.lock().evictions += 1;
                }
            }
        }

        let now = now_ms();
        let entry = TtlEntry {
            key: key.to_string(),
            size: estimate_size(&value),
            value: value.clone(),
            created_at_ms: now,
            accessed_at_ms: now,
            access_count: 0,
            ttl_ms: ttl,
        };
        entries.insert(key.to_string(), entry);
        self.stats.lock().writes += 1;
        value
    }

    pub fn delete(&self, key: &str) -> bool {
        self.entries.lock().shift_remove(key).is_some()
    }

    /// 检查 key 是否存在且未过期
    pub fn has(&self, key: &str) -> bool {
        let entries = self.entries.lock();
        entries
            .get(key)
            .map(|e| !e.is_expired())
            .unwrap_or(false)
    }

    /// 缓存穿透保护：命中则返回缓存值，未命中则计算并以默认 TTL 写入
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

    /// 同 get_or_compute，但允许指定自定义 TTL
    pub fn get_or_compute_with_ttl<F>(&self, key: &str, ttl_ms: u64, compute_fn: F) -> Value
    where
        F: FnOnce() -> Value,
    {
        if let Some(value) = self.get(key) {
            return value;
        }
        let value = compute_fn();
        self.set(key, value.clone(), Some(ttl_ms));
        value
    }

    /// 查询指定 key 的剩余存活时间（毫秒）
    pub fn remaining_ttl(&self, key: &str) -> Option<u64> {
        self.entries.lock().get(key).map(|e| e.remaining_ttl())
    }

    /// 批量清理所有过期条目，返回清理数量。适合定时任务调用。
    pub fn cleanup_expired(&self) -> usize {
        let mut entries = self.entries.lock();
        let expired_keys: Vec<String> = entries
            .iter()
            .filter(|(_, e)| e.is_expired())
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired_keys.len();
        for k in &expired_keys {
            entries.shift_remove(k);
        }
        self.stats.lock().expirations += count as u64;
        count
    }

    pub fn clear(&self) {
        self.entries.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }

    pub fn stats(&self) -> TtlCacheStats {
        let entries = self.entries.lock();
        let mut stats = self.stats.lock().clone();
        stats.entry_count = entries.len();
        stats.size_bytes = entries.values().map(|e| e.size).sum();
        stats
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_miss_on_empty() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        assert!(cache.get("x").is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_set_and_get() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        cache.set("k", json!("v"), None);
        assert_eq!(cache.get("k"), Some(json!("v")));
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().writes, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_ttl_expiration() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        cache.set("ephemeral", json!("gone"), Some(1)); // 1ms TTL
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.get("ephemeral").is_none());
        let stats = cache.stats();
        assert_eq!(stats.expirations, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_has() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        assert!(!cache.has("k"));
        cache.set("k", json!(1), None);
        assert!(cache.has("k"));
    }

    #[test]
    fn test_has_expired() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        cache.set("k", json!(1), Some(1));
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(!cache.has("k"));
    }

    #[test]
    fn test_delete() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        cache.set("k", json!(1), None);
        assert!(cache.delete("k"));
        assert!(cache.get("k").is_none());
        assert!(!cache.delete("k"));
    }

    #[test]
    fn test_eviction_on_capacity() {
        let cache = TtlCache::new(Some(60_000), Some(3));
        cache.set("a", json!(1), None);
        std::thread::sleep(std::time::Duration::from_millis(1));
        cache.set("b", json!(2), None);
        std::thread::sleep(std::time::Duration::from_millis(1));
        cache.set("c", json!(3), None);
        // 容量满，插入 d 应淘汰最早创建的 a
        cache.set("d", json!(4), None);
        assert!(cache.get("a").is_none(), "a should have been evicted");
        assert_eq!(cache.get("d"), Some(json!(4)));
        assert!(cache.stats().evictions >= 1);
    }

    #[test]
    fn test_eviction_prefers_expired() {
        let cache = TtlCache::new(Some(60_000), Some(3));
        cache.set("short", json!(1), Some(1)); // 将很快过期
        cache.set("long1", json!(2), None);
        cache.set("long2", json!(3), None);
        std::thread::sleep(std::time::Duration::from_millis(10));
        // 插入新条目，应该清理过期的 short 而非淘汰 long1
        cache.set("new", json!(4), None);
        assert!(cache.get("long1").is_some(), "long1 should still exist");
        assert!(cache.get("long2").is_some(), "long2 should still exist");
        assert_eq!(cache.stats().expirations, 1);
    }

    #[test]
    fn test_get_or_compute() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        let v1 = cache.get_or_compute("k", || json!(42));
        assert_eq!(v1, json!(42));
        let v2 = cache.get_or_compute("k", || json!(999));
        assert_eq!(v2, json!(42)); // 应命中缓存
    }

    #[test]
    fn test_get_or_compute_with_ttl() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        let v = cache.get_or_compute_with_ttl("k", 1, || json!("fast"));
        assert_eq!(v, json!("fast"));
        std::thread::sleep(std::time::Duration::from_millis(10));
        // 已过期，应重新计算
        let v2 = cache.get_or_compute_with_ttl("k", 60_000, || json!("new"));
        assert_eq!(v2, json!("new"));
    }

    #[test]
    fn test_remaining_ttl() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        assert!(cache.remaining_ttl("missing").is_none());
        cache.set("k", json!(1), Some(10_000));
        let ttl = cache.remaining_ttl("k").unwrap();
        assert!(ttl > 0 && ttl <= 10_000);
    }

    #[test]
    fn test_cleanup_expired() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        cache.set("a", json!(1), Some(1));
        cache.set("b", json!(2), Some(1));
        cache.set("c", json!(3), Some(60_000));
        std::thread::sleep(std::time::Duration::from_millis(10));
        let cleaned = cache.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert_eq!(cache.len(), 1);
        assert!(cache.has("c"));
    }

    #[test]
    fn test_clear() {
        let cache = TtlCache::new(Some(60_000), Some(100));
        cache.set("a", json!(1), None);
        cache.set("b", json!(2), None);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_hit_rate() {
        let stats = TtlCacheStats {
            hits: 3,
            misses: 1,
            ..Default::default()
        };
        assert!((stats.hit_rate() - 0.75).abs() < f64::EPSILON);

        let empty = TtlCacheStats::default();
        assert!((empty.hit_rate() - 0.0).abs() < f64::EPSILON);
    }
}
