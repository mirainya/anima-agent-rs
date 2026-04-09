//! 工具执行流程
//!
//! 实现 schema 校验 → pre-hook → permission → call → post-hook → tool_result 的完整执行闭环。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use super::definition::{Tool, ToolContext};
use super::result::{ToolError, ToolResult, ToolResultBlock};
use crate::hooks::{HookEvent, HookRegistry};
use crate::permissions::{PermissionChecker, PermissionDecision, PermissionRequest};
use crate::support::now_ms;

/// 工具执行选项
#[derive(Debug, Clone)]
pub struct RunToolOptions {
    /// 工具名称
    pub tool_name: String,
    /// 工具入参
    pub input: Value,
    /// 执行上下文
    pub context: ToolContext,
    /// 可选的调用标识；未提供时会自动生成
    pub invocation_id: Option<String>,
}

pub type ToolLifecycleEventCallback = Arc<dyn Fn(&str, Value) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInvocationPhase {
    Detected,
    ValidatingInput,
    PermissionChecking,
    PermissionRequested,
    PermissionResolved,
    Executing,
    Completed,
    Failed,
    ResultRecorded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionState {
    NotRequired,
    Checking,
    Requested,
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocationRecord {
    pub invocation_id: String,
    pub tool_name: String,
    pub phase: ToolInvocationPhase,
    pub permission_state: ToolPermissionState,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub tool_use_id: Option<String>,
    pub result_summary: Option<String>,
    pub error_summary: Option<String>,
}

impl ToolInvocationRecord {
    pub fn new(invocation_id: String, tool_name: String, tool_use_id: Option<String>) -> Self {
        Self {
            invocation_id,
            tool_name,
            phase: ToolInvocationPhase::Detected,
            permission_state: ToolPermissionState::NotRequired,
            started_at_ms: now_ms(),
            finished_at_ms: None,
            tool_use_id,
            result_summary: None,
            error_summary: None,
        }
    }

    pub fn mark_phase(&mut self, phase: ToolInvocationPhase) {
        self.phase = phase;
        if matches!(
            self.phase,
            ToolInvocationPhase::Completed | ToolInvocationPhase::Failed
        ) {
            self.finished_at_ms = Some(now_ms());
        }
    }

    pub fn set_permission_state(&mut self, state: ToolPermissionState) {
        self.permission_state = state;
    }

    pub fn set_result(&mut self, result: &ToolResult) {
        self.result_summary = summarize_tool_result(result);
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error_summary = Some(error.into());
    }
}

#[derive(Debug, Clone)]
pub struct AwaitingToolPermission {
    pub invocation: ToolInvocationRecord,
    pub input: Value,
    pub permission_request: PermissionRequest,
    pub context: ToolContext,
}

#[derive(Debug, Clone)]
pub enum RunToolUseOutcome {
    Completed {
        invocation: ToolInvocationRecord,
        result: ToolResult,
    },
    AwaitingPermission(Box<AwaitingToolPermission>),
}

fn emit_tool_lifecycle_event(
    callback: Option<&ToolLifecycleEventCallback>,
    event: &str,
    record: &ToolInvocationRecord,
    details: Value,
) {
    if let Some(callback) = callback {
        callback(
            event,
            serde_json::json!({
                "invocation_id": record.invocation_id,
                "tool_name": record.tool_name,
                "tool_use_id": record.tool_use_id,
                "phase": record.phase,
                "permission_state": record.permission_state,
                "started_at_ms": record.started_at_ms,
                "finished_at_ms": record.finished_at_ms,
                "result_summary": record.result_summary,
                "error_summary": record.error_summary,
                "details": details,
            }),
        );
    }
}

fn summarize_tool_result(result: &ToolResult) -> Option<String> {
    let parts = result
        .blocks
        .iter()
        .take(2)
        .map(|block| match block {
            ToolResultBlock::Text(text) => text.clone(),
            ToolResultBlock::Json(value) => serde_json::to_string(value).unwrap_or_default(),
            ToolResultBlock::Binary { mime_type, data } => {
                format!("[binary: {mime_type}, {} bytes]", data.len())
            }
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        let joined = parts.join("\n");
        let summary: String = joined.chars().take(240).collect();
        Some(summary)
    }
}

fn execute_tool_call(
    tool: &Arc<dyn Tool>,
    options: &RunToolOptions,
    hook_registry: Option<&HookRegistry>,
    invocation: &mut ToolInvocationRecord,
    lifecycle_callback: Option<&ToolLifecycleEventCallback>,
) -> std::result::Result<ToolResult, ToolError> {
    if let Some(hooks) = hook_registry {
        let event = HookEvent::PreToolUse {
            tool_name: options.tool_name.clone(),
            input: options.input.clone(),
        };
        hooks.run_pre_hooks(&event);
    }

    invocation.mark_phase(ToolInvocationPhase::Executing);
    emit_tool_lifecycle_event(
        lifecycle_callback,
        "tool_execution_started",
        invocation,
        serde_json::json!({
            "tool_input": options.input,
        }),
    );

    let result = match tool.call(options.input.clone(), &options.context) {
        Ok(result) => result,
        Err(err) => {
            invocation.mark_phase(ToolInvocationPhase::Failed);
            invocation.set_error(err.to_string());
            emit_tool_lifecycle_event(
                lifecycle_callback,
                "tool_execution_failed",
                invocation,
                serde_json::json!({
                    "error": err.to_string(),
                }),
            );
            return Err(err);
        }
    };

    if let Some(hooks) = hook_registry {
        let event = HookEvent::PostToolUse {
            tool_name: options.tool_name.clone(),
            result: result.clone(),
        };
        hooks.run_post_hooks(&event);
    }

    invocation.mark_phase(ToolInvocationPhase::Completed);
    invocation.set_result(&result);
    emit_tool_lifecycle_event(
        lifecycle_callback,
        "tool_execution_finished",
        invocation,
        serde_json::json!({
            "is_error": result.is_error,
        }),
    );

    Ok(result)
}

/// 执行一次工具调用的完整流程
///
/// 流程：schema 校验 → pre-hook → 权限检查 → 调用 → post-hook
pub fn run_tool_use(
    tool: &Arc<dyn Tool>,
    options: RunToolOptions,
    permission_checker: Option<&PermissionChecker>,
    hook_registry: Option<&HookRegistry>,
    lifecycle_callback: Option<&ToolLifecycleEventCallback>,
) -> std::result::Result<RunToolUseOutcome, ToolError> {
    let mut invocation = ToolInvocationRecord::new(
        options
            .invocation_id
            .clone()
            .unwrap_or_else(|| format!("toolinv_{}", uuid::Uuid::new_v4())),
        options.tool_name.clone(),
        None,
    );

    emit_tool_lifecycle_event(
        lifecycle_callback,
        "tool_invocation_detected",
        &invocation,
        serde_json::json!({
            "tool_input": options.input,
        }),
    );

    invocation.mark_phase(ToolInvocationPhase::ValidatingInput);
    tool.validate_input(&options.input).map_err(|msg| {
        invocation.mark_phase(ToolInvocationPhase::Failed);
        invocation.set_error(msg.clone());
        emit_tool_lifecycle_event(
            lifecycle_callback,
            "tool_execution_failed",
            &invocation,
            serde_json::json!({
                "error": msg,
                "stage": "validate_input",
            }),
        );
        ToolError::ValidationFailed(msg)
    })?;

    invocation.mark_phase(ToolInvocationPhase::PermissionChecking);
    invocation.set_permission_state(if permission_checker.is_some() {
        ToolPermissionState::Checking
    } else {
        ToolPermissionState::NotRequired
    });

    if let Some(checker) = permission_checker {
        match checker.has_permission(tool.name(), &options.input) {
            PermissionDecision::Allow => {
                invocation.set_permission_state(ToolPermissionState::Allowed);
                invocation.mark_phase(ToolInvocationPhase::PermissionResolved);
                emit_tool_lifecycle_event(
                    lifecycle_callback,
                    "tool_permission_resolved",
                    &invocation,
                    serde_json::json!({
                        "decision": "allow",
                        "source": "permission_checker",
                    }),
                );
            }
            PermissionDecision::Deny(reason) => {
                invocation.set_permission_state(ToolPermissionState::Denied);
                invocation.mark_phase(ToolInvocationPhase::Failed);
                invocation.set_error(reason.clone());
                emit_tool_lifecycle_event(
                    lifecycle_callback,
                    "tool_permission_resolved",
                    &invocation,
                    serde_json::json!({
                        "decision": "deny",
                        "source": "permission_checker",
                        "reason": reason,
                    }),
                );
                return Err(ToolError::PermissionDenied(reason));
            }
            PermissionDecision::Ask(permission_request) => {
                invocation.set_permission_state(ToolPermissionState::Requested);
                invocation.mark_phase(ToolInvocationPhase::PermissionRequested);
                emit_tool_lifecycle_event(
                    lifecycle_callback,
                    "tool_permission_requested",
                    &invocation,
                    serde_json::json!({
                        "prompt": permission_request.prompt,
                        "options": permission_request.options,
                        "input_preview": permission_request.input_preview,
                    }),
                );
                return Ok(RunToolUseOutcome::AwaitingPermission(Box::new(
                    AwaitingToolPermission {
                        invocation,
                        input: options.input.clone(),
                        permission_request,
                        context: options.context.clone(),
                    },
                )));
            }
        }
    }

    let result = execute_tool_call(
        tool,
        &options,
        hook_registry,
        &mut invocation,
        lifecycle_callback,
    )?;

    Ok(RunToolUseOutcome::Completed { invocation, result })
}

pub fn execute_tool_after_permission(
    tool: &Arc<dyn Tool>,
    options: RunToolOptions,
    hook_registry: Option<&HookRegistry>,
    mut invocation: ToolInvocationRecord,
    lifecycle_callback: Option<&ToolLifecycleEventCallback>,
) -> std::result::Result<(ToolInvocationRecord, ToolResult), ToolError> {
    invocation.mark_phase(ToolInvocationPhase::ValidatingInput);
    tool.validate_input(&options.input).map_err(|msg| {
        invocation.mark_phase(ToolInvocationPhase::Failed);
        invocation.set_error(msg.clone());
        emit_tool_lifecycle_event(
            lifecycle_callback,
            "tool_execution_failed",
            &invocation,
            serde_json::json!({
                "error": msg,
                "stage": "validate_input",
            }),
        );
        ToolError::ValidationFailed(msg)
    })?;

    invocation.set_permission_state(ToolPermissionState::Allowed);
    invocation.mark_phase(ToolInvocationPhase::PermissionResolved);
    emit_tool_lifecycle_event(
        lifecycle_callback,
        "tool_permission_resolved",
        &invocation,
        serde_json::json!({
            "decision": "allow",
            "source": "resume",
        }),
    );

    let result = execute_tool_call(
        tool,
        &options,
        hook_registry,
        &mut invocation,
        lifecycle_callback,
    )?;

    Ok((invocation, result))
}
