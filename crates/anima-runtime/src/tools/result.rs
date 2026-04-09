//! 工具执行结果类型

use serde_json::Value;

/// 工具结果块类型
#[derive(Debug, Clone, PartialEq)]
pub enum ToolResultBlock {
    /// 纯文本结果
    Text(String),
    /// JSON 结构化结果
    Json(Value),
    /// 二进制数据（Base64 编码）
    Binary { mime_type: String, data: String },
}

/// 工具执行结果
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// 结果块列表（工具可能返回多个结果块）
    pub blocks: Vec<ToolResultBlock>,
    /// 是否为错误结果
    pub is_error: bool,
}

impl ToolResult {
    /// 创建一个成功的文本结果
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            blocks: vec![ToolResultBlock::Text(content.into())],
            is_error: false,
        }
    }

    /// 创建一个错误文本结果
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            blocks: vec![ToolResultBlock::Text(message.into())],
            is_error: true,
        }
    }

    /// 创建一个 JSON 结果
    pub fn json(value: Value) -> Self {
        Self {
            blocks: vec![ToolResultBlock::Json(value)],
            is_error: false,
        }
    }
}

/// 工具执行错误
#[derive(Debug, Clone, PartialEq)]
pub enum ToolError {
    /// 输入校验失败
    ValidationFailed(String),
    /// 权限不足
    PermissionDenied(String),
    /// 执行超时
    Timeout { tool_name: String, timeout_ms: u64 },
    /// 工具内部错误
    Internal(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ValidationFailed(msg) => write!(f, "validation failed: {msg}"),
            Self::PermissionDenied(msg) => write!(f, "permission denied: {msg}"),
            Self::Timeout {
                tool_name,
                timeout_ms,
            } => {
                write!(f, "tool '{tool_name}' timed out after {timeout_ms}ms")
            }
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for ToolError {}
