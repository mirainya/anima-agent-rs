//! 消息标准化
//!
//! 对标 claude-code-main 的 `normalize_messages_for_api()`：过滤/清洗/合并消息列表。

use super::types::{value_from_blocks, ApiMsg, InternalMsg, MessageRole};

/// 将内部消息列表标准化为 API 消息列表
///
/// 执行以下清洗操作：
/// 1. 过滤已标记为 filtered 的消息
/// 2. 合并连续的同角色消息
/// 3. 确保消息以 user 角色开头
pub fn normalize_messages_for_api(messages: &[InternalMsg]) -> Vec<ApiMsg> {
    let mut result: Vec<ApiMsg> = Vec::new();

    for msg in messages {
        if msg.filtered {
            continue;
        }

        let api_msg = ApiMsg {
            role: msg.role.clone(),
            content: value_from_blocks(&msg.blocks),
        };

        // 合并连续同角色消息
        if let Some(last) = result.last_mut() {
            if last.role == api_msg.role {
                merge_content(&mut last.content, &api_msg.content);
                continue;
            }
        }

        result.push(api_msg);
    }

    // 确保以 user 开头
    if result.first().map(|m| &m.role) != Some(&MessageRole::User) {
        result.insert(
            0,
            ApiMsg {
                role: MessageRole::User,
                content: serde_json::Value::String(String::new()),
            },
        );
    }

    result
}

/// 合并两个 content 值
fn merge_content(existing: &mut serde_json::Value, new: &serde_json::Value) {
    match (existing, new) {
        (serde_json::Value::String(ref mut a), serde_json::Value::String(b)) => {
            a.push('\n');
            a.push_str(b);
        }
        (existing, new) => {
            // 非字符串情况，转为数组
            let parts = vec![existing.clone(), new.clone()];
            *existing = serde_json::Value::Array(parts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::types::ContentBlock;
    use serde_json::json;

    #[test]
    fn filters_flagged_messages() {
        let msgs = vec![
            InternalMsg {
                role: MessageRole::User,
                blocks: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
                message_id: "1".into(),
                tool_use_id: None,
                filtered: false,
                metadata: json!({}),
            },
            InternalMsg {
                role: MessageRole::Assistant,
                blocks: vec![ContentBlock::Text {
                    text: "filtered".into(),
                }],
                message_id: "2".into(),
                tool_use_id: None,
                filtered: true,
                metadata: json!({}),
            },
        ];
        let result = normalize_messages_for_api(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, json!("hello"));
    }

    #[test]
    fn merges_consecutive_same_role() {
        let msgs = vec![
            InternalMsg {
                role: MessageRole::User,
                blocks: vec![ContentBlock::Text { text: "a".into() }],
                message_id: "1".into(),
                tool_use_id: None,
                filtered: false,
                metadata: json!({}),
            },
            InternalMsg {
                role: MessageRole::User,
                blocks: vec![ContentBlock::Text { text: "b".into() }],
                message_id: "2".into(),
                tool_use_id: None,
                filtered: false,
                metadata: json!({}),
            },
        ];
        let result = normalize_messages_for_api(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, json!("a\nb"));
    }
}
