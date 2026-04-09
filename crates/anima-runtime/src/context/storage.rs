//! 上下文存储模块
//!
//! 实现两级存储架构（L1 内存 + L2 文件），用于管理 Agent 运行时的上下文数据。
//! - L1（`MemoryStorage`）：基于 LRU 淘汰策略的内存存储，访问速度快
//! - L2（`FileStorage`）：基于文件系统的持久化存储，容量大但速度慢
//! - `TieredStorage`：组合 L1/L2，自动根据访问频率进行数据升降级（promote/demote）
//!
//! 所有条目支持 TTL 过期机制。

use crate::support::now_ms;
use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{json, Value};

// ── Context Entry ───────────────────────────────────────────────────

/// 上下文条目，存储键值对及其元数据（层级、访问统计、TTL 等）
#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub key: String,
    pub value: Value,
    pub tier: StorageTier,
    pub created_at_ms: u64,
    pub accessed_at_ms: u64,
    pub access_count: u64,
    pub size: usize,
    pub ttl_ms: Option<u64>,
}

impl ContextEntry {
    /// 判断条目是否已过期（基于 TTL）
    pub fn is_expired(&self) -> bool {
        match self.ttl_ms {
            Some(ttl) => now_ms().saturating_sub(self.created_at_ms) > ttl,
            None => false,
        }
    }
}

/// 存储层级标识
#[derive(Debug, Clone, PartialEq)]
pub enum StorageTier {
    /// L1 内存层
    L1,
    /// L2 文件层
    L2,
}

// ── Storage Trait ───────────────────────────────────────────────────

/// 上下文存储的统一接口，L1 和 L2 都实现此 trait
pub trait ContextStorage: Send + Sync {
    fn get_entry(&self, key: &str) -> Option<ContextEntry>;
    fn set_entry(&self, key: &str, value: Value, opts: EntryOpts) -> ContextEntry;
    fn delete_entry(&self, key: &str) -> bool;
    fn has_key(&self, key: &str) -> bool;
    fn keys_matching(&self, prefix: &str) -> Vec<String>;
    fn clear(&self);
    fn entry_count(&self) -> usize;
}

/// 条目写入选项
#[derive(Debug, Clone, Default)]
pub struct EntryOpts {
    pub ttl_ms: Option<u64>,
    pub tier: Option<StorageTier>,
}

// ── L1 Memory Storage ───────────────────────────────────────────────

/// L1 内存存储，基于 LRU 淘汰策略
///
/// 使用 IndexMap 保持插入顺序，配合 lru_order 向量实现 LRU 追踪。
/// 达到容量上限时自动淘汰最久未访问的条目。
pub struct MemoryStorage {
    entries: Mutex<IndexMap<String, ContextEntry>>,
    lru_order: Mutex<Vec<String>>,
    max_entries: usize,
}

impl MemoryStorage {
    pub fn new(max_entries: Option<usize>) -> Self {
        Self {
            entries: Mutex::new(IndexMap::new()),
            lru_order: Mutex::new(Vec::new()),
            max_entries: max_entries.unwrap_or(10_000),
        }
    }

    /// 将 key 移到 LRU 队列末尾（标记为最近访问）
    fn touch_lru(&self, key: &str) {
        let mut order = self.lru_order.lock();
        order.retain(|k| k != key);
        order.push(key.to_string());
    }

    /// 淘汰 LRU 队列头部（最久未访问）的条目
    fn evict_lru(&self) {
        let mut entries = self.entries.lock();
        let mut order = self.lru_order.lock();
        if let Some(evict_key) = order.first().cloned() {
            entries.shift_remove(&evict_key);
            order.remove(0);
        }
    }
}

impl ContextStorage for MemoryStorage {
    fn get_entry(&self, key: &str) -> Option<ContextEntry> {
        let mut entries = self.entries.lock();
        let expired = entries.get(key).map(|e| e.is_expired()).unwrap_or(false);
        if expired {
            entries.shift_remove(key);
            self.lru_order.lock().retain(|k| k != key);
            return None;
        }
        if let Some(entry) = entries.get_mut(key) {
            entry.access_count += 1;
            entry.accessed_at_ms = now_ms();
            let result = entry.clone();
            drop(entries);
            self.touch_lru(key);
            Some(result)
        } else {
            None
        }
    }

    fn set_entry(&self, key: &str, value: Value, opts: EntryOpts) -> ContextEntry {
        let mut entries = self.entries.lock();
        if entries.len() >= self.max_entries && !entries.contains_key(key) {
            drop(entries);
            self.evict_lru();
            entries = self.entries.lock();
        }
        let now = now_ms();
        let entry = ContextEntry {
            key: key.to_string(),
            size: estimate_size(&value),
            value,
            tier: StorageTier::L1,
            created_at_ms: now,
            accessed_at_ms: now,
            access_count: 0,
            ttl_ms: opts.ttl_ms,
        };
        entries.insert(key.to_string(), entry.clone());
        drop(entries);
        self.touch_lru(key);
        entry
    }

    fn delete_entry(&self, key: &str) -> bool {
        let deleted = self.entries.lock().shift_remove(key).is_some();
        if deleted {
            self.lru_order.lock().retain(|k| k != key);
        }
        deleted
    }

    fn has_key(&self, key: &str) -> bool {
        let entries = self.entries.lock();
        entries.get(key).map(|e| !e.is_expired()).unwrap_or(false)
    }

    fn keys_matching(&self, prefix: &str) -> Vec<String> {
        self.entries
            .lock()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn clear(&self) {
        self.entries.lock().clear();
        self.lru_order.lock().clear();
    }

    fn entry_count(&self) -> usize {
        self.entries.lock().len()
    }
}

// ── L2 File Storage ─────────────────────────────────────────────────

/// L2 文件存储，将上下文条目持久化为 JSON 文件
///
/// 每个 key 对应一个 JSON 文件，文件名通过特殊字符转义避免路径冲突。
/// 维护内存中的元数据缓存以加速查询。
pub struct FileStorage {
    base_path: String,
    metadata_cache: Mutex<IndexMap<String, ContextEntry>>,
}

impl FileStorage {
    pub fn new(base_path: impl Into<String>) -> Self {
        let path = base_path.into();
        let _ = std::fs::create_dir_all(&path);
        Self {
            base_path: path,
            metadata_cache: Mutex::new(IndexMap::new()),
        }
    }

    /// 将 key 中的特殊字符转义为安全的文件名
    fn key_to_filename(key: &str) -> String {
        key.replace('/', "_SLASH_")
            .replace('\\', "_BSLASH_")
            .replace(':', "_COLON_")
    }

    fn filename_to_key(filename: &str) -> String {
        filename
            .replace("_SLASH_", "/")
            .replace("_BSLASH_", "\\")
            .replace("_COLON_", ":")
    }

    fn file_path(&self, key: &str) -> String {
        format!("{}/{}.json", self.base_path, Self::key_to_filename(key))
    }
}

impl ContextStorage for FileStorage {
    fn get_entry(&self, key: &str) -> Option<ContextEntry> {
        let path = self.file_path(key);
        let content = std::fs::read_to_string(&path).ok()?;
        let stored: Value = serde_json::from_str(&content).ok()?;

        let entry = ContextEntry {
            key: stored.get("key")?.as_str()?.to_string(),
            value: stored.get("value")?.clone(),
            tier: StorageTier::L2,
            created_at_ms: stored.get("created_at_ms")?.as_u64()?,
            accessed_at_ms: now_ms(),
            access_count: stored.get("access_count")?.as_u64().unwrap_or(0) + 1,
            size: stored.get("size")?.as_u64()? as usize,
            ttl_ms: stored.get("ttl_ms").and_then(Value::as_u64),
        };

        if entry.is_expired() {
            let _ = std::fs::remove_file(&path);
            return None;
        }

        self.metadata_cache
            .lock()
            .insert(key.to_string(), entry.clone());
        Some(entry)
    }

    fn set_entry(&self, key: &str, value: Value, opts: EntryOpts) -> ContextEntry {
        let now = now_ms();
        let entry = ContextEntry {
            key: key.to_string(),
            size: estimate_size(&value),
            value: value.clone(),
            tier: StorageTier::L2,
            created_at_ms: now,
            accessed_at_ms: now,
            access_count: 0,
            ttl_ms: opts.ttl_ms,
        };

        let stored = json!({
            "key": key,
            "value": value,
            "created_at_ms": now,
            "access_count": 0,
            "size": entry.size,
            "ttl_ms": opts.ttl_ms,
        });

        let _ = std::fs::create_dir_all(&self.base_path);
        let path = self.file_path(key);
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap());

        self.metadata_cache
            .lock()
            .insert(key.to_string(), entry.clone());
        entry
    }

    fn delete_entry(&self, key: &str) -> bool {
        let path = self.file_path(key);
        self.metadata_cache.lock().shift_remove(key);
        std::fs::remove_file(&path).is_ok()
    }

    fn has_key(&self, key: &str) -> bool {
        let path = self.file_path(key);
        if !std::path::Path::new(&path).exists() {
            return false;
        }
        // Check TTL
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(stored) = serde_json::from_str::<Value>(&content) {
                if let Some(ttl) = stored.get("ttl_ms").and_then(Value::as_u64) {
                    if let Some(created) = stored.get("created_at_ms").and_then(Value::as_u64) {
                        if crate::support::now_ms().saturating_sub(created) > ttl {
                            let _ = std::fs::remove_file(&path);
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn keys_matching(&self, prefix: &str) -> Vec<String> {
        let dir = std::path::Path::new(&self.base_path);
        if !dir.exists() {
            return Vec::new();
        }
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".json") {
                    let key = Self::filename_to_key(&name[..name.len() - 5]);
                    if key.starts_with(prefix) {
                        Some(key)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn clear(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.base_path) {
            for entry in entries.flatten() {
                if entry
                    .path()
                    .extension()
                    .map(|e| e == "json")
                    .unwrap_or(false)
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        self.metadata_cache.lock().clear();
    }

    fn entry_count(&self) -> usize {
        std::fs::read_dir(&self.base_path)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext == "json")
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

// ── Tiered Storage Trait ────────────────────────────────────────────

/// 分层存储行为 trait，定义数据在 L1/L2 之间的升降级策略
pub trait TieredStorageTrait: Send + Sync {
    /// 将数据从 L2 提升到 L1（热数据升级）
    fn promote(&self, key: &str) -> bool;
    /// 将数据从 L1 降级到 L2（冷数据降级）
    fn demote(&self, key: &str) -> bool;
    fn get_tier(&self, key: &str) -> Option<StorageTier>;
    fn should_promote(&self, entry: &ContextEntry) -> bool;
    fn should_demote(&self, entry: &ContextEntry) -> bool;
}

// ── Tiered Storage ──────────────────────────────────────────────────

/// 两级存储实现，组合 L1 内存存储和可选的 L2 文件存储
///
/// 读取时先查 L1 再查 L2，写入时同时写入两层。
/// 根据访问频率自动升级（promote）热数据到 L1，
/// 根据空闲时间自动降级（demote）冷数据到 L2。
pub struct TieredStorage {
    pub l1: MemoryStorage,
    pub l2: Option<FileStorage>,
    promotion_threshold: u64,
    demotion_threshold_ms: u64,
    promotions: Mutex<u64>,
    demotions: Mutex<u64>,
}

/// 分层存储配置
#[derive(Debug, Clone, Default)]
pub struct TieredStorageConfig {
    pub l1_max_entries: Option<usize>,
    pub l2_base_path: Option<String>,
    pub enable_l2: Option<bool>,
    pub promotion_threshold: Option<u64>,
    pub demotion_threshold_ms: Option<u64>,
}

impl TieredStorage {
    pub fn new(config: TieredStorageConfig) -> Self {
        let enable_l2 = config.enable_l2.unwrap_or(true);
        Self {
            l1: MemoryStorage::new(config.l1_max_entries),
            l2: if enable_l2 {
                Some(FileStorage::new(
                    config
                        .l2_base_path
                        .unwrap_or_else(|| ".opencode/context".into()),
                ))
            } else {
                None
            },
            promotion_threshold: config.promotion_threshold.unwrap_or(5),
            demotion_threshold_ms: config.demotion_threshold_ms.unwrap_or(30 * 60 * 1000),
            promotions: Mutex::new(0),
            demotions: Mutex::new(0),
        }
    }

    /// Get from L1 first, then L2. Promote from L2→L1 if access count exceeds threshold.
    pub fn get(&self, key: &str) -> Option<Value> {
        // Try L1
        if let Some(entry) = self.l1.get_entry(key) {
            return Some(entry.value);
        }
        // Try L2
        if let Some(ref l2) = self.l2 {
            if let Some(entry) = l2.get_entry(key) {
                // Promote to L1 if accessed enough
                if entry.access_count >= self.promotion_threshold {
                    self.l1.set_entry(
                        key,
                        entry.value.clone(),
                        EntryOpts {
                            ttl_ms: entry.ttl_ms,
                            ..Default::default()
                        },
                    );
                    *self.promotions.lock() += 1;
                }
                return Some(entry.value);
            }
        }
        None
    }

    /// Set in L1 (primary). Optionally also persist to L2.
    pub fn set(&self, key: &str, value: Value, opts: EntryOpts) -> ContextEntry {
        let entry = self.l1.set_entry(key, value.clone(), opts.clone());
        // Also persist to L2
        if let Some(ref l2) = self.l2 {
            l2.set_entry(key, value, opts);
        }
        entry
    }

    pub fn delete(&self, key: &str) -> bool {
        let l1_deleted = self.l1.delete_entry(key);
        let l2_deleted = self
            .l2
            .as_ref()
            .map(|l2| l2.delete_entry(key))
            .unwrap_or(false);
        l1_deleted || l2_deleted
    }

    /// Demote cold entries from L1 to L2.
    /// 扫描 L1 中的冷数据并批量降级到 L2
    pub fn demote_cold_entries(&self) -> usize {
        let threshold_ms = self.demotion_threshold_ms;
        let now = now_ms();
        let cold_keys: Vec<String> = {
            let entries = self.l1.entries.lock();
            entries
                .iter()
                .filter(|(_, e)| now.saturating_sub(e.accessed_at_ms) > threshold_ms)
                .map(|(k, _)| k.clone())
                .collect()
        };

        let mut count = 0;
        for key in &cold_keys {
            if let Some(entry) = self.l1.get_entry(key) {
                if let Some(ref l2) = self.l2 {
                    l2.set_entry(
                        key,
                        entry.value,
                        EntryOpts {
                            ttl_ms: entry.ttl_ms,
                            ..Default::default()
                        },
                    );
                }
                self.l1.delete_entry(key);
                count += 1;
            }
        }
        *self.demotions.lock() += count as u64;
        count
    }

    pub fn l1_count(&self) -> usize {
        self.l1.entry_count()
    }

    pub fn l2_count(&self) -> usize {
        self.l2.as_ref().map(|l2| l2.entry_count()).unwrap_or(0)
    }

    pub fn promotions(&self) -> u64 {
        *self.promotions.lock()
    }

    pub fn demotions(&self) -> u64 {
        *self.demotions.lock()
    }
}

impl TieredStorageTrait for TieredStorage {
    fn promote(&self, key: &str) -> bool {
        let l2 = match &self.l2 {
            Some(l2) => l2,
            None => return false,
        };
        let entry = match l2.get_entry(key) {
            Some(e) => e,
            None => return false,
        };
        self.l1.set_entry(
            key,
            entry.value,
            EntryOpts {
                ttl_ms: entry.ttl_ms,
                ..Default::default()
            },
        );
        l2.delete_entry(key);
        *self.promotions.lock() += 1;
        true
    }

    fn demote(&self, key: &str) -> bool {
        let l2 = match &self.l2 {
            Some(l2) => l2,
            None => return false,
        };
        let entry = match self.l1.get_entry(key) {
            Some(e) => e,
            None => return false,
        };
        l2.set_entry(
            key,
            entry.value,
            EntryOpts {
                ttl_ms: entry.ttl_ms,
                tier: Some(StorageTier::L2),
            },
        );
        self.l1.delete_entry(key);
        *self.demotions.lock() += 1;
        true
    }

    fn get_tier(&self, key: &str) -> Option<StorageTier> {
        if self.l1.has_key(key) {
            return Some(StorageTier::L1);
        }
        if let Some(ref l2) = self.l2 {
            if l2.has_key(key) {
                return Some(StorageTier::L2);
            }
        }
        None
    }

    fn should_promote(&self, entry: &ContextEntry) -> bool {
        entry.access_count >= self.promotion_threshold
    }

    fn should_demote(&self, entry: &ContextEntry) -> bool {
        let age = crate::support::now_ms().saturating_sub(entry.accessed_at_ms);
        age > self.demotion_threshold_ms
    }
}

/// 估算 JSON 值的内存占用大小（字节），用于存储统计
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
