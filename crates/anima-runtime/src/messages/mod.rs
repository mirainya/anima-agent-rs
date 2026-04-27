//! 消息协议三层映射
//!
//! 对标 claude-code-main 的 `messages.ts`（normalize + lookup + pairing）。

pub mod compact;
pub(crate) mod lookup;
pub(crate) mod normalize;
pub(crate) mod pairing;
pub mod types;

pub use compact::{
    clear_old_tool_results, compact_if_needed, estimate_msg_tokens, estimate_total_tokens,
    filter_oldest_messages, CompactConfig, CompactResult,
};
pub use lookup::{build_message_lookups, MessageLookups};
pub use normalize::normalize_messages_for_api;
pub use pairing::ensure_tool_result_pairing;
pub use types::{
    blocks_from_value, value_from_blocks, ApiMsg, ContentBlock, InternalMsg, MessageRole, SdkMsg,
};
