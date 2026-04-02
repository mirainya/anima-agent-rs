//! 上下文管理器模块
//!
//! 提供高层级的上下文管理能力，封装键值存储、快照/恢复、会话历史等功能。
//! `ContextManager` 是 Agent 运行时的核心组件之一，负责维护对话上下文状态，
//! 并通过快照机制支持上下文的时间点回溯。内置 GC 自动清理过期快照。

use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;
use crate::support::now_ms;

/// 上下文快照，记录某个 key 在特定时间点的值
#[derive(Debug, Clone, PartialEq)]
pub struct ContextSnapshot {
    pub id: String,
    pub key: String,
    pub value: Value,
    pub source_tier: String,
    pub timestamp_ms: u64,
}

/// 上下文管理器的操作统计
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

/// 管理器运行状态
#[derive(Debug, Clone, PartialEq)]
pub struct ManagerStatus {
    pub id: String,
    pub status: String,
    pub stats: ContextStats,
}

/// 快照数量上限
const MAX_SNAPSHOTS: usize = 500;
/// 快照最大存活时间（1 小时）
const SNAPSHOT_MAX_AGE_MS: u64 = 3_600_000; // 1 hour

/// 上下文管理器，提供键值存储、快照回溯和会话历史管理
///
/// 线程安全，所有状态通过 Mutex/AtomicBool 保护。
/// gc_running 控制是否启用过期快照的自动清理。
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

    /// 获取或创建上下文值（惰性初始化模式）
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

    /// 为指定 key 创建快照，达到容量上限时优先淘汰过期快照，其次淘汰最旧快照
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

    /// 从快照恢复上下文值
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

    /// 向指定会话的历史记录追加一条消息（存储在 "session:{id}:history" 键下）
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

    /// 清除指定会话的所有上下文数据（匹配 "session:{id}*" 前缀的所有键）
    pub fn clear_session_context(&self, session_id: &str) {
        let pattern = format!("session:{session_id}*");
        for key in self.list_keys(&pattern) {
            let _ = self.delete_context(&key);
        }
    }

    /// 停止 GC 并清空所有上下文和快照
    pub fn close(&self) {
        self.gc_running.store(false, Ordering::SeqCst);
        self.contexts.lock().clear();
        self.snapshots.lock().clear();
        let mut stats = self.stats.lock();
        stats.entries = 0;
        stats.snapshots = 0;
    }

    /// 清理超过 SNAPSHOT_MAX_AGE_MS 的过期快照，返回清理数量
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_set_context() {
        let mgr = ContextManager::new(Some(false));
        assert!(mgr.get_context("k").is_none());
        mgr.set_context("k", json!("v"));
        assert_eq!(mgr.get_context("k"), Some(json!("v")));
        let stats = mgr.status().stats;
        assert_eq!(stats.reads, 1);
        assert_eq!(stats.writes, 1);
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn test_delete_context() {
        let mgr = ContextManager::new(Some(false));
        mgr.set_context("k", json!(1));
        assert!(mgr.delete_context("k"));
        assert!(mgr.get_context("k").is_none());
        assert!(!mgr.delete_context("k")); // 已删除
        assert_eq!(mgr.status().stats.deletes, 1);
    }

    #[test]
    fn test_get_or_create() {
        let mgr = ContextManager::new(Some(false));
        let v1 = mgr.get_or_create("k", || json!(42));
        assert_eq!(v1, json!(42));
        let v2 = mgr.get_or_create("k", || json!(999));
        assert_eq!(v2, json!(42)); // 命中
    }

    #[test]
    fn test_list_keys() {
        let mgr = ContextManager::new(Some(false));
        mgr.set_context("session:a:history", json!([]));
        mgr.set_context("session:a:state", json!({}));
        mgr.set_context("session:b:history", json!([]));
        mgr.set_context("other", json!(1));

        let keys = mgr.list_keys("session:a*");
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"session:a:history".into()));
        assert!(keys.contains(&"session:a:state".into()));
    }

    #[test]
    fn test_session_history() {
        let mgr = ContextManager::new(Some(false));
        assert!(mgr.get_session_history("s1").is_empty());
        mgr.add_to_session_history("s1", json!({"msg": "hello"}));
        mgr.add_to_session_history("s1", json!({"msg": "world"}));
        let history = mgr.get_session_history("s1");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["msg"], "hello");
    }

    #[test]
    fn test_session_history_non_array_recovery() {
        let mgr = ContextManager::new(Some(false));
        // 故意设置非数组值
        mgr.set_context("session:s1:history", json!("not_an_array"));
        mgr.add_to_session_history("s1", json!("entry1"));
        let history = mgr.get_session_history("s1");
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_clear_session_context() {
        let mgr = ContextManager::new(Some(false));
        mgr.set_context("session:s1:history", json!([]));
        mgr.set_context("session:s1:state", json!({}));
        mgr.set_context("session:s2:history", json!([]));
        mgr.clear_session_context("s1");
        assert!(mgr.get_context("session:s1:history").is_none());
        assert!(mgr.get_context("session:s1:state").is_none());
        assert!(mgr.get_context("session:s2:history").is_some()); // 不应受影响
    }

    #[test]
    fn test_snapshot_and_restore() {
        let mgr = ContextManager::new(Some(false));
        mgr.set_context("k", json!("original"));
        let snap = mgr.snapshot("k").unwrap();
        assert_eq!(snap.value, json!("original"));

        // 修改值
        mgr.set_context("k", json!("modified"));
        assert_eq!(mgr.get_context("k"), Some(json!("modified")));

        // 恢复快照
        let restored = mgr.restore(&snap.id).unwrap();
        assert_eq!(restored.value, json!("original"));
        assert_eq!(mgr.get_context("k"), Some(json!("original")));
    }

    #[test]
    fn test_snapshot_nonexistent_key() {
        let mgr = ContextManager::new(Some(false));
        assert!(mgr.snapshot("missing").is_none());
    }

    #[test]
    fn test_restore_nonexistent_snapshot() {
        let mgr = ContextManager::new(Some(false));
        assert!(mgr.restore("fake-id").is_none());
    }

    #[test]
    fn test_close() {
        let mgr = ContextManager::new(Some(true));
        mgr.set_context("k", json!(1));
        mgr.snapshot("k");
        mgr.close();
        assert!(mgr.get_context("k").is_none());
        assert_eq!(mgr.status().status, "stopped");
        assert_eq!(mgr.status().stats.entries, 0);
    }

    #[test]
    fn test_status() {
        let mgr = ContextManager::new(Some(true));
        let status = mgr.status();
        assert_eq!(status.status, "running");
        assert_eq!(status.stats.entries, 0);
    }
}
