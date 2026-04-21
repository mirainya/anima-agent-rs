//! 权限判定系统
//!
//! 对标 claude-code-main 的 `permissions.ts`。

pub(crate) mod checker;
pub(crate) mod policy;
pub(crate) mod types;

pub use checker::PermissionChecker;
pub use policy::{AllowAllPolicy, DenyAllPolicy, PermissionPolicy};
pub use types::{
    PermissionDecision, PermissionMode, PermissionRequest, PermissionRiskLevel, PermissionRule,
};
