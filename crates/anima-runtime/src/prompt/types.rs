//! System Prompt 类型定义

/// 一个可组装的提示词段落
#[derive(Debug, Clone)]
pub struct PromptSection {
    /// 段落标识，如 "identity", "tools_guidance", "environment"
    pub id: String,
    /// 段落内容
    pub content: String,
    /// 排序权重，越小越靠前
    pub order: i32,
}

/// 组装完成的系统提示词
#[derive(Debug, Clone, Default)]
pub struct SystemPrompt {
    /// 最终拼接的文本
    pub text: String,
    /// 参与拼接的段落 ID 列表（按 order 排列）
    pub section_ids: Vec<String>,
}

/// 环境信息（供 environment 段落使用）
#[derive(Debug, Clone, Default)]
pub struct EnvironmentInfo {
    /// 运行平台，如 "windows", "linux"
    pub platform: Option<String>,
    /// 当前工作目录
    pub working_dir: Option<String>,
    /// 自定义键值对
    pub custom: Vec<(String, String)>,
}
