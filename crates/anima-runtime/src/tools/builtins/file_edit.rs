//! 文件编辑工具（精确字符串替换）

use serde_json::{json, Value};
use std::fmt::Debug;
use std::fs;

use super::validate_path_safe;
use crate::tools::definition::{Tool, ToolContext};
use crate::tools::result::{ToolError, ToolResult};

#[derive(Debug)]
pub struct FileEditTool;

impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string match."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement string"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn validate_input(&self, input: &Value) -> Result<(), String> {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or("missing required field: 'file_path' (string)")?;
        validate_path_safe(path)?;

        input
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required field: 'old_string' (string)".to_string())?;

        input
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required field: 'new_string' (string)".to_string())?;

        Ok(())
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let file_path = input["file_path"].as_str().expect("validated");
        let old_string = input["old_string"].as_str().expect("validated");
        let new_string = input["new_string"].as_str().expect("validated");
        let replace_all = input
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let content = fs::read_to_string(file_path)
            .map_err(|e| ToolError::Internal(format!("failed to read '{file_path}': {e}")))?;

        if !content.contains(old_string) {
            return Ok(ToolResult::error(format!(
                "old_string not found in '{file_path}'"
            )));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        fs::write(file_path, &new_content)
            .map_err(|e| ToolError::Internal(format!("failed to write '{file_path}': {e}")))?;

        let count = if replace_all {
            content.matches(old_string).count()
        } else {
            1
        };

        Ok(ToolResult::text(format!(
            "replaced {count} occurrence(s) in '{file_path}'"
        )))
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
    fn test_single_replace() {
        let dir = std::env::temp_dir().join("anima_test_file_edit");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("edit.txt");
        let mut f = fs::File::create(&path).unwrap();
        write!(f, "foo bar foo baz").unwrap();

        let tool = FileEditTool;
        let input = json!({
            "file_path": path.to_string_lossy(),
            "old_string": "foo",
            "new_string": "qux"
        });
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "qux bar foo baz");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_replace_all() {
        let dir = std::env::temp_dir().join("anima_test_file_edit_all");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("edit_all.txt");
        let mut f = fs::File::create(&path).unwrap();
        write!(f, "foo bar foo baz foo").unwrap();

        let tool = FileEditTool;
        let input = json!({
            "file_path": path.to_string_lossy(),
            "old_string": "foo",
            "new_string": "qux",
            "replace_all": true
        });
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "qux bar qux baz qux");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_old_string_not_found() {
        let dir = std::env::temp_dir().join("anima_test_file_edit_nf");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("nf.txt");
        let mut f = fs::File::create(&path).unwrap();
        write!(f, "hello world").unwrap();

        let tool = FileEditTool;
        let input = json!({
            "file_path": path.to_string_lossy(),
            "old_string": "nonexistent",
            "new_string": "x"
        });
        let result = tool.call(input, &ctx()).unwrap();
        assert!(result.is_error);

        let _ = fs::remove_dir_all(&dir);
    }
}
