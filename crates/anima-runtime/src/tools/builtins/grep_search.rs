//! 正则表达式文件内容搜索工具

use regex::Regex;
use serde_json::{json, Value};
use std::fmt::Debug;
use std::fs;
use std::path::Path;

use crate::tools::definition::{Tool, ToolContext};
use crate::tools::result::{ToolError, ToolResult};

/// 最大匹配条数
const MAX_MATCHES: usize = 500;

#[derive(Debug)]
pub struct GrepSearchTool;

impl Tool for GrepSearchTool {
    fn name(&self) -> &str {
        "grep_search"
    }

    fn description(&self) -> &str {
        "Search file contents using a regex pattern."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in (default: current directory)"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs')"
                },
                "include_context": {
                    "type": "integer",
                    "description": "Number of context lines before and after each match"
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
        Regex::new(pattern).map_err(|e| format!("invalid regex: {e}"))?;

        if let Some(ctx) = input.get("include_context") {
            let v = ctx.as_i64().ok_or("'include_context' must be an integer")?;
            if v < 0 {
                return Err("'include_context' must be >= 0".into());
            }
        }
        Ok(())
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let pattern = input["pattern"].as_str().expect("validated");
        let search_path = input.get("path").and_then(Value::as_str).unwrap_or(".");
        let glob_filter = input.get("glob").and_then(Value::as_str);
        let context_lines = input
            .get("include_context")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;

        let re = Regex::new(pattern)
            .map_err(|e| ToolError::Internal(format!("regex compile error: {e}")))?;

        let glob_re = glob_filter
            .map(glob_to_regex)
            .transpose()
            .map_err(|e| ToolError::Internal(format!("glob filter error: {e}")))?;

        let mut results = Vec::new();
        let path = Path::new(search_path);

        if path.is_file() {
            search_file(&re, path, context_lines, &mut results);
        } else if path.is_dir() {
            walk_dir(path, &re, glob_re.as_ref(), context_lines, &mut results);
        } else {
            return Ok(ToolResult::error(format!(
                "path '{search_path}' does not exist"
            )));
        }

        if results.is_empty() {
            return Ok(ToolResult::text(format!(
                "no matches found for pattern '{pattern}'"
            )));
        }

        let truncated = results.len() > MAX_MATCHES;
        if truncated {
            results.truncate(MAX_MATCHES);
        }

        let mut output = results.join("\n");
        if truncated {
            output.push_str(&format!("\n\n(truncated at {MAX_MATCHES} matches)"));
        }

        Ok(ToolResult::text(output))
    }
}

/// 递归遍历目录
fn walk_dir(
    dir: &Path,
    re: &Regex,
    glob_re: Option<&Regex>,
    context_lines: usize,
    results: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if results.len() >= MAX_MATCHES {
            return;
        }

        // 跳过隐藏文件/目录和常见的大目录
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
        }

        if path.is_dir() {
            walk_dir(&path, re, glob_re, context_lines, results);
        } else if path.is_file() {
            // 检查 glob 过滤
            if let Some(glob_re) = glob_re {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !glob_re.is_match(file_name) {
                    continue;
                }
            }
            search_file(re, &path, context_lines, results);
        }
    }
}

/// 搜索单个文件
fn search_file(re: &Regex, path: &Path, context_lines: usize, results: &mut Vec<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // 跳过不可读的文件（二进制等）
    };

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    for (i, line) in lines.iter().enumerate() {
        if results.len() >= MAX_MATCHES {
            return;
        }
        if re.is_match(line) {
            let start = i.saturating_sub(context_lines);
            let end = (i + context_lines + 1).min(total);

            let mut block = format!("{}:{}:", path.display(), i + 1);
            if context_lines > 0 {
                block.push('\n');
                for (j, line_content) in lines.iter().enumerate().take(end).skip(start) {
                    let marker = if j == i { ">" } else { " " };
                    block.push_str(&format!("{marker} {:>4} | {}\n", j + 1, line_content));
                }
            } else {
                block.push_str(line);
            }
            results.push(block);
        }
    }
}

/// 简易 glob → regex 转换（支持 * 和 ?）
fn glob_to_regex(glob: &str) -> Result<Regex, String> {
    let mut re = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '.' | '+' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(ch);
            }
            _ => re.push(ch),
        }
    }
    re.push('$');
    Regex::new(&re).map_err(|e| format!("failed to build glob regex: {e}"))
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
    fn test_grep_known_content() {
        let dir = std::env::temp_dir().join("anima_test_grep");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.rs");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "fn main() {{}}").unwrap();
        writeln!(f, "fn helper() {{}}").unwrap();
        writeln!(f, "struct Foo;").unwrap();

        let tool = GrepSearchTool;
        let input = json!({"pattern": "fn \\w+", "path": dir.to_string_lossy()});
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("fn main"));
        assert!(text.contains("fn helper"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_grep_with_context() {
        let dir = std::env::temp_dir().join("anima_test_grep_ctx");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ctx.txt");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "MATCH HERE").unwrap();
        writeln!(f, "line 4").unwrap();
        writeln!(f, "line 5").unwrap();

        let tool = GrepSearchTool;
        let input = json!({
            "pattern": "MATCH",
            "path": dir.to_string_lossy(),
            "include_context": 1
        });
        let result = tool.call(input, &ctx()).unwrap();
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("line 2"));
        assert!(text.contains("MATCH HERE"));
        assert!(text.contains("line 4"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_grep_no_match() {
        let dir = std::env::temp_dir().join("anima_test_grep_nomatch");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.txt");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "hello world").unwrap();

        let tool = GrepSearchTool;
        let input = json!({"pattern": "zzz_nonexistent", "path": dir.to_string_lossy()});
        let result = tool.call(input, &ctx()).unwrap();
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("no matches found"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_invalid_regex() {
        let tool = GrepSearchTool;
        assert!(tool
            .validate_input(&json!({"pattern": "[invalid"}))
            .is_err());
    }
}
