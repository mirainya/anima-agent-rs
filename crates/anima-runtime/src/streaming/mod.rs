//! 流式 API 解析与流式工具执行
//!
//! 对标 claude-code-main 的 `claude.ts` streaming + `StreamingToolExecutor.ts`。

pub mod api_parser;
pub mod executor;
pub mod types;

pub use api_parser::*;
pub use executor::*;
pub use types::*;
