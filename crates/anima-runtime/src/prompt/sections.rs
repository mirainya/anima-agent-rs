//! 内置段落工厂函数
//!
//! 提供预定义的 PromptSection 构造方法。

#![allow(dead_code)]

use super::types::{EnvironmentInfo, PromptSection};

/// 身份段落：描述智能体的角色和名称
pub(crate) fn identity_section(agent_name: &str) -> PromptSection {
    PromptSection {
        id: "identity".into(),
        content: format!(
            "You are {agent_name}, an AI assistant powered by Anima runtime. \
             Follow user instructions carefully and use available tools when needed."
        ),
        order: 0,
    }
}

/// 工具使用指南段落：列出可用工具名称
pub(crate) fn tool_usage_section(tool_names: &[&str]) -> PromptSection {
    let list = if tool_names.is_empty() {
        "No tools are currently available.".to_string()
    } else {
        format!(
            "You have access to the following tools: {}. \
             Use them when appropriate to accomplish the user's request.",
            tool_names.join(", ")
        )
    };
    PromptSection {
        id: "tools_guidance".into(),
        content: list,
        order: 100,
    }
}

pub(crate) fn completion_status_section() -> PromptSection {
    PromptSection {
        id: "completion_status".into(),
        content: "在每次回复的最末尾，附加一个 XML 标记表示任务完成状态：\n\
                  - <completion_status>complete</completion_status> — 已完整满足用户请求\n\
                  - <completion_status>partial</completion_status> — 部分完成，仍需继续\n\
                  - <completion_status>needs_input</completion_status> — 需要用户提供额外信息\n\
                  此标记必须出现在回复正文之后，不要省略。"
            .into(),
        order: 300,
    }
}

/// 环境信息段落：平台、工作目录等运行时上下文
pub(crate) fn environment_section(env: &EnvironmentInfo) -> PromptSection {
    let mut parts = Vec::new();
    if let Some(ref platform) = env.platform {
        parts.push(format!("Platform: {platform}"));
    }
    if let Some(ref dir) = env.working_dir {
        parts.push(format!("Working directory: {dir}"));
    }
    for (key, value) in &env.custom {
        parts.push(format!("{key}: {value}"));
    }
    let content = if parts.is_empty() {
        "No environment information available.".to_string()
    } else {
        format!("# Environment\n{}", parts.join("\n"))
    };
    PromptSection {
        id: "environment".into(),
        content,
        order: 200,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_section() {
        let section = identity_section("Anima");
        assert_eq!(section.id, "identity");
        assert_eq!(section.order, 0);
        assert!(section.content.contains("Anima"));
    }

    #[test]
    fn test_environment_section() {
        let env = EnvironmentInfo {
            platform: Some("linux".into()),
            working_dir: Some("/home/user".into()),
            custom: vec![("Shell".into(), "bash".into())],
        };
        let section = environment_section(&env);
        assert_eq!(section.id, "environment");
        assert_eq!(section.order, 200);
        assert!(section.content.contains("linux"));
        assert!(section.content.contains("/home/user"));
        assert!(section.content.contains("Shell: bash"));
    }

    #[test]
    fn test_tool_usage_section() {
        let section = tool_usage_section(&["echo", "read_file"]);
        assert_eq!(section.id, "tools_guidance");
        assert_eq!(section.order, 100);
        assert!(section.content.contains("echo"));
        assert!(section.content.contains("read_file"));
    }

    #[test]
    fn test_tool_usage_section_empty() {
        let section = tool_usage_section(&[]);
        assert!(section.content.contains("No tools"));
    }

    #[test]
    fn test_environment_section_empty() {
        let env = EnvironmentInfo::default();
        let section = environment_section(&env);
        assert!(section.content.contains("No environment information"));
    }
}
