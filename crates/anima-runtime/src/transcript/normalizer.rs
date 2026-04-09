use crate::messages::normalize::normalize_messages_for_api;
use crate::messages::types::{ApiMsg, InternalMsg};

pub fn normalize_transcript_messages(messages: &[InternalMsg]) -> Vec<ApiMsg> {
    normalize_messages_for_api(messages)
}
