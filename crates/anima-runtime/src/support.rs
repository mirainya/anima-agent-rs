//! 运行时支撑模块
//!
//! 统一导出缓存、上下文、指标等基础设施，并提供通用工具函数。

pub use crate::cache::{
    CacheEntry, CacheStats, EvictionPolicy, LruCache, LruEvictionPolicy, TtlCache, TtlCacheStats,
    TtlEntry, make_api_cache_key, make_session_cache_key, make_task_cache_key,
};
pub use crate::context::{
    ContextEntry, ContextManager, ContextSnapshot, ContextStats, ContextStorage, ContextType,
    EntryOpts, FileStorage, ManagerStatus, MemoryStorage, StorageTier, TieredStorage,
    TieredStorageConfig, TieredStorageTrait, context_type, key_parts, key_prefix, matches_pattern,
};
pub use crate::metrics::{HistogramValue, MetricsCollector, MetricsSnapshot, SummaryValue};

use std::time::{SystemTime, UNIX_EPOCH};

/// 获取当前时间的毫秒级 Unix 时间戳
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
