use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::approval::ApprovalMode;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AnimaConfig {
    pub server: ServerConfig,
    pub cli: CliConfig,
    pub sdk: SdkConfig,
    pub runtime: RuntimeConfig,
    pub worker: WorkerConfig,
    pub approval_mode: ApprovalMode,
    pub provider: ProviderConfig,
    pub permissions: PermissionConfig,
    pub prompts: PromptsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    pub prompt: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SdkConfig {
    pub request_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub retry_backoff_cap_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub state_path: String,
    pub builtin_tools: bool,
    pub store: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WorkerConfig {
    pub pool_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub kind: String,
    pub model: String,
    pub api_key_env: String,
    pub base_url: Option<String>,
    pub max_tokens: u32,
}


impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:9711".into(),
        }
    }
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            prompt: "anima> ".into(),
            enabled: true,
        }
    }
}

impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            request_timeout_ms: 600_000,
            connect_timeout_ms: 60_000,
            max_retries: 1,
            retry_backoff_ms: 250,
            retry_backoff_cap_ms: 5_000,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            state_path: ".opencode/runtime/state.json".into(),
            builtin_tools: true,
            store: "json".into(),
        }
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { pool_size: 4 }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: "opencode".into(),
            model: "claude-sonnet-4-20250514".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            base_url: None,
            max_tokens: 8192,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PromptsConfig {
    pub task_decompose: String,
    pub context_infer: String,
    pub escalation_resolve: String,
    pub requirement_judge: String,
    pub question_followup: String,
    pub result_followup: String,
    pub agentic_loop_system: String,
    pub tool_resume_system: String,
    pub message_classifier: String,
    pub identity: String,
}

impl Default for PromptsConfig {
    fn default() -> Self {
        Self {
            task_decompose: DEFAULT_TASK_DECOMPOSE_PROMPT.into(),
            context_infer: DEFAULT_CONTEXT_INFER_PROMPT.into(),
            escalation_resolve: DEFAULT_ESCALATION_RESOLVE_PROMPT.into(),
            requirement_judge: DEFAULT_REQUIREMENT_JUDGE_PROMPT.into(),
            question_followup: DEFAULT_QUESTION_FOLLOWUP_PROMPT.into(),
            result_followup: DEFAULT_RESULT_FOLLOWUP_PROMPT.into(),
            agentic_loop_system: DEFAULT_AGENTIC_LOOP_SYSTEM_PROMPT.into(),
            tool_resume_system: DEFAULT_TOOL_RESUME_SYSTEM_PROMPT.into(),
            message_classifier: DEFAULT_MESSAGE_CLASSIFIER_PROMPT.into(),
            identity: DEFAULT_IDENTITY_PROMPT.into(),
        }
    }
}

pub const DEFAULT_TASK_DECOMPOSE_PROMPT: &str = "\
你是任务分解引擎。分析用户请求，判断是否需要拆解为多个子任务。\n\n\
用户请求: {request}\n\n\
## 判断标准\n\
### 必须拆解的情况（最高优先级）：\n\
- 用户明确要求拆分/拆解/分成多个任务（如『拆成4个任务』『分别做』『并行执行』）\n\
- 用户指定了具体的子任务数量或列举了多个独立工作项\n\
- 此时严格按照用户指定的数量和内容拆分，每个子任务的 description 必须精确描述该子任务要做的具体事情\n\n\
### 不需要拆解，直接返回空数组 []：\n\
- 简单问答、闲聊、知识查询\n\
- 单步操作（如：翻译一段话、解释一个概念、写一个函数）\n\
- 请求本身已经足够具体，不需要分工协作\n\n\
### 需要拆解：\n\
- 请求包含多个独立或有依赖关系的工作项\n\
- 需要不同专业能力协作完成（如设计+实现+测试）\n\
- 工作量大到需要分阶段推进\n\n\
## 输出格式\n\
输出 JSON 数组，每个元素包含:\n\
- name: 子任务名称（英文短横线命名）\n\
- task_type: 类型（design/frontend/backend/testing/data-collection/analysis/generic）\n\
- specialist_type: 专家类型（designer/frontend-dev/backend-dev/tester/data-engineer/analyst/default）\n\
- dependencies: 依赖的子任务 name 数组（禁止循环依赖）\n\
- description: 该子任务要执行的具体指令（必须足够具体，让执行者无需看原始请求就能完成）\n\n\
约束：子任务数量不超过 6 个。只输出 JSON 数组，不要其他内容。";

pub const DEFAULT_CONTEXT_INFER_PROMPT: &str = "\
你是上下文完整性分析器。以下是多个子任务的执行结果，判断是否有多个子任务因缺少共享上下文信息而无法继续。\n\n\
原始请求: {request}\n\n\
子任务结果:\n{results}\n\n\
请输出 JSON 对象:\n\
- needs_question: bool — 是否需要向用户追问\n\
- prompt: string — 追问内容（简洁精准，一个问题）\n\
- options: string[] — 3-5 个常见选项\n\n\
如果子任务结果充分、不需要追问，设 needs_question 为 false。\n\
只输出 JSON 对象，不要其他内容。";

pub const DEFAULT_ESCALATION_RESOLVE_PROMPT: &str = "\
你是主调度 Agent。一个子任务执行时遇到阻塞，需要你判断能否从已有信息中推断出答案。\n\n\
用户原始请求: {request}\n\n\
阻塞原因: {reason}\n\n\
如果你能从用户请求中推断出合理答案，请直接给出简短答案。\n\
如果信息不足无法判断，请只回复: CANNOT_RESOLVE";

pub const DEFAULT_REQUIREMENT_JUDGE_PROMPT: &str = "\
你是需求完成度评估器。判断 AI 的回复是否完整满足了用户的原始请求。\n\n\
用户请求: {request}\n\n\
AI 回复（前500字）: {reply_preview}\n\n\
输出 JSON: {{\"satisfied\": true/false, \"reason\": \"简短原因\"}}";

pub const DEFAULT_QUESTION_FOLLOWUP_PROMPT: &str = "\
上游返回了一个结构化问题，请尝试从用户原始请求的上下文中推断答案并继续执行。\n\
如果确实无法推断，请返回结构化 question 给用户。\n\n\
用户原始请求: {request}\n\n\
问题: {question}\n\
选项: {options}";

pub const DEFAULT_RESULT_FOLLOWUP_PROMPT: &str = "\
用户原始请求: {request}\n\n\
上一轮结果（前300字）: {result_preview}\n\n\
{instruction}";

pub const DEFAULT_AGENTIC_LOOP_SYSTEM_PROMPT: &str = "你是 Anima 智能助手。";

pub const DEFAULT_TOOL_RESUME_SYSTEM_PROMPT: &str = "你是 Anima 智能助手。";

pub const DEFAULT_MESSAGE_CLASSIFIER_PROMPT: &str = r#"You are a task classifier. Analyze the user's message and classify it.

RESPOND WITH ONLY A JSON OBJECT, NO OTHER TEXT.

Classification types:
1. "simple-chat" - General conversation, greetings, simple questions, asking about capabilities
2. "complex-task" - Requests to write code, create files, build features, fix bugs, implement things

Examples:
- "hi" → {"type": "simple-chat", "confidence": 0.95, "reasoning": "greeting"}
- "Write a function" → {"type": "complex-task", "confidence": 0.95, "reasoning": "code generation request"}

User message: "#;

pub const DEFAULT_IDENTITY_PROMPT: &str = "\
You are {agent_name}, an AI assistant powered by Anima runtime. \
Follow user instructions carefully and use available tools when needed.";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PermissionConfig {
    pub mode: String,
    pub rules: Vec<PermissionRuleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PermissionRuleConfig {
    pub tool_pattern: String,
    pub decision: String,
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_priority() -> u32 {
    10
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            mode: "rule_based".into(),
            rules: vec![
                PermissionRuleConfig {
                    tool_pattern: "read*".into(),
                    decision: "allow".into(),
                    priority: 10,
                },
                PermissionRuleConfig {
                    tool_pattern: "list*".into(),
                    decision: "allow".into(),
                    priority: 10,
                },
                PermissionRuleConfig {
                    tool_pattern: "glob*".into(),
                    decision: "allow".into(),
                    priority: 10,
                },
                PermissionRuleConfig {
                    tool_pattern: "grep*".into(),
                    decision: "allow".into(),
                    priority: 10,
                },
            ],
        }
    }
}

impl AnimaConfig {
    pub fn load(path: Option<&Path>) -> Self {
        let mut config = match path {
            Some(p) => Self::load_from_file(p),
            None => {
                let default_path = Path::new("anima.toml");
                if default_path.exists() {
                    Self::load_from_file(default_path)
                } else {
                    Self::default()
                }
            }
        };
        config.apply_env_overrides();
        config
    }

    fn load_from_file(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("ANIMA_SDK_REQUEST_TIMEOUT_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.request_timeout_ms = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_CONNECT_TIMEOUT_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.connect_timeout_ms = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_MAX_RETRIES") {
            if let Ok(v) = v.parse() {
                self.sdk.max_retries = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_RETRY_BACKOFF_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.retry_backoff_ms = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_RETRY_BACKOFF_CAP_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.retry_backoff_cap_ms = v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_hardcoded_values() {
        let c = AnimaConfig::default();
        assert_eq!(c.server.url, "http://127.0.0.1:9711");
        assert_eq!(c.cli.prompt, "anima> ");
        assert!(c.cli.enabled);
        assert_eq!(c.sdk.request_timeout_ms, 600_000);
        assert_eq!(c.sdk.connect_timeout_ms, 60_000);
        assert_eq!(c.sdk.max_retries, 1);
        assert_eq!(c.runtime.state_path, ".opencode/runtime/state.json");
        assert!(c.runtime.builtin_tools);
        assert_eq!(c.worker.pool_size, 4);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let c = AnimaConfig::load(Some(Path::new("nonexistent.toml")));
        assert_eq!(c.server.url, "http://127.0.0.1:9711");
    }

    #[test]
    fn parse_partial_toml() {
        let toml_str = r#"
[server]
url = "http://localhost:8080"

[worker]
pool_size = 8
"#;
        let c: AnimaConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(c.server.url, "http://localhost:8080");
        assert_eq!(c.worker.pool_size, 8);
        // unset fields keep defaults
        assert_eq!(c.sdk.request_timeout_ms, 600_000);
        assert_eq!(c.cli.prompt, "anima> ");
    }

    #[test]
    fn env_override_takes_precedence() {
        std::env::set_var("ANIMA_SDK_MAX_RETRIES", "5");
        let mut c = AnimaConfig::default();
        c.apply_env_overrides();
        assert_eq!(c.sdk.max_retries, 5);
        std::env::remove_var("ANIMA_SDK_MAX_RETRIES");
    }
}
