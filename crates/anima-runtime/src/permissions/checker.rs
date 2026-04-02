//! 权限检查器
//!
//! 组合权限模式、规则列表和策略，提供统一的权限判定入口。

use serde_json::Value;
use std::sync::Arc;

use super::policy::PermissionPolicy;
use super::types::{PermissionDecision, PermissionMode, PermissionRule};

/// 权限检查器
#[derive(Debug)]
pub struct PermissionChecker {
    mode: PermissionMode,
    rules: Vec<PermissionRule>,
    policy: Option<Arc<dyn PermissionPolicy>>,
}

impl PermissionChecker {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            rules: Vec::new(),
            policy: None,
        }
    }

    /// 添加权限规则
    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
        self.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// 设置自定义策略
    pub fn set_policy(&mut self, policy: Arc<dyn PermissionPolicy>) {
        self.policy = Some(policy);
    }

    /// 判定指定工具是否有权限执行
    pub fn has_permission(&self, tool_name: &str, input: &Value) -> PermissionDecision {
        // 1. 模式快速判定
        match &self.mode {
            PermissionMode::AllowAll => return PermissionDecision::Allow,
            PermissionMode::DenyAll => {
                return PermissionDecision::Deny("deny-all mode".into());
            }
            PermissionMode::RuleBased => {}
        }

        // 2. 规则匹配
        for rule in &self.rules {
            if tool_matches(&rule.tool_pattern, tool_name) {
                return rule.decision.clone();
            }
        }

        // 3. 自定义策略
        if let Some(policy) = &self.policy {
            return policy.check(tool_name, input);
        }

        // 4. 默认：需要确认
        PermissionDecision::Ask(format!("allow tool '{tool_name}'?"))
    }
}

/// 简单的工具名称匹配（支持 `*` 通配符）
fn tool_matches(pattern: &str, tool_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return tool_name.starts_with(prefix);
    }
    pattern == tool_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_mode() {
        let checker = PermissionChecker::new(PermissionMode::AllowAll);
        assert_eq!(
            checker.has_permission("bash", &Value::Null),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn deny_all_mode() {
        let checker = PermissionChecker::new(PermissionMode::DenyAll);
        assert!(matches!(
            checker.has_permission("bash", &Value::Null),
            PermissionDecision::Deny(_)
        ));
    }

    #[test]
    fn rule_based_matching() {
        let mut checker = PermissionChecker::new(PermissionMode::RuleBased);
        checker.add_rule(PermissionRule {
            tool_pattern: "read*".into(),
            decision: PermissionDecision::Allow,
            priority: 10,
        });
        assert_eq!(
            checker.has_permission("read_file", &Value::Null),
            PermissionDecision::Allow
        );
        assert!(matches!(
            checker.has_permission("bash", &Value::Null),
            PermissionDecision::Ask(_)
        ));
    }
}
