//! 工具注册与执行闭环
//!
//! 对标 claude-code-main 的 `toolExecution.ts` + `StreamingToolExecutor.ts`。
//! 提供工具的注册、校验、权限检查、执行和结果收集能力。

pub mod builtins;
pub mod definition;
pub mod execution;
pub mod registry;
pub mod result;

pub use definition::{Tool, ToolContext};
pub use execution::{
    execute_tool_after_permission, run_tool_use, AwaitingToolPermission, RunToolOptions,
    RunToolUseOutcome, ToolInvocationPhase, ToolInvocationRecord, ToolLifecycleEventCallback,
    ToolPermissionState,
};
pub use registry::ToolRegistry;
pub use result::{ToolError, ToolResult, ToolResultBlock};
