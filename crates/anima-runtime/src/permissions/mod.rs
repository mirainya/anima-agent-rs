//! 权限判定系统
//!
//! 对标 claude-code-main 的 `permissions.ts`。

pub mod checker;
pub mod policy;
pub mod types;

pub use checker::PermissionChecker;
pub use policy::{AllowAllPolicy, DenyAllPolicy, PermissionPolicy};
pub use types::{
    PermissionDecision, PermissionMode, PermissionRequest, PermissionRiskLevel, PermissionRule,
};
