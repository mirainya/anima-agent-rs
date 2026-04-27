use crate::messages::types::{ContentBlock, InternalMsg, MessageRole};
use crate::streaming::types::{ContentBlock as StreamContentBlock, ContentDelta};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageRecord {
    pub message_id: String,
    pub run_id: String,
    pub turn_id: Option<String>,
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    pub tool_use_id: Option<String>,
    pub metadata: Value,
    pub filtered: bool,
    pub appended_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptInvariantViolation {
    MissingToolResult,
    DuplicateToolResult,
}

impl MessageRecord {
    pub fn from_internal(
        run_id: String,
        turn_id: Option<String>,
        appended_at_ms: u64,
        msg: &InternalMsg,
    ) -> Self {
        Self {
            message_id: msg.message_id.clone(),
            run_id,
            turn_id,
            role: msg.role.clone(),
            blocks: msg.blocks.clone(),
            tool_use_id: msg.tool_use_id.clone(),
            metadata: msg.metadata.clone(),
            filtered: msg.filtered,
            appended_at_ms,
        }
    }

    pub fn to_internal(&self) -> InternalMsg {
        InternalMsg {
            role: self.role.clone(),
            blocks: self.blocks.clone(),
            message_id: self.message_id.clone(),
            tool_use_id: self.tool_use_id.clone(),
            filtered: self.filtered,
            metadata: self.metadata.clone(),
        }
    }
}

pub fn stream_block_to_transcript(block: &StreamContentBlock) -> ContentBlock {
    match block {
        StreamContentBlock::Text { text } => ContentBlock::Text { text: text.clone() },
        StreamContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        StreamContentBlock::Thinking { thinking } => ContentBlock::Thinking {
            thinking: thinking.clone(),
        },
    }
}

pub fn apply_delta(block: &mut ContentBlock, delta: &ContentDelta) {
    match (block, delta) {
        (ContentBlock::Text { text }, ContentDelta::TextDelta { text: dt }) => {
            text.push_str(dt);
        }
        (ContentBlock::ToolUse { input, .. }, ContentDelta::InputJsonDelta { partial_json }) => {
            match input {
                Value::String(s) => s.push_str(partial_json),
                _ => *input = Value::String(partial_json.clone()),
            }
        }
        (ContentBlock::Thinking { thinking }, ContentDelta::ThinkingDelta { thinking: dt }) => {
            thinking.push_str(dt);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_internal_round_trip() {
        let msg = InternalMsg {
            role: MessageRole::User,
            blocks: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            message_id: "m1".into(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        };
        let record = MessageRecord::from_internal("r1".into(), None, 100, &msg);
        let back = record.to_internal();
        assert_eq!(back.role, msg.role);
        assert_eq!(back.blocks, msg.blocks);
        assert_eq!(back.message_id, msg.message_id);
    }

    #[test]
    fn apply_text_delta() {
        let mut block = ContentBlock::Text {
            text: "hel".into(),
        };
        apply_delta(
            &mut block,
            &ContentDelta::TextDelta {
                text: "lo".into(),
            },
        );
        assert_eq!(
            block,
            ContentBlock::Text {
                text: "hello".into()
            }
        );
    }

    #[test]
    fn apply_input_json_delta() {
        let mut block = ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: Value::String("{\"cmd\":".into()),
        };
        apply_delta(
            &mut block,
            &ContentDelta::InputJsonDelta {
                partial_json: "\"ls\"}".into(),
            },
        );
        if let ContentBlock::ToolUse { input, .. } = &block {
            assert_eq!(input, &Value::String("{\"cmd\":\"ls\"}".into()));
        } else {
            panic!("expected ToolUse");
        }
    }

    #[test]
    fn apply_input_json_delta_from_non_string() {
        let mut block = ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: Value::Null,
        };
        apply_delta(
            &mut block,
            &ContentDelta::InputJsonDelta {
                partial_json: "{\"a\":1}".into(),
            },
        );
        if let ContentBlock::ToolUse { input, .. } = &block {
            assert_eq!(input, &Value::String("{\"a\":1}".into()));
        } else {
            panic!("expected ToolUse");
        }
    }

    #[test]
    fn stream_block_to_transcript_maps_correctly() {
        let text = StreamContentBlock::Text {
            text: "hi".into(),
        };
        assert_eq!(
            stream_block_to_transcript(&text),
            ContentBlock::Text {
                text: "hi".into()
            }
        );

        let tool = StreamContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: json!({}),
        };
        assert!(
            matches!(stream_block_to_transcript(&tool), ContentBlock::ToolUse { id, .. } if id == "t1")
        );
    }
}
