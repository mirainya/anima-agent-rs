pub(crate) mod append;
pub(crate) mod model;
pub(crate) mod normalizer;
pub(crate) mod pairing;

pub use append::append_internal_message;
pub use model::{
    apply_delta, stream_block_to_transcript, MessageRecord, TranscriptInvariantViolation,
};
pub use normalizer::normalize_transcript_messages;
pub use pairing::{ensure_pairing, validate_pairing};
