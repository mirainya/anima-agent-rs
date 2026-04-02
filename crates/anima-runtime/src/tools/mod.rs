//! 工具注册与执行闭环
//!
//! 对标 claude-code-main 的 `toolExecution.ts` + `StreamingToolExecutor.ts`。
//! 提供工具的注册、校验、权限检查、执行和结果收集能力。

pub mod definition;
pub mod execution;
pub mod registry;
pub mod result;
pub mod builtins;

pub use definition::*;
pub use execution::*;
pub use registry::*;
pub use result::*;
