//! 文件读取工具

use serde_json::{json, Value};
use std::fmt::Debug;
use std::fs;

use super::validate_path_safe;
use crate::tools::definition::{Tool, ToolContext};
use crate::tools::result::{ToolError, ToolResult};

#[derive(Debug)]
pub struct FileReadTool;

impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file with optional line range."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Starting line number (1-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["file_path"]
        })
    }

    fn validate_input(&self, input: &Value) -> Result<(), String> {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or("missing required field: 'file_path' (string)")?;
        validate_path_safe(path)?;

        if let Some(offset) = input.get("offset") {
            let v = offset.as_i64().ok_or("'offset' must be an integer")?;
            if v < 1 {
                return Err("'offset' must be >= 1".into());
            }
        }
        if let Some(limit) = input.get("limit") {
            let v = limit.as_i64().ok_or("'limit' must be an integer")?;
            if v < 1 {
                return Err("'limit' must be >= 1".into());
            }
        }
        Ok(())
    }

    fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let file_path = input["file_path"].as_str().expect("validated");
        let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(1) as usize;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize);

        let content = fs::read_to_string(file_path)
            .map_err(|e| ToolError::Internal(format!("failed to read '{file_path}': {e}")))?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let start = (offset - 1).min(total);
        let end = match limit {
            Some(l) => (start + l).min(total),
            None => total,
        };

        let mut output = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            output.push_str(&format!("{line_num:>6}\t{line}\n"));
        }

        if output.is_empty() {
            output = "(empty file or no lines in range)\n".into();
        }

        Ok(ToolResult::text(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            trace_id: "test".into(),
            metadata: json!({}),
        }
    }

    #[test]
    fn test_read_existing_file() {
        let dir = std::env::temp_dir().join("anima_test_file_read");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.txt");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "line one").unwrap();
        writeln!(f, "line two").unwrap();
        writeln!(f, "line three").unwrap();

        let tool = FileReadTool;
        let input = json!({"file_path": path.to_string_lossy()});
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("line one"));
        assert!(text.contains("line three"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_with_offset_and_limit() {
        let dir = std::env::temp_dir().join("anima_test_file_read_range");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("range.txt");
        let mut f = fs::File::create(&path).unwrap();
        for i in 1..=10 {
            writeln!(f, "line {i}").unwrap();
        }

        let tool = FileReadTool;
        let input = json!({"file_path": path.to_string_lossy(), "offset": 3, "limit": 2});
        let result = tool.call(input, &ctx()).unwrap();
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("line 3"));
        assert!(text.contains("line 4"));
        assert!(!text.contains("line 5"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_nonexistent_file() {
        let tool = FileReadTool;
        let input = json!({"file_path": "/tmp/anima_nonexistent_xyz_123.txt"});
        let result = tool.call(input, &ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_path_traversal() {
        let tool = FileReadTool;
        assert!(tool
            .validate_input(&json!({"file_path": "../etc/passwd"}))
            .is_err());
    }
}
