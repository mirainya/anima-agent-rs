//! 钩子执行器

use std::fmt::Debug;

use super::types::{HookEvent, HookResult};

/// 钩子处理器 trait
pub trait HookHandler: Send + Sync + Debug {
    /// 处理钩子事件
    fn handle(&self, event: &HookEvent) -> HookResult;
}

/// 日志钩子：将事件输出到日志
#[derive(Debug)]
pub struct LoggingHook {
    pub prefix: String,
}

impl HookHandler for LoggingHook {
    fn handle(&self, event: &HookEvent) -> HookResult {
        match event {
            HookEvent::PreToolUse { tool_name, .. } => {
                tracing::debug!(prefix = %self.prefix, tool = %tool_name, "pre-tool-use");
            }
            HookEvent::PostToolUse { tool_name, .. } => {
                tracing::debug!(prefix = %self.prefix, tool = %tool_name, "post-tool-use");
            }
            HookEvent::PreSendMessage { content } => {
                tracing::debug!(
                    prefix = %self.prefix,
                    preview = &content[..content.len().min(50)],
                    "pre-send"
                );
            }
            HookEvent::PostSendMessage { .. } => {
                tracing::debug!(prefix = %self.prefix, "post-send");
            }
        }
        HookResult::Continue
    }
}
