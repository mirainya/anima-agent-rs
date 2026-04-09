use crate::messages::pairing::ensure_tool_result_pairing;
use crate::messages::types::InternalMsg;
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
                crate::transcript::model::ContentBlock::ToolUse { id, .. } => {
                    tool_uses.insert(id.clone());
                }
                crate::transcript::model::ContentBlock::ToolResult { tool_use_id, .. } => {
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
