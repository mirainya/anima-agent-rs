//! 权限策略 trait

use serde_json::Value;

use super::types::PermissionDecision;

/// 权限策略 trait
pub trait PermissionPolicy: Send + Sync + std::fmt::Debug {
    /// 判定指定工具和输入是否有权限执行
    fn check(&self, tool_name: &str, input: &Value) -> PermissionDecision;
}

/// 始终允许的策略
#[derive(Debug)]
pub struct AllowAllPolicy;

impl PermissionPolicy for AllowAllPolicy {
    fn check(&self, _tool_name: &str, _input: &Value) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

/// 始终拒绝的策略
#[derive(Debug)]
pub struct DenyAllPolicy;

impl PermissionPolicy for DenyAllPolicy {
    fn check(&self, tool_name: &str, _input: &Value) -> PermissionDecision {
        PermissionDecision::Deny(format!("all tools denied: {tool_name}"))
    }
}
