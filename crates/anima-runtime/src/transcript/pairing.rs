use crate::messages::pairing::ensure_tool_result_pairing;
use crate::messages::types::{ContentBlock, InternalMsg};
use crate::transcript::model::{MessageRecord, TranscriptInvariantViolation};
use std::collections::HashSet;

pub fn ensure_pairing(messages: &mut Vec<InternalMsg>) {
    ensure_tool_result_pairing(messages);
}

pub fn validate_pairing(messages: &[MessageRecord]) -> Vec<TranscriptInvariantViolation> {
    let mut tool_uses = HashSet::new();
    let mut tool_results = HashSet::new();
    let mut violations = Vec::new();

    for message in messages {
        for block in &message.blocks {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    tool_uses.insert(id.clone());
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    if !tool_results.insert(tool_use_id.clone()) {
                        violations.push(TranscriptInvariantViolation::DuplicateToolResult);
                    }
                }
                _ => {}
            }
        }
    }

    for tool_use_id in tool_uses {
        if !tool_results.contains(&tool_use_id) {
            violations.push(TranscriptInvariantViolation::MissingToolResult);
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::types::MessageRole;
    use serde_json::json;

    fn msg(blocks: Vec<ContentBlock>) -> MessageRecord {
        MessageRecord {
            message_id: "m".into(),
            run_id: "r".into(),
            turn_id: None,
            role: MessageRole::Assistant,
            blocks,
            tool_use_id: None,
            metadata: json!({}),
            filtered: false,
            appended_at_ms: 0,
        }
    }

    #[test]
    fn valid_pairing_no_violations() {
        let messages = vec![
            msg(vec![ContentBlock::ToolUse {
                id: "tu1".into(), name: "bash".into(), input: json!({}),
            }]),
            msg(vec![ContentBlock::ToolResult {
                tool_use_id: "tu1".into(), content: json!("ok"), is_error: false,
            }]),
        ];
        assert!(validate_pairing(&messages).is_empty());
    }

    #[test]
    fn missing_tool_result() {
        let messages = vec![
            msg(vec![ContentBlock::ToolUse {
                id: "tu1".into(), name: "bash".into(), input: json!({}),
            }]),
        ];
        let v = validate_pairing(&messages);
        assert_eq!(v, vec![TranscriptInvariantViolation::MissingToolResult]);
    }

    #[test]
    fn duplicate_tool_result() {
        let messages = vec![
            msg(vec![ContentBlock::ToolUse {
                id: "tu1".into(), name: "bash".into(), input: json!({}),
            }]),
            msg(vec![ContentBlock::ToolResult {
                tool_use_id: "tu1".into(), content: json!("ok"), is_error: false,
            }]),
            msg(vec![ContentBlock::ToolResult {
                tool_use_id: "tu1".into(), content: json!("dup"), is_error: false,
            }]),
        ];
        let v = validate_pairing(&messages);
        assert!(v.contains(&TranscriptInvariantViolation::DuplicateToolResult));
    }
}
