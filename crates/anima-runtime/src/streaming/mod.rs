//! 流式 API 解析与流式工具执行
//!
//! 对标 claude-code-main 的 `claude.ts` streaming + `StreamingToolExecutor.ts`。

pub(crate) mod api_parser;
pub(crate) mod executor;
pub(crate) mod types;

pub use api_parser::parse_sse_event;
pub use executor::{
    consume_runtime_stream, consume_sse_stream, RuntimeStreamEvent, StreamAccumulator,
    StreamingFinalResult, StreamingToolExecutor,
};
pub use types::{ContentBlock, ContentDelta, StreamEvent, TrackedToolState};
