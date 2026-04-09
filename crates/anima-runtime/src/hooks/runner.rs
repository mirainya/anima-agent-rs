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
                eprintln!("[{}] pre-tool-use: {}", self.prefix, tool_name);
            }
            HookEvent::PostToolUse { tool_name, .. } => {
                eprintln!("[{}] post-tool-use: {}", self.prefix, tool_name);
            }
            HookEvent::PreSendMessage { content } => {
                eprintln!(
                    "[{}] pre-send: {}...",
                    self.prefix,
                    &content[..content.len().min(50)]
                );
            }
            HookEvent::PostSendMessage { .. } => {
                eprintln!("[{}] post-send", self.prefix);
            }
        }
        HookResult::Continue
    }
}
