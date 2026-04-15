//! 钩子注册中心

use std::sync::Arc;

use serde_json::Value;

use super::runner::HookHandler;
use super::types::{HookEvent, HookResult};

/// 钩子注册中心：管理 pre/post 钩子
#[derive(Debug, Default)]
pub struct HookRegistry {
    pre_hooks: Vec<Arc<dyn HookHandler>>,
    post_hooks: Vec<Arc<dyn HookHandler>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册 pre-hook
    pub fn register_pre_hook(&mut self, handler: Arc<dyn HookHandler>) {
        self.pre_hooks.push(handler);
    }

    /// 注册 post-hook
    pub fn register_post_hook(&mut self, handler: Arc<dyn HookHandler>) {
        self.post_hooks.push(handler);
    }

    /// 执行所有 pre-hooks（保持 side-effect 语义）
    pub fn run_pre_hooks(&self, event: &HookEvent) {
        for hook in &self.pre_hooks {
            let _ = hook.handle(event);
        }
    }

    /// 执行 pre-tool hooks，并聚合 Continue / Transform / Block 结果
    pub fn run_pre_tool_hooks(&self, tool_name: &str, input: &Value) -> HookResult {
        let mut effective_input = input.clone();
        let mut transformed = false;

        for hook in &self.pre_hooks {
            let event = HookEvent::PreToolUse {
                tool_name: tool_name.to_string(),
                input: effective_input.clone(),
            };
            match hook.handle(&event) {
                HookResult::Continue => {}
                HookResult::Transform(value) => {
                    effective_input = value;
                    transformed = true;
                }
                HookResult::Block(reason) => return HookResult::Block(reason),
            }
        }

        if transformed {
            HookResult::Transform(effective_input)
        } else {
            HookResult::Continue
        }
    }

    /// 执行所有 post-hooks
    pub fn run_post_hooks(&self, event: &HookEvent) {
        for hook in &self.post_hooks {
            hook.handle(event);
        }
    }

    /// 返回 pre-hook 数量
    pub fn pre_hook_count(&self) -> usize {
        self.pre_hooks.len()
    }

    /// 返回 post-hook 数量
    pub fn post_hook_count(&self) -> usize {
        self.post_hooks.len()
    }
}
