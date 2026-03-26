//! 运行时支撑模块
//!
//! 统一导出缓存、上下文、指标等基础设施，并提供通用工具函数。

pub use crate::cache::*;
pub use crate::context::*;
pub use crate::metrics::*;

use std::time::{SystemTime, UNIX_EPOCH};

/// 获取当前时间的毫秒级 Unix 时间戳
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
