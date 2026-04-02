//! 流式事件类型定义

use serde_json::Value;

/// SSE 流式事件类型
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// 消息开始
    MessageStart { message_id: String },
    /// 内容块开始
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    /// 内容块增量更新
    ContentBlockDelta { index: usize, delta: ContentDelta },
    /// 内容块结束
    ContentBlockStop { index: usize },
    /// 消息增量更新
    MessageDelta { stop_reason: Option<String> },
    /// 消息结束
    MessageStop,
    /// 心跳
    Ping,
    /// 错误
    Error { error_type: String, message: String },
}

/// 内容块类型
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    /// 文本块
    Text { text: String },
    /// 工具使用块
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

/// 内容增量
#[derive(Debug, Clone, PartialEq)]
pub enum ContentDelta {
    /// 文本增量
    TextDelta { text: String },
    /// 工具输入 JSON 增量
    InputJsonDelta { partial_json: String },
}

/// 流式工具追踪状态
#[derive(Debug, Clone, PartialEq)]
pub enum TrackedToolState {
    /// 等待输入完成
    ReceivingInput {
        id: String,
        name: String,
        accumulated_json: String,
    },
    /// 输入完成，等待执行
    ReadyToExecute {
        id: String,
        name: String,
        input: Value,
    },
    /// 执行中
    Executing { id: String, name: String },
    /// 执行完成
    Completed {
        id: String,
        name: String,
        result: Value,
    },
}
