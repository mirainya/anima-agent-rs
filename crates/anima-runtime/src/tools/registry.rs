//! 工具注册中心
//!
//! 管理所有可用工具的注册、查询和列表。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::definition::Tool;

/// 工具注册中心：管理所有可用工具
#[derive(Debug, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// 注册一个工具
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// 注销一个工具
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.remove(name)
    }

    /// 按名称查找工具
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// 列出所有已注册工具的名称
    pub fn list_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// 返回已注册工具数量
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// 生成所有已注册工具的定义列表（供 API 的 tools 参数使用）
    pub fn tool_definitions(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|tool| {
                json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::definition::ToolContext;
    use crate::tools::result::{ToolError, ToolResult};

    #[derive(Debug)]
    struct DummyTool {
        tool_name: &'static str,
        tool_desc: &'static str,
    }

    impl Tool for DummyTool {
        fn name(&self) -> &str {
            self.tool_name
        }
        fn description(&self) -> &str {
            self.tool_desc
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn validate_input(&self, _input: &Value) -> Result<(), String> {
            Ok(())
        }
        fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::text("ok"))
        }
    }

    /// description() 的默认实现：使用 Tool trait 的 default（返回 name）
    #[derive(Debug)]
    struct DefaultDescTool;

    impl Tool for DefaultDescTool {
        fn name(&self) -> &str {
            "default_desc"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn validate_input(&self, _input: &Value) -> Result<(), String> {
            Ok(())
        }
        fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::text("ok"))
        }
    }

    #[test]
    fn test_tool_definitions_empty() {
        let registry = ToolRegistry::new();
        let defs = registry.tool_definitions();
        assert!(defs.is_empty());
    }

    #[test]
    fn test_tool_definitions_format() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool {
            tool_name: "my_tool",
            tool_desc: "A useful tool",
        }));

        let defs = registry.tool_definitions();
        assert_eq!(defs.len(), 1);

        let def = &defs[0];
        assert_eq!(def["name"], "my_tool");
        assert_eq!(def["description"], "A useful tool");
        assert!(def.get("input_schema").is_some());
    }

    #[test]
    fn test_tool_description_default() {
        let tool = DefaultDescTool;
        // 默认 description() 返回 name()
        assert_eq!(tool.description(), "default_desc");
    }
}
