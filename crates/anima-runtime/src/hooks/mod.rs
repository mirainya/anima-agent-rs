//! Pre/Post 钩子机制
//!
//! 对标 claude-code-main 的 hook 系统。

pub(crate) mod registry;
pub(crate) mod runner;
pub(crate) mod stop_hook;
pub(crate) mod types;

pub use registry::HookRegistry;
pub use runner::{HookHandler, LoggingHook};
pub use stop_hook::StopHook;
pub use types::{HookConfig, HookEvent, HookResult, PostToolHookResult};
