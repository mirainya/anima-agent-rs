//! 路由算法模块
//!
//! 提供负载均衡器使用的基础路由算法：
//! - 轮询（Round Robin）：按顺序循环选择目标
//! - 一致性哈希（Consistent Hashing）：相同 key 始终路由到同一目标

/// FNV-1a 哈希算法，用于一致性哈希路由
/// 选择 FNV-1a 是因为它计算快、分布均匀，适合短字符串的哈希场景
fn stable_hash(input: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// 轮询路由：返回下一个索引，到达末尾后从头开始
/// 当 `last_index` 为 None 时从 0 开始
pub fn round_robin_index(last_index: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    Some(match last_index {
        Some(index) => (index + 1) % len,
        None => 0,
    })
}

/// 一致性哈希路由：相同 key 在目标数量不变时始终映射到同一索引
pub fn constant_hashing_index(key: &str, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    Some((stable_hash(key) % len as u64) as usize)
}

#[cfg(test)]
mod tests {
    use super::{constant_hashing_index, round_robin_index};

    #[test]
    fn round_robin_handles_empty_input() {
        assert_eq!(round_robin_index(None, 0), None);
        assert_eq!(round_robin_index(Some(0), 0), None);
    }

    #[test]
    fn round_robin_cycles_indices() {
        let mut last = None;
        let mut observed = Vec::new();
        for _ in 0..5 {
            let next = round_robin_index(last, 3).unwrap();
            observed.push(next);
            last = Some(next);
        }
        assert_eq!(observed, vec![0, 1, 2, 0, 1]);
    }

    #[test]
    fn constant_hashing_is_stable_for_same_key() {
        let first = constant_hashing_index("session-1", 4).unwrap();
        let second = constant_hashing_index("session-1", 4).unwrap();
        assert_eq!(first, second);
    }
}
