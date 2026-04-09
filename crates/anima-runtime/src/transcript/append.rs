use crate::messages::types::InternalMsg;
use crate::transcript::model::MessageRecord;

pub fn append_internal_message(
    transcript: &mut Vec<MessageRecord>,
    run_id: String,
    turn_id: Option<String>,
    appended_at_ms: u64,
    message: &InternalMsg,
) -> usize {
    transcript.push(MessageRecord::from_internal(
        run_id,
        turn_id,
        appended_at_ms,
        message,
    ));
    transcript.len()
}
