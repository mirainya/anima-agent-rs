//! 工具执行流程
//!
//! 实现 schema 校验 → pre-hook → permission → call → post-hook → tool_result 的完整执行闭环。

use serde_json::Value;
use std::sync::Arc;

use super::definition::{Tool, ToolContext};
use super::result::{ToolError, ToolResult};
use crate::hooks::{HookEvent, HookRegistry};
use crate::permissions::{PermissionChecker, PermissionDecision, PermissionRequest};

/// 工具执行选项
#[derive(Debug, Clone)]
pub struct RunToolOptions {
    /// 工具名称
    pub tool_name: String,
    /// 工具入参
    pub input: Value,
    /// 执行上下文
    pub context: ToolContext,
}

#[derive(Debug, Clone)]
pub struct AwaitingToolPermission {
    pub tool_name: String,
    pub input: Value,
    pub permission_request: PermissionRequest,
    pub context: ToolContext,
}

#[derive(Debug, Clone)]
pub enum RunToolUseOutcome {
    Completed(ToolResult),
    AwaitingPermission(Box<AwaitingToolPermission>),
}

/// 执行一次工具调用的完整流程
///
/// 流程：schema 校验 → pre-hook → 权限检查 → 调用 → post-hook
pub fn run_tool_use(
    tool: &Arc<dyn Tool>,
    options: RunToolOptions,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
) -> std::result::Result<RunToolUseOutcome, ToolError> {
    // 1. 校验入参
    tool.validate_input(&options.input)
        .map_err(ToolError::ValidationFailed)?;

    // 2. Pre-hook
    if let Some(hooks) = hook_registry {
        let event = HookEvent::PreToolUse {
            tool_name: options.tool_name.clone(),
            input: options.input.clone(),
        };
        hooks.run_pre_hooks(&event);
    }

    // 3. 权限检查
    if let Some(checker) = permission_checker {
        match checker.has_permission(tool.name(), &options.input) {
            PermissionDecision::Allow => {}
            PermissionDecision::Deny(reason) => {
                return Err(ToolError::PermissionDenied(reason));
            }
            PermissionDecision::Ask(permission_request) => {
                return Ok(RunToolUseOutcome::AwaitingPermission(Box::new(AwaitingToolPermission {
                    tool_name: options.tool_name.clone(),
                    input: options.input.clone(),
                    permission_request,
                    context: options.context.clone(),
                })));
            }
        }
    }

    // 4. 调用工具
    let result = tool.call(options.input.clone(), &options.context)?;

    // 5. Post-hook
    if let Some(hooks) = hook_registry {
        let event = HookEvent::PostToolUse {
            tool_name: options.tool_name,
            result: result.clone(),
        };
        hooks.run_post_hooks(&event);
    }

    Ok(RunToolUseOutcome::Completed(result))
}

pub fn execute_tool_after_permission(
    tool: &Arc<dyn Tool>,
    options: RunToolOptions,
    hook_registry: Option<&HookRegistry>,
) -> std::result::Result<ToolResult, ToolError> {
    tool.validate_input(&options.input)
        .map_err(ToolError::ValidationFailed)?;

    if let Some(hooks) = hook_registry {
        let event = HookEvent::PreToolUse {
            tool_name: options.tool_name.clone(),
            input: options.input.clone(),
        };
        hooks.run_pre_hooks(&event);
    }

    let result = tool.call(options.input.clone(), &options.context)?;

    if let Some(hooks) = hook_registry {
        let event = HookEvent::PostToolUse {
            tool_name: options.tool_name,
            result: result.clone(),
        };
        hooks.run_post_hooks(&event);
    }

    Ok(result)
}
