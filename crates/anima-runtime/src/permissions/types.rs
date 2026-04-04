//! 权限类型定义

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 权限风险等级
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRiskLevel {
    Low,
    High,
}

/// 结构化权限确认请求
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub risk_level: PermissionRiskLevel,
    pub requires_user_confirmation: bool,
    pub raw_input: Value,
    pub input_preview: String,
}

/// 权限判定结果
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    /// 允许执行
    Allow,
    /// 拒绝执行，附带原因
    Deny(String),
    /// 需要用户确认
    Ask(PermissionRequest),
}

/// 权限规则
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRule {
    /// 匹配的工具名称模式（支持 glob）
    pub tool_pattern: String,
    /// 判定结果
    pub decision: PermissionDecision,
    /// 规则优先级（越高越优先）
    pub priority: u32,
}

/// 权限模式
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Default)]
pub enum PermissionMode {
    /// 全部允许（无需确认）
    AllowAll,
    /// 全部拒绝
    DenyAll,
    /// 基于规则判定
    #[default]
    RuleBased,
}

