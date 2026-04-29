use regex::Regex;

use crate::hooks::{HookEvent, HookHandler, HookResult};
use crate::tools::result::{ToolResult, ToolResultBlock};

#[derive(Debug, Default)]
pub struct StopHook;

impl HookHandler for StopHook {
    fn handle(&self, event: &HookEvent) -> HookResult {
        let HookEvent::PostToolUse { tool_name, result } = event else {
            return HookResult::Continue;
        };

        if !result.is_error {
            return HookResult::Continue;
        }

        let text = tool_result_text(result);
        if text.is_empty() {
            return HookResult::Continue;
        }

        let lower = text.to_ascii_lowercase();
        if !is_stop_candidate(&lower) {
            return HookResult::Continue;
        }

        HookResult::TransformToolResult(ToolResult::error(build_feedback_message(
            tool_name, &text, &lower,
        )))
    }
}

fn tool_result_text(result: &ToolResult) -> String {
    result
        .blocks
        .iter()
        .filter_map(|block| match block {
            ToolResultBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_stop_candidate(lower: &str) -> bool {
    matches_exit_code(lower)
        || lower.contains("--- stderr ---")
        || lower.contains("tool execution error:")
        || lower.contains("not found in registry")
        || lower.contains("blocked by hook:")
        || lower.contains("denied by user")
}

fn matches_exit_code(lower: &str) -> bool {
    Regex::new(r"(?m)^exit code \d+")
        .expect("valid regex")
        .is_match(lower)
}

fn build_feedback_message(tool_name: &str, text: &str, lower: &str) -> String {
    let guidance = if lower.contains("cargo clippy") || lower.contains("clippy") {
        "lint 检查失败，请根据下面的输出修复告警或错误后再继续。"
    } else if lower.contains("cargo test") || lower.contains("test failed") {
        "测试失败，请根据下面的输出修复失败项后再继续。"
    } else if lower.contains("npm run build")
        || lower.contains("vite build")
        || lower.contains("build failed")
    {
        "构建失败，请根据下面的输出修复构建错误后再继续。"
    } else if lower.contains("npm run lint") || lower.contains("eslint") {
        "前端 lint 检查失败，请根据下面的输出修复问题后再继续。"
    } else {
        "工具执行出现需要处理的错误，请先根据下面的输出完成修复，再继续后续步骤。"
    };

    format!(
        "[{tool_name}] {guidance}\n\n原始输出：\n{text}",
        tool_name = tool_name,
        guidance = guidance,
        text = text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_hook_transforms_matching_error_output() {
        let hook = StopHook;
        let event = HookEvent::PostToolUse {
            tool_name: "bash_exec".into(),
            result: ToolResult::error("exit code 1\n--- stderr ---\ntest failed"),
        };

        let result = hook.handle(&event);
        match result {
            HookResult::TransformToolResult(result) => {
                assert!(result.is_error);
                let text = tool_result_text(&result);
                assert!(text.contains("测试失败"));
                assert!(text.contains("原始输出"));
            }
            other => panic!("expected transformed tool result, got {other:?}"),
        }
    }

    #[test]
    fn stop_hook_ignores_success_output() {
        let hook = StopHook;
        let event = HookEvent::PostToolUse {
            tool_name: "bash_exec".into(),
            result: ToolResult::text("all good"),
        };

        assert_eq!(hook.handle(&event), HookResult::Continue);
    }
}
