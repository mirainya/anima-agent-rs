//! tool_use / tool_result 配对修复
//!
//! 对标 claude-code-main 的 `ensure_tool_result_pairing()`。
//! 确保每个 tool_use 都有对应的 tool_result，缺失时自动补全空结果。

use serde_json::json;

use super::lookup::build_message_lookups;
use super::types::{InternalMsg, MessageRole};

/// 确保 tool_use 和 tool_result 正确配对
///
/// 遍历消息列表，对于每个 tool_use（assistant 角色、有 tool_use_id）,
/// 如果找不到对应的 tool_result，则自动插入一个空 tool_result。
pub fn ensure_tool_result_pairing(messages: &mut Vec<InternalMsg>) {
    let lookups = build_message_lookups(messages);
    let mut inserts: Vec<(usize, InternalMsg)> = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == MessageRole::Assistant {
            if let Some(ref tool_use_id) = msg.tool_use_id {
                if !lookups.tool_result_by_use_id.contains_key(tool_use_id) {
                    // 缺失 tool_result，需要在 tool_use 后面插入
                    let placeholder = InternalMsg {
                        role: MessageRole::User,
                        content: json!([{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": ""
                        }]),
                        message_id: format!("{}_auto_result", msg.message_id),
                        tool_use_id: Some(tool_use_id.clone()),
                        filtered: false,
                        metadata: json!({"auto_generated": true}),
                    };
                    inserts.push((i + 1, placeholder));
                }
            }
        }
    }

    // 从后往前插入，避免索引偏移
    for (pos, msg) in inserts.into_iter().rev() {
        messages.insert(pos, msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inserts_missing_tool_result() {
        let mut msgs = vec![InternalMsg {
            role: MessageRole::Assistant,
            content: json!({}),
            message_id: "m1".into(),
            tool_use_id: Some("tu_1".into()),
            filtered: false,
            metadata: json!({}),
        }];

        ensure_tool_result_pairing(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, MessageRole::User);
        assert_eq!(msgs[1].tool_use_id.as_deref(), Some("tu_1"));
    }

    #[test]
    fn skips_when_paired() {
        let mut msgs = vec![
            InternalMsg {
                role: MessageRole::Assistant,
                content: json!({}),
                message_id: "m1".into(),
                tool_use_id: Some("tu_1".into()),
                filtered: false,
                metadata: json!({}),
            },
            InternalMsg {
                role: MessageRole::User,
                content: json!("result"),
                message_id: "m2".into(),
                tool_use_id: Some("tu_1".into()),
                filtered: false,
                metadata: json!({}),
            },
        ];

        ensure_tool_result_pairing(&mut msgs);
        assert_eq!(msgs.len(), 2); // 不变
    }
}
