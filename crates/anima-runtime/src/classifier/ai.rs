use crate::bus::InboundMessage;
use crate::cache::LruCache;
use crate::classifier::rule::{ClassificationDecision, ClassificationKind};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;

// ── Classification Types ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AiClassificationType {
    SimpleChat,
    ComplexTask,
    StatusQuery,
    SystemCommand,
}

impl AiClassificationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SimpleChat => "simple-chat",
            Self::ComplexTask => "complex-task",
            Self::StatusQuery => "status-query",
            Self::SystemCommand => "system-command",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "complex-task" => Self::ComplexTask,
            "status-query" => Self::StatusQuery,
            "system-command" => Self::SystemCommand,
            _ => Self::SimpleChat,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiClassificationResult {
    pub classification: AiClassificationType,
    pub confidence: f64,
    pub reasoning: String,
    pub quick: bool,
}

// ── Quick Pattern Classification ────────────────────────────────────

lazy_static::lazy_static! {
    static ref SIMPLE_CHAT_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"(?i)^(hi|hello|hey|good\s+(morning|afternoon|evening))$").unwrap(),
        Regex::new(r"^(你好|您好|嗨)$").unwrap(),
        Regex::new(r"(?i)^(thanks?|thank\s+you)$").unwrap(),
        Regex::new(r"^(谢谢|多谢)$").unwrap(),
    ];

    static ref STATUS_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"(?i)^status$").unwrap(),
        Regex::new(r"^(任务状态|进度|查询)$").unwrap(),
    ];

    static ref JSON_OBJECT_RE: Regex = Regex::new(r"\{[^{}]*\}").unwrap();

    static ref SYSTEM_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"(?i)^(exit|quit|bye)$").unwrap(),
        Regex::new(r"^(退出|再见)$").unwrap(),
    ];

    static ref COMPLEX_TASK_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"(?i).*(write|create|build|implement|develop|generate|make|design)\s+(a|an|the|some)?\s*(function|class|module|component|api|website|app|system|service|feature)").unwrap(),
        Regex::new(r"(?i).*(help|please)\s+(me\s+)?(write|create|build|implement|develop|generate|make)").unwrap(),
        Regex::new(r"(?i).*(fix|debug|solve|resolve)\s+(the\s+)?(bug|error|issue|problem)").unwrap(),
        Regex::new(r"(?i).*(refactor|optimize|improve|update)\s+(the\s+)?(code|function|class)").unwrap(),
        Regex::new(r".*(写|创建|建|实现|开发|生成|制作|设计).*(功能|函数|类|模块|组件|网站|应用|系统|服务|API)").unwrap(),
        Regex::new(r".*(帮我|请).*(写|创建|建|实现|开发|生成|制作|设计)").unwrap(),
        Regex::new(r".*(修复|调试|解决).*(bug|错误|问题)").unwrap(),
        Regex::new(r".*(重构|优化|改进).*(代码)").unwrap(),
    ];
}

pub fn quick_classify(message: &str) -> Option<AiClassificationResult> {
    let trimmed = message.trim();

    let check =
        |patterns: &[Regex], kind: AiClassificationType| -> Option<AiClassificationResult> {
            if patterns.iter().any(|p| p.is_match(trimmed)) {
                Some(AiClassificationResult {
                    classification: kind,
                    confidence: 1.0,
                    reasoning: "Pattern match".into(),
                    quick: true,
                })
            } else {
                None
            }
        };

    check(&SIMPLE_CHAT_PATTERNS, AiClassificationType::SimpleChat)
        .or_else(|| check(&STATUS_PATTERNS, AiClassificationType::StatusQuery))
        .or_else(|| check(&SYSTEM_PATTERNS, AiClassificationType::SystemCommand))
        .or_else(|| check(&COMPLEX_TASK_PATTERNS, AiClassificationType::ComplexTask))
}

// ── AI Classifier ───────────────────────────────────────────────────

pub static DEFAULT_CLASSIFICATION_PROMPT: &str = r#"You are a task classifier. Analyze the user's message and classify it.

RESPOND WITH ONLY A JSON OBJECT, NO OTHER TEXT.

Classification types:
1. "simple-chat" - General conversation, greetings, simple questions, asking about capabilities
2. "complex-task" - Requests to write code, create files, build features, fix bugs, implement things

Examples:
- "hi" → {"type": "simple-chat", "confidence": 0.95, "reasoning": "greeting"}
- "Write a function" → {"type": "complex-task", "confidence": 0.95, "reasoning": "code generation request"}

User message: "#;

pub struct AiClassifier {
    pub id: String,
    cache: Arc<LruCache>,
    prompt_template: String,
    cache_enabled: bool,
}

impl AiClassifier {
    pub fn new(cache: Arc<LruCache>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            cache,
            prompt_template: DEFAULT_CLASSIFICATION_PROMPT.to_string(),
            cache_enabled: true,
        }
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt_template = prompt.into();
        self
    }

    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.cache_enabled = enabled;
        self
    }

    /// Parse an AI response text into a classification result.
    pub fn parse_response(response_text: &str) -> AiClassificationResult {
        // Try to extract JSON from response
        let json_re = &*JSON_OBJECT_RE;
        if let Some(m) = json_re.find(response_text) {
            if let Ok(parsed) = serde_json::from_str::<Value>(m.as_str()) {
                let type_str = parsed
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("simple-chat");
                let confidence = parsed
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5);
                let reasoning = parsed
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .unwrap_or("AI classification")
                    .to_string();
                return AiClassificationResult {
                    classification: AiClassificationType::from_str(type_str),
                    confidence,
                    reasoning,
                    quick: false,
                };
            }
        }
        // Fallback
        AiClassificationResult {
            classification: AiClassificationType::SimpleChat,
            confidence: 0.5,
            reasoning: "Failed to parse AI response".into(),
            quick: false,
        }
    }

    fn cache_key(message: &str) -> String {
        format!("classify:{}", message.trim().to_lowercase())
    }

    /// Get cached classification.
    pub fn get_cached(&self, message: &str) -> Option<AiClassificationResult> {
        if !self.cache_enabled {
            return None;
        }
        let key = Self::cache_key(message);
        self.cache.get(&key).and_then(|v| {
            let t = v.get("type")?.as_str()?;
            let c = v.get("confidence")?.as_f64()?;
            let r = v.get("reasoning")?.as_str()?.to_string();
            Some(AiClassificationResult {
                classification: AiClassificationType::from_str(t),
                confidence: c,
                reasoning: r,
                quick: false,
            })
        })
    }

    /// Cache a classification result.
    pub fn cache_result(&self, message: &str, result: &AiClassificationResult) {
        if !self.cache_enabled {
            return;
        }
        let key = Self::cache_key(message);
        self.cache.set(
            &key,
            json!({
                "type": result.classification.as_str(),
                "confidence": result.confidence,
                "reasoning": result.reasoning,
            }),
            None,
        );
    }

    /// Build the full prompt for AI classification.
    pub fn build_prompt(&self, message: &str) -> String {
        format!("{}{}", self.prompt_template, message)
    }
}

// ── Smart Classify (combined entry point) ───────────────────────────

/// Smart classification: quick patterns first, then AI if available.
pub fn smart_classify(
    classifier: Option<&AiClassifier>,
    message: &str,
    ai_response: Option<&str>,
) -> AiClassificationResult {
    // 1. Quick patterns
    if let Some(result) = quick_classify(message) {
        return result;
    }

    // 2. Check cache
    if let Some(cls) = classifier {
        if let Some(cached) = cls.get_cached(message) {
            return cached;
        }
    }

    // 3. AI response (caller is responsible for making the API call)
    if let Some(response_text) = ai_response {
        let result = AiClassifier::parse_response(response_text);
        if let Some(cls) = classifier {
            cls.cache_result(message, &result);
        }
        return result;
    }

    // 4. Fallback
    AiClassificationResult {
        classification: AiClassificationType::SimpleChat,
        confidence: 0.5,
        reasoning: "No classifier available".into(),
        quick: false,
    }
}

// ── Bridge: AI classification → existing ClassificationKind ─────────

pub fn ai_to_rule_classification(
    ai_result: &AiClassificationResult,
    inbound_msg: &InboundMessage,
) -> ClassificationDecision {
    match ai_result.classification {
        AiClassificationType::SystemCommand => ClassificationDecision {
            kind: ClassificationKind::Direct,
        },
        AiClassificationType::StatusQuery => ClassificationDecision {
            kind: ClassificationKind::Direct,
        },
        AiClassificationType::SimpleChat => ClassificationDecision {
            kind: ClassificationKind::Single,
        },
        AiClassificationType::ComplexTask => {
            let specialist = inbound_msg
                .metadata
                .get("specialist")
                .and_then(Value::as_str);

            if specialist.is_some() {
                ClassificationDecision {
                    kind: ClassificationKind::SpecialistRoute,
                }
            } else {
                ClassificationDecision {
                    kind: ClassificationKind::Single,
                }
            }
        }
    }
}
