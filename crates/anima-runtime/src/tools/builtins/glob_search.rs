//! Glob 模式文件搜索工具

use serde_json::{json, Value};
use std::fmt::Debug;

use crate::tools::definition::{Tool, ToolContext};
use crate::tools::result::{ToolError, ToolResult};

/// 最大返回条数
const MAX_RESULTS: usize = 1000;

#[derive(Debug)]
pub struct GlobSearchTool;

impl Tool for GlobSearchTool {
    fn name(&self) -> &str {
        "glob_search"
    }

    fn description(&self) -> &str {
        "Search for files matching a glob pattern."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g. '**/*.rs', 'src/**/*.ts')"
                },
                "path": {
                    "type": "string",
                    "description": "Base directory to search in (default: current directory)"
                }
            },
            "required": ["pattern"]
        })
    }

    fn validate_input(&self, input: &Value) -> Result<(), String> {
        let pattern = input
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or("missing required field: 'pattern' (string)")?;
        if pattern.trim().is_empty() {
            return Err("'pattern' must not be empty".into());
        }
        // 验证 glob pattern 合法性
        let full = if let Some(base) = input.get("path").and_then(Value::as_str) {
            format!("{base}/{pattern}")
        } else {
            pattern.to_string()
        };
        glob::glob(&full).map_err(|e| format!("invalid glob pattern: {e}"))?;
        Ok(())
    }

    fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let pattern = input["pattern"].as_str().expect("validated");
        let base = input.get("path").and_then(Value::as_str);

        let full_pattern = match base {
            Some(b) => format!("{b}/{pattern}"),
            None => pattern.to_string(),
        };

        let entries = glob::glob(&full_pattern)
            .map_err(|e| ToolError::Internal(format!("glob error: {e}")))?;

        let mut paths = Vec::new();
        let mut truncated = false;

        for entry in entries {
            match entry {
                Ok(path) => {
                    paths.push(path.display().to_string());
                    if paths.len() >= MAX_RESULTS {
                        truncated = true;
                        break;
                    }
                }
                Err(e) => {
                    // 跳过无法访问的路径
                    eprintln!("glob entry error: {e}");
                }
            }
        }

        if paths.is_empty() {
            return Ok(ToolResult::text(format!(
                "no files matched pattern '{full_pattern}'"
            )));
        }

        let mut output = paths.join("\n");
        if truncated {
            output.push_str(&format!("\n\n(truncated at {MAX_RESULTS} results)"));
        }

        Ok(ToolResult::text(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::Write;

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            trace_id: "test".into(),
            metadata: json!({}),
        }
    }

    #[test]
    fn test_glob_match_rs_files() {
        let dir = std::env::temp_dir().join("anima_test_glob");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut f = fs::File::create(dir.join("main.rs")).unwrap();
        write!(f, "fn main() {{}}").unwrap();
        let mut f = fs::File::create(dir.join("lib.rs")).unwrap();
        write!(f, "// lib").unwrap();
        let mut f = fs::File::create(dir.join("readme.md")).unwrap();
        write!(f, "# readme").unwrap();

        let tool = GlobSearchTool;
        let input = json!({"pattern": "*.rs", "path": dir.to_string_lossy()});
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("main.rs"));
        assert!(text.contains("lib.rs"));
        assert!(!text.contains("readme.md"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_glob_no_match() {
        let dir = std::env::temp_dir().join("anima_test_glob_empty");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let tool = GlobSearchTool;
        let input = json!({"pattern": "*.xyz_nonexistent", "path": dir.to_string_lossy()});
        let result = tool.call(input, &ctx()).unwrap();
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("no files matched"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_invalid_pattern() {
        let tool = GlobSearchTool;
        assert!(tool.validate_input(&json!({"pattern": "[invalid"})).is_err());
    }
}
