//! 文件写入工具

use serde_json::{json, Value};
use std::fmt::Debug;
use std::fs;

use super::validate_path_safe;
use crate::tools::definition::{Tool, ToolContext};
use crate::tools::result::{ToolError, ToolResult};

#[derive(Debug)]
pub struct FileWriteTool;

impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with the given content."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    fn validate_input(&self, input: &Value) -> Result<(), String> {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or("missing required field: 'file_path' (string)")?;
        validate_path_safe(path)?;

        input
            .get("content")
            .and_then(Value::as_str)
            .ok_or("missing required field: 'content' (string)")?;

        Ok(())
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let file_path = input["file_path"].as_str().expect("validated");
        let content = input["content"].as_str().expect("validated");

        let path = std::path::Path::new(file_path);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| {
                    ToolError::Internal(format!(
                        "failed to create parent directory '{}': {e}",
                        parent.display()
                    ))
                })?;
            }
        }

        fs::write(file_path, content)
            .map_err(|e| ToolError::Internal(format!("failed to write '{file_path}': {e}")))?;

        Ok(ToolResult::text(format!(
            "wrote {} bytes to '{file_path}'",
            content.len()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            trace_id: "test".into(),
            metadata: json!({}),
        }
    }

    #[test]
    fn test_write_new_file() {
        let dir = std::env::temp_dir().join("anima_test_file_write");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("new.txt");

        let tool = FileWriteTool;
        let input = json!({"file_path": path.to_string_lossy(), "content": "hello world"});
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("anima_test_file_write_deep");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("a").join("b").join("c.txt");

        let tool = FileWriteTool;
        let input = json!({"file_path": path.to_string_lossy(), "content": "deep"});
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);
        assert_eq!(fs::read_to_string(&path).unwrap(), "deep");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_path_traversal() {
        let tool = FileWriteTool;
        assert!(tool
            .validate_input(&json!({"file_path": "../evil.txt", "content": "x"}))
            .is_err());
    }
}
