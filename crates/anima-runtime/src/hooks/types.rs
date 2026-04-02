//! 钩子类型定义

use serde_json::Value;
use crate::tools::result::ToolResult;

/// 钩子事件
#[derive(Debug, Clone)]
pub enum HookEvent {
    /// 工具执行前
    PreToolUse {
        tool_name: String,
        input: Value,
    },
    /// 工具执行后
    PostToolUse {
        tool_name: String,
        result: ToolResult,
    },
    /// 消息发送前
    PreSendMessage {
        content: String,
    },
    /// 消息发送后
    PostSendMessage {
        content: String,
        response: Option<String>,
    },
}

/// 钩子执行结果
#[derive(Debug, Clone, PartialEq)]
pub enum HookResult {
    /// 继续执行
    Continue,
    /// 阻止后续操作
    Block(String),
    /// 修改输入后继续
    Transform(Value),
}

/// 钩子配置
#[derive(Debug, Clone)]
pub struct HookConfig {
    /// 钩子名称
    pub name: String,
    /// 匹配的事件类型
    pub event_pattern: String,
    /// 是否启用
    pub enabled: bool,
}
