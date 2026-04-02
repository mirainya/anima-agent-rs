//! Shell 命令执行工具

use serde_json::{json, Value};
use std::fmt::Debug;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::tools::definition::{Tool, ToolContext};
use crate::tools::result::{ToolError, ToolResult};

/// 默认超时：30 秒
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// 最大超时：10 分钟
const MAX_TIMEOUT_MS: u64 = 600_000;

#[derive(Debug)]
pub struct BashExecTool;

impl Tool for BashExecTool {
    fn name(&self) -> &str {
        "bash_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 30000, max: 600000)"
                }
            },
            "required": ["command"]
        })
    }

    fn validate_input(&self, input: &Value) -> Result<(), String> {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .ok_or("missing required field: 'command' (string)")?;
        if command.trim().is_empty() {
            return Err("'command' must not be empty".into());
        }
        if let Some(timeout) = input.get("timeout_ms") {
            let ms = timeout
                .as_u64()
                .ok_or("'timeout_ms' must be a positive integer")?;
            if ms > MAX_TIMEOUT_MS {
                return Err(format!("'timeout_ms' must not exceed {MAX_TIMEOUT_MS}"));
            }
        }
        Ok(())
    }

    fn call(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let command = input["command"]
            .as_str()
            .expect("validated")
            .to_string();
        let timeout_ms = input
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        let (tx, rx) = mpsc::channel();

        let cmd = command.clone();
        thread::spawn(move || {
            let output = if cfg!(target_os = "windows") {
                Command::new("cmd").args(["/C", &cmd]).output()
            } else {
                Command::new("sh").args(["-c", &cmd]).output()
            };
            let _ = tx.send(output);
        });

        match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("{stdout}\n--- stderr ---\n{stderr}")
                };

                if output.status.success() {
                    Ok(ToolResult::text(combined))
                } else {
                    let code = output.status.code().unwrap_or(-1);
                    Ok(ToolResult::error(format!(
                        "exit code {code}\n{combined}"
                    )))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!(
                "failed to execute command: {e}"
            ))),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(ToolError::Timeout {
                tool_name: "bash_exec".into(),
                timeout_ms,
            }),
            Err(e) => Err(ToolError::Internal(format!(
                "channel error: {e}"
            ))),
        }
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
    fn test_echo_command() {
        let tool = BashExecTool;
        let input = if cfg!(target_os = "windows") {
            json!({"command": "echo hello"})
        } else {
            json!({"command": "echo hello"})
        };
        let result = tool.call(input, &ctx()).unwrap();
        assert!(!result.is_error);
        let text = match &result.blocks[0] {
            crate::tools::result::ToolResultBlock::Text(t) => t.clone(),
            _ => panic!("expected text"),
        };
        assert!(text.contains("hello"));
    }

    #[test]
    fn test_nonexistent_command() {
        let tool = BashExecTool;
        let input = json!({"command": "this_command_does_not_exist_xyz_123"});
        let result = tool.call(input, &ctx()).unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_nonzero_exit_code() {
        let tool = BashExecTool;
        let input = if cfg!(target_os = "windows") {
            json!({"command": "cmd /C exit 42"})
        } else {
            json!({"command": "exit 42"})
        };
        let result = tool.call(input, &ctx()).unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_validate_missing_command() {
        let tool = BashExecTool;
        assert!(tool.validate_input(&json!({})).is_err());
    }

    #[test]
    fn test_validate_empty_command() {
        let tool = BashExecTool;
        assert!(tool.validate_input(&json!({"command": "  "})).is_err());
    }

    #[test]
    fn test_validate_timeout_too_large() {
        let tool = BashExecTool;
        assert!(tool
            .validate_input(&json!({"command": "echo", "timeout_ms": 999_999_999}))
            .is_err());
    }
}
