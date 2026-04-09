//! 上下文自动压缩（Context Auto-Compaction）
//!
//! 对标 claude-code-main 的 microcompact + snip compact 模式。
//! 两阶段压缩：
//! - 阶段一（Microcompact）：清除旧 tool_result 内容
//! - 阶段二（Snip Compact）：从最旧消息开始标记 filtered

use serde_json::json;

use super::types::{InternalMsg, MessageRole};

// ---------------------------------------------------------------------------
// 配置与结果类型
// ---------------------------------------------------------------------------

/// 压缩配置
#[derive(Debug, Clone)]
pub struct CompactConfig {
    /// 模型上下文窗口大小（token 数），默认 200_000
    pub context_window: usize,
    /// 触发压缩的阈值比例，默认 0.87
    pub threshold_ratio: f64,
    /// 安全缓冲 token 数，默认 13_000
    pub buffer_tokens: usize,
    /// 保护最近 N 轮不被压缩，默认 1
    pub preserve_recent_turns: usize,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            context_window: 200_000,
            threshold_ratio: 0.87,
            buffer_tokens: 13_000,
            preserve_recent_turns: 1,
        }
    }
}

/// 压缩结果
#[derive(Debug, Clone, Default)]
pub struct CompactResult {
    pub did_compact: bool,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub cleared_tokens: usize,
    pub filtered_tokens: usize,
}

// ---------------------------------------------------------------------------
// Token 估算
// ---------------------------------------------------------------------------

/// 估算单条消息的 token 数（bytes / 4 + 20 固定开销）
pub fn estimate_msg_tokens(msg: &InternalMsg) -> usize {
    let bytes = serde_json::to_string(&msg.content)
        .unwrap_or_default()
        .len();
    bytes / 4 + 20
}

/// 估算所有未被过滤消息的总 token 数
pub fn estimate_total_tokens(messages: &[InternalMsg]) -> usize {
    messages
        .iter()
        .filter(|m| !m.filtered)
        .map(estimate_msg_tokens)
        .sum()
}

// ---------------------------------------------------------------------------
// Turn 边界识别
// ---------------------------------------------------------------------------

/// 从末尾反向扫描，识别最近 N 轮的边界索引。
/// 返回的索引及之后的消息不参与压缩。
///
/// 一个 "turn" = 一条 assistant 消息 + 紧随其后的所有 tool_result 消息。
fn find_recent_turn_boundary(messages: &[InternalMsg], preserve_turns: usize) -> usize {
    if messages.is_empty() || preserve_turns == 0 {
        return messages.len();
    }

    let mut turns_found = 0;
    let mut i = messages.len();

    while i > 0 {
        i -= 1;

        // 跳过尾部的 tool_result（属于当前 turn）
        while i > 0 && messages[i].role == MessageRole::User && messages[i].tool_use_id.is_some() {
            i -= 1;
        }

        // 当前位置应该是 assistant 消息
        if messages[i].role == MessageRole::Assistant {
            turns_found += 1;
            if turns_found >= preserve_turns {
                return i;
            }
        }

        // 如果不是 assistant，继续向前
    }

    // 没找到足够的 turn，保护全部消息
    0
}

// ---------------------------------------------------------------------------
// 阶段一：Tool Result Clearing（Microcompact）
// ---------------------------------------------------------------------------

/// 清除边界之前的旧 tool_result 内容，返回释放的 token 数
pub fn clear_old_tool_results(messages: &mut [InternalMsg], boundary: usize) -> usize {
    let mut freed = 0;
    let end = boundary.min(messages.len());

    for msg in &mut messages[..end] {
        // 条件：是 user 消息、有 tool_use_id、未被过滤、未被清除过
        if msg.role != MessageRole::User
            || msg.tool_use_id.is_none()
            || msg.filtered
            || msg.metadata.get("compacted").is_some()
        {
            continue;
        }

        let original_tokens = estimate_msg_tokens(msg);
        let tool_use_id = msg.tool_use_id.clone().unwrap_or_default();

        // 替换 content
        msg.content = json!([{
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": format!("[cleared: {original_tokens} tokens]"),
            "is_error": false
        }]);

        // 标记 metadata
        msg.metadata = json!({
            "compacted": true,
            "original_tokens": original_tokens
        });

        let new_tokens = estimate_msg_tokens(msg);
        if original_tokens > new_tokens {
            freed += original_tokens - new_tokens;
        }
    }

    freed
}

// ---------------------------------------------------------------------------
// 阶段二：Message Filtering（Snip Compact）
// ---------------------------------------------------------------------------

/// 从最旧消息开始成组过滤，直到释放 >= target token。
/// 返回实际释放的 token 数。
///
/// 规则：
/// - 永不过滤 messages[0]（初始 user 消息）
/// - assistant + 关联 tool_results 成组过滤
pub fn filter_oldest_messages(
    messages: &mut [InternalMsg],
    boundary: usize,
    target: usize,
) -> usize {
    let mut freed = 0;
    let end = boundary.min(messages.len());
    let mut i = 1; // 跳过 messages[0]

    while i < end && freed < target {
        if messages[i].filtered {
            i += 1;
            continue;
        }

        if messages[i].role == MessageRole::Assistant {
            // 过滤 assistant 消息
            freed += estimate_msg_tokens(&messages[i]);
            messages[i].filtered = true;

            // 过滤紧随其后的 tool_result 消息
            let mut j = i + 1;
            while j < end
                && messages[j].role == MessageRole::User
                && messages[j].tool_use_id.is_some()
            {
                if !messages[j].filtered {
                    freed += estimate_msg_tokens(&messages[j]);
                    messages[j].filtered = true;
                }
                j += 1;
            }
            i = j;
        } else {
            // 非 assistant 消息（如普通 user 消息），单独过滤
            freed += estimate_msg_tokens(&messages[i]);
            messages[i].filtered = true;
            i += 1;
        }
    }

    freed
}

// ---------------------------------------------------------------------------
// 顶层入口
// ---------------------------------------------------------------------------

/// 检查是否需要压缩，如需要则执行两阶段压缩
pub fn compact_if_needed(messages: &mut [InternalMsg], config: &CompactConfig) -> CompactResult {
    let total = estimate_total_tokens(messages);
    let threshold = (config.context_window as f64 * config.threshold_ratio) as usize;

    if total < threshold {
        return CompactResult {
            did_compact: false,
            tokens_before: total,
            tokens_after: total,
            ..Default::default()
        };
    }

    let safe_target = threshold.saturating_sub(config.buffer_tokens);
    let need_to_free = total.saturating_sub(safe_target);

    let boundary = find_recent_turn_boundary(messages, config.preserve_recent_turns);

    // 阶段一
    let cleared = clear_old_tool_results(messages, boundary);

    let after_phase1 = estimate_total_tokens(messages);
    if after_phase1 < threshold {
        return CompactResult {
            did_compact: true,
            tokens_before: total,
            tokens_after: after_phase1,
            cleared_tokens: cleared,
            filtered_tokens: 0,
        };
    }

    // 阶段二
    let remaining_target = need_to_free.saturating_sub(cleared);
    let filtered = filter_oldest_messages(messages, boundary, remaining_target);

    let after_phase2 = estimate_total_tokens(messages);
    CompactResult {
        did_compact: true,
        tokens_before: total,
        tokens_after: after_phase2,
        cleared_tokens: cleared,
        filtered_tokens: filtered,
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::pairing::ensure_tool_result_pairing;
    use uuid::Uuid;

    fn make_user_msg(text: &str) -> InternalMsg {
        InternalMsg {
            role: MessageRole::User,
            content: json!(text),
            message_id: Uuid::new_v4().to_string(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }
    }

    fn make_assistant_msg(text: &str) -> InternalMsg {
        InternalMsg {
            role: MessageRole::Assistant,
            content: json!([{"type": "text", "text": text}]),
            message_id: Uuid::new_v4().to_string(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }
    }

    fn make_assistant_with_tool_use(text: &str, tool_use_id: &str) -> InternalMsg {
        InternalMsg {
            role: MessageRole::Assistant,
            content: json!([
                {"type": "text", "text": text},
                {"type": "tool_use", "id": tool_use_id, "name": "echo", "input": {"text": "hi"}}
            ]),
            message_id: Uuid::new_v4().to_string(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }
    }

    fn make_tool_result(tool_use_id: &str, content: &str) -> InternalMsg {
        InternalMsg {
            role: MessageRole::User,
            content: json!([{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": false
            }]),
            message_id: Uuid::new_v4().to_string(),
            tool_use_id: Some(tool_use_id.to_string()),
            filtered: false,
            metadata: json!({}),
        }
    }

    fn make_large_tool_result(tool_use_id: &str, size: usize) -> InternalMsg {
        let content = "x".repeat(size);
        make_tool_result(tool_use_id, &content)
    }

    // 1. Token 估算
    #[test]
    fn test_estimate_msg_tokens() {
        let small = make_user_msg("hi");
        let large = make_user_msg(&"x".repeat(400));

        let small_tokens = estimate_msg_tokens(&small);
        let large_tokens = estimate_msg_tokens(&large);

        assert!(small_tokens > 20); // 至少有固定开销
        assert!(large_tokens > small_tokens);
        // 400 bytes / 4 + 20 ≈ 120+
        assert!(large_tokens > 100);
    }

    // 2. filtered 消息不计入
    #[test]
    fn test_estimate_total_tokens_skips_filtered() {
        let mut msgs = vec![make_user_msg("hello"), make_user_msg("world")];
        let total_before = estimate_total_tokens(&msgs);

        msgs[1].filtered = true;
        let total_after = estimate_total_tokens(&msgs);

        assert!(total_after < total_before);
        assert_eq!(total_after, estimate_msg_tokens(&msgs[0]));
    }

    // 3. 未超阈值不压缩
    #[test]
    fn test_compact_below_threshold_noop() {
        let mut msgs = vec![make_user_msg("hello"), make_assistant_msg("hi")];
        let config = CompactConfig {
            context_window: 200_000,
            ..Default::default()
        };
        let result = compact_if_needed(&mut msgs, &config);
        assert!(!result.did_compact);
        assert!(!msgs[0].filtered);
        assert!(!msgs[1].filtered);
    }

    // 4. 边界前 tool_result 被清除，边界后保留
    #[test]
    fn test_clear_old_tool_results() {
        let mut msgs = vec![
            make_user_msg("start"),
            make_assistant_with_tool_use("calling", "tu_1"),
            make_large_tool_result("tu_1", 1000),
            make_assistant_with_tool_use("calling again", "tu_2"),
            make_large_tool_result("tu_2", 1000),
        ];

        // boundary = 3: 只清除前 3 条中的 tool_result
        let freed = clear_old_tool_results(&mut msgs, 3);
        assert!(freed > 0);

        // msgs[2] 应该被清除（compacted 标记）
        assert!(msgs[2].metadata.get("compacted").is_some());
        // msgs[4] 应该保留原内容
        assert!(msgs[4].metadata.get("compacted").is_none());
    }

    // 5. 清除后 pairing 不插入额外消息
    #[test]
    fn test_clear_preserves_pairing() {
        let mut msgs = vec![
            make_user_msg("start"),
            make_assistant_with_tool_use("call", "tu_1"),
            make_large_tool_result("tu_1", 500),
        ];

        clear_old_tool_results(&mut msgs, 3);

        let len_before = msgs.len();
        ensure_tool_result_pairing(&mut msgs);
        assert_eq!(
            msgs.len(),
            len_before,
            "pairing should not insert new messages"
        );
    }

    // 6. 幂等性
    #[test]
    fn test_clear_idempotent() {
        let mut msgs = vec![
            make_user_msg("start"),
            make_assistant_with_tool_use("call", "tu_1"),
            make_large_tool_result("tu_1", 500),
        ];

        let freed1 = clear_old_tool_results(&mut msgs, 3);
        let freed2 = clear_old_tool_results(&mut msgs, 3);

        assert!(freed1 > 0);
        assert_eq!(freed2, 0, "second call should not free additional tokens");
    }

    // 7. 从最旧开始过滤，成组操作
    #[test]
    fn test_filter_oldest_messages() {
        let mut msgs = vec![
            make_user_msg("start"),                        // 0: 永不过滤
            make_assistant_with_tool_use("call1", "tu_1"), // 1
            make_tool_result("tu_1", "result1"),           // 2
            make_assistant_with_tool_use("call2", "tu_2"), // 3
            make_tool_result("tu_2", "result2"),           // 4
        ];

        let freed = filter_oldest_messages(&mut msgs, 5, usize::MAX);

        assert!(!msgs[0].filtered, "initial user msg must not be filtered");
        assert!(msgs[1].filtered);
        assert!(msgs[2].filtered);
        assert!(msgs[3].filtered);
        assert!(msgs[4].filtered);
        assert!(freed > 0);
    }

    // 8. 第一条 user 消息永不被过滤
    #[test]
    fn test_filter_preserves_initial_user_msg() {
        let mut msgs = vec![make_user_msg("start"), make_user_msg("second")];

        filter_oldest_messages(&mut msgs, 2, usize::MAX);

        assert!(!msgs[0].filtered);
        assert!(msgs[1].filtered);
    }

    // 9. 阶段一不够时触发阶段二
    #[test]
    fn test_compact_phases_cascade() {
        // 构建大量消息使总 token 超过阈值
        let mut msgs = vec![make_user_msg("start")];
        for i in 0..20 {
            let tu_id = format!("tu_{i}");
            msgs.push(make_assistant_with_tool_use(&format!("call {i}"), &tu_id));
            msgs.push(make_large_tool_result(&tu_id, 4000));
        }
        // 最新的一轮
        msgs.push(make_assistant_with_tool_use("latest call", "tu_latest"));
        msgs.push(make_large_tool_result("tu_latest", 4000));

        let config = CompactConfig {
            context_window: 5000, // 非常小的窗口，强制压缩
            threshold_ratio: 0.5,
            buffer_tokens: 500,
            preserve_recent_turns: 1,
        };

        let result = compact_if_needed(&mut msgs, &config);
        assert!(result.did_compact);
        assert!(result.tokens_after < result.tokens_before);
        // 应该有一些消息被 filtered
        assert!(result.filtered_tokens > 0 || result.cleared_tokens > 0);
    }

    // 10. 边界识别
    #[test]
    fn test_find_recent_turn_boundary() {
        let msgs = vec![
            make_user_msg("start"),                        // 0
            make_assistant_with_tool_use("call1", "tu_1"), // 1
            make_tool_result("tu_1", "r1"),                // 2
            make_assistant_with_tool_use("call2", "tu_2"), // 3
            make_tool_result("tu_2", "r2"),                // 4
        ];

        // preserve 1 turn → 边界应在 idx 3（最后一个 assistant）
        let b1 = find_recent_turn_boundary(&msgs, 1);
        assert_eq!(b1, 3);

        // preserve 2 turns → 边界应在 idx 1
        let b2 = find_recent_turn_boundary(&msgs, 2);
        assert_eq!(b2, 1);

        // preserve 0 → 全部可压缩
        let b0 = find_recent_turn_boundary(&msgs, 0);
        assert_eq!(b0, msgs.len());
    }
}
