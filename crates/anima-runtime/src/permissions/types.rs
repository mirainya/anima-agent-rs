//! 权限类型定义

/// 权限判定结果
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    /// 允许执行
    Allow,
    /// 拒绝执行，附带原因
    Deny(String),
    /// 需要用户确认
    Ask(String),
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

