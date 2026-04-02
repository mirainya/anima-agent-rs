//! 工具定义 trait
//!
//! 所有可注册工具必须实现 `Tool` trait。

use serde_json::Value;
use std::fmt::Debug;

use super::result::{ToolError, ToolResult};

/// 工具执行时的上下文信息
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// 当前会话 ID
    pub session_id: String,
    /// 当前追踪 ID
    pub trace_id: String,
    /// 额外元数据
    pub metadata: Value,
}

/// 工具 trait：所有可注册工具必须实现
///
/// # 生命周期
/// 1. `validate_input()` — 校验入参是否合法
/// 2. `call()` — 执行工具逻辑，返回结果或错误
pub trait Tool: Send + Sync + Debug {
    /// 工具唯一名称
    fn name(&self) -> &str;

    /// 入参 JSON Schema，供注册中心校验和文档展示
    fn input_schema(&self) -> Value;

    /// 校验入参，返回 Ok(()) 或错误信息
    fn validate_input(&self, input: &Value) -> std::result::Result<(), String>;

    /// 执行工具
    fn call(&self, input: Value, context: &ToolContext) -> std::result::Result<ToolResult, ToolError>;

    /// 可选：渲染工具结果为人类可读文本
    fn render(&self, result: &ToolResult) -> Option<String> {
        let _ = result;
        None
    }
}
