//! 消息查找表
//!
//! 对标 claude-code-main 的 `build_message_lookups()`。

use std::collections::HashMap;

use super::types::InternalMsg;

/// 消息查找表：提供按 tool_use_id 的快速检索
#[derive(Debug, Default)]
pub struct MessageLookups {
    /// tool_use_id → 对应的 tool_result 消息索引
    pub tool_result_by_use_id: HashMap<String, usize>,
    /// tool_use_id → tool_use 消息索引
    pub tool_use_by_id: HashMap<String, usize>,
}

/// 构建消息查找表
pub fn build_message_lookups(messages: &[InternalMsg]) -> MessageLookups {
    let mut lookups = MessageLookups::default();

    for (i, msg) in messages.iter().enumerate() {
        if let Some(ref tool_use_id) = msg.tool_use_id {
            // 如果是 assistant 角色，视为 tool_use
            if msg.role == super::types::MessageRole::Assistant {
                lookups.tool_use_by_id.insert(tool_use_id.clone(), i);
            } else {
                // 其他角色带 tool_use_id 的视为 tool_result
                lookups.tool_result_by_use_id.insert(tool_use_id.clone(), i);
            }
        }
    }

    lookups
}

#[cfg(test)]
mod tests {
    use super::super::types::{InternalMsg, MessageRole};
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_lookup_correctly() {
        let msgs = vec![
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
        let lookups = build_message_lookups(&msgs);
        assert_eq!(lookups.tool_use_by_id.get("tu_1"), Some(&0));
        assert_eq!(lookups.tool_result_by_use_id.get("tu_1"), Some(&1));
    }
}
