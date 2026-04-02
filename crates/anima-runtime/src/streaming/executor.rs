//! 流式工具执行器
//!
//! 对标 claude-code-main 的 `StreamingToolExecutor.ts`。
//! 追踪流式响应中的 tool_use 块，在输入完成时触发执行。

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::types::TrackedToolState;
use crate::tools::registry::ToolRegistry;

/// 流式工具执行器：追踪正在接收的 tool_use 块
#[derive(Debug)]
pub struct StreamingToolExecutor {
    /// 按 content block index 追踪的工具状态
    tracked: HashMap<usize, TrackedToolState>,
    /// 工具注册中心引用
    registry: Arc<ToolRegistry>,
}

impl StreamingToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            tracked: HashMap::new(),
            registry,
        }
    }

    /// 开始追踪一个新的 tool_use 块
    pub fn on_tool_use_start(&mut self, index: usize, id: String, name: String) {
        self.tracked.insert(
            index,
            TrackedToolState::ReceivingInput {
                id,
                name,
                accumulated_json: String::new(),
            },
        );
    }

    /// 追加 tool_use 输入的 JSON 增量
    pub fn on_input_delta(&mut self, index: usize, partial_json: &str) {
        if let Some(TrackedToolState::ReceivingInput {
            accumulated_json, ..
        }) = self.tracked.get_mut(&index)
        {
            accumulated_json.push_str(partial_json);
        }
    }

    /// 标记 tool_use 输入完成，解析完整 JSON
    pub fn on_tool_use_stop(&mut self, index: usize) -> Option<&TrackedToolState> {
        let state = self.tracked.get(&index)?;
        if let TrackedToolState::ReceivingInput {
            id,
            name,
            accumulated_json,
        } = state
        {
            let input: Value = serde_json::from_str(accumulated_json).unwrap_or(Value::Null);
            let new_state = TrackedToolState::ReadyToExecute {
                id: id.clone(),
                name: name.clone(),
                input,
            };
            self.tracked.insert(index, new_state);
        }
        self.tracked.get(&index)
    }

    /// 获取指定 index 的追踪状态
    pub fn get_state(&self, index: usize) -> Option<&TrackedToolState> {
        self.tracked.get(&index)
    }

    /// 获取工具注册中心
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// 列出所有追踪中的 tool index
    pub fn tracked_indices(&self) -> Vec<usize> {
        self.tracked.keys().copied().collect()
    }
}
