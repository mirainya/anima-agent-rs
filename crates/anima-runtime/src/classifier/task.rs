use indexmap::IndexMap;
use regex::Regex;
use std::sync::OnceLock;

// ── Task Types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskType {
    CodeGeneration,
    CodeReview,
    CodeDebug,
    CodeRefactor,
    WebSearch,
    Documentation,
    DataAnalysis,
    TestGeneration,
    ApiCall,
    GeneralChat,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CodeGeneration => "code-generation",
            Self::CodeReview => "code-review",
            Self::CodeDebug => "code-debug",
            Self::CodeRefactor => "code-refactor",
            Self::WebSearch => "web-search",
            Self::Documentation => "documentation",
            Self::DataAnalysis => "data-analysis",
            Self::TestGeneration => "test-generation",
            Self::ApiCall => "api-call",
            Self::GeneralChat => "general-chat",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::CodeGeneration => "代码生成 - 编写新代码",
            Self::CodeReview => "代码审查 - 检查代码质量",
            Self::CodeDebug => "代码调试 - 修复错误",
            Self::CodeRefactor => "代码重构 - 优化改进",
            Self::WebSearch => "网络搜索 - 查找信息",
            Self::Documentation => "文档生成 - 创建文档",
            Self::DataAnalysis => "数据分析 - 处理数据",
            Self::TestGeneration => "测试生成 - 编写测试",
            Self::ApiCall => "API调用 - 外部接口",
            Self::GeneralChat => "普通对话 - 日常交流",
        }
    }

    pub fn specialist(&self) -> &'static str {
        match self {
            Self::CodeGeneration
            | Self::CodeReview
            | Self::CodeDebug
            | Self::CodeRefactor
            | Self::TestGeneration => "code-specialist",
            Self::WebSearch => "search-specialist",
            Self::Documentation => "doc-specialist",
            Self::DataAnalysis => "analysis-specialist",
            Self::GeneralChat => "chat-specialist",
            _ => "general-specialist",
        }
    }
}

// ── Classification Result ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClassificationResult {
    pub task_type: TaskType,
    pub confidence: f64,
    pub scores: IndexMap<TaskType, u64>,
    pub detected_keywords: Vec<String>,
    pub message_length: usize,
    pub has_code_block: bool,
}

// ── Internal ────────────────────────────────────────────────────────

struct TaskTypeDef {
    keywords: Vec<&'static str>,
    patterns: Vec<Regex>,
    priority: u64,
}

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(p).expect("invalid regex"))
        .collect()
}

fn build_definitions() -> IndexMap<TaskType, TaskTypeDef> {
    let mut d = IndexMap::new();
    d.insert(
        TaskType::CodeGeneration,
        TaskTypeDef {
            keywords: vec![
                "write",
                "create",
                "implement",
                "build",
                "develop",
                "编写",
                "创建",
                "实现",
                "生成",
                "代码",
            ],
            patterns: compile(&[
                r"(?i)write\s+(a|an|the)?\s*(function|method|class|module|component)",
                r"(?i)implement\s+(a|an|the)?\s*\w+",
                r"(?i)create\s+(a|an|the)?\s*(function|class|api)",
                r"(?i)build\s+(a|an|the)?\s*\w+",
                "编写",
                "创建",
                "实现",
                "生成代码",
            ]),
            priority: 10,
        },
    );
    d.insert(
        TaskType::CodeReview,
        TaskTypeDef {
            keywords: vec![
                "review",
                "check",
                "analyze",
                "audit",
                "审查",
                "检查",
                "分析代码",
            ],
            patterns: compile(&[
                r"(?i)review\s+(this|the|my)?\s*code",
                r"(?i)check\s+(this|the|my)?\s*code",
                r"(?i)analyze\s+(this|the|my)?\s*code",
                r"(?i)code\s*review",
                "审查代码",
                "检查代码",
                "代码审查",
            ]),
            priority: 8,
        },
    );
    d.insert(
        TaskType::CodeDebug,
        TaskTypeDef {
            keywords: vec![
                "debug", "fix", "error", "bug", "issue", "problem", "调试", "修复", "错误",
            ],
            patterns: compile(&[
                r"(?i)debug\s+(this|the|my)?\s*(code|error|issue)",
                r"(?i)fix\s+(this|the|my)?\s*(error|bug|issue)",
                r"(?i)(error|exception|bug)\s+(in|with)",
                "调试",
                "修复",
                "错误",
                "报错",
            ]),
            priority: 9,
        },
    );
    d.insert(
        TaskType::CodeRefactor,
        TaskTypeDef {
            keywords: vec![
                "refactor", "optimize", "improve", "clean", "重构", "优化", "改进",
            ],
            patterns: compile(&[
                r"(?i)refactor\s+(this|the|my)?\s*code",
                r"(?i)optimize\s+(this|the|my)?\s*(code|performance)",
                r"(?i)improve\s+(this|the|my)?\s*code",
                "重构",
                "优化",
                "改进代码",
            ]),
            priority: 7,
        },
    );
    d.insert(
        TaskType::WebSearch,
        TaskTypeDef {
            keywords: vec!["search", "find", "lookup", "google", "搜索", "查找"],
            patterns: compile(&[
                r"(?i)search\s+(for|about|on)",
                r"(?i)find\s+(information|docs|documentation)",
                r"(?i)look\s+up",
                "搜索",
                "查找",
                "搜索一下",
            ]),
            priority: 6,
        },
    );
    d.insert(
        TaskType::Documentation,
        TaskTypeDef {
            keywords: vec![
                "document", "docs", "readme", "explain", "describe", "文档", "说明", "解释",
            ],
            patterns: compile(&[
                r"(?i)(write|create|generate)\s+(documentation|docs|readme)",
                r"(?i)document\s+(this|the|my)?\s*code",
                r"(?i)explain\s+(how|what|why)",
                "写文档",
                "生成文档",
                "说明文档",
            ]),
            priority: 5,
        },
    );
    d.insert(
        TaskType::DataAnalysis,
        TaskTypeDef {
            keywords: vec![
                "analyze",
                "data",
                "statistics",
                "chart",
                "graph",
                "分析",
                "数据",
                "统计",
                "图表",
            ],
            patterns: compile(&[
                r"(?i)analyze\s+(this|the|my)?\s*(data|dataset)",
                r"(?i)(show|create|generate)\s+(a|the)?\s*(chart|graph|plot)",
                "分析数据",
                "数据分析",
                "统计",
            ]),
            priority: 6,
        },
    );
    d.insert(
        TaskType::TestGeneration,
        TaskTypeDef {
            keywords: vec!["test", "testing", "spec", "unit test", "测试", "单元测试"],
            patterns: compile(&[
                r"(?i)(write|create|generate)\s+(tests?|specs?|unit\s+tests?)",
                r"(?i)test\s+(this|the|my)?\s*(function|method|class|code)",
                "写测试",
                "生成测试",
                "单元测试",
            ]),
            priority: 7,
        },
    );
    d.insert(
        TaskType::ApiCall,
        TaskTypeDef {
            keywords: vec!["api", "endpoint", "request", "http", "rest", "接口", "请求"],
            patterns: compile(&[
                r"(?i)(call|invoke|hit)\s+(the|an?)?\s*(api|endpoint)",
                r"(?i)(get|post|put|delete)\s+request",
                "调用接口",
                "API请求",
            ]),
            priority: 8,
        },
    );
    d.insert(
        TaskType::GeneralChat,
        TaskTypeDef {
            keywords: vec![
                "hello", "hi", "hey", "thanks", "thank", "你好", "谢谢", "您好",
            ],
            patterns: compile(&[
                r"(?i)^(hello|hi|hey|good\s+(morning|afternoon|evening))",
                "你好",
                "您好",
                "谢谢",
            ]),
            priority: 1,
        },
    );
    d
}

static DEFINITIONS: OnceLock<IndexMap<TaskType, TaskTypeDef>> = OnceLock::new();

fn definitions() -> &'static IndexMap<TaskType, TaskTypeDef> {
    DEFINITIONS.get_or_init(build_definitions)
}

// ── Classification Logic ────────────────────────────────────────────

fn count_keyword_matches(text: &str, keywords: &[&str]) -> u64 {
    let lower = text.to_lowercase();
    keywords
        .iter()
        .filter(|kw| lower.contains(&kw.to_lowercase()))
        .count() as u64
}

fn check_patterns(text: &str, patterns: &[Regex]) -> bool {
    patterns.iter().any(|p| p.is_match(text))
}

fn score_task_type(message: &str, def: &TaskTypeDef) -> u64 {
    let keyword_score = count_keyword_matches(message, &def.keywords) * 10;
    let pattern_score = if check_patterns(message, &def.patterns) {
        50
    } else {
        0
    };
    keyword_score + pattern_score + def.priority * 5
}

/// Classify a message into a task type with confidence scoring.
pub fn classify_task(message: &str) -> ClassificationResult {
    let defs = definitions();
    let has_code_block = message.contains("```") || message.contains('`');

    let mut scores = IndexMap::new();
    let mut best_type = TaskType::GeneralChat;
    let mut best_score = 0u64;
    let mut total_score = 0u64;

    for (task_type, def) in defs {
        let score = score_task_type(message, def);
        scores.insert(*task_type, score);
        total_score += score;
        if score > best_score {
            best_score = score;
            best_type = *task_type;
        }
    }

    let confidence = if total_score > 0 {
        (best_score as f64 / total_score as f64).min(1.0)
    } else {
        0.0
    };

    let detected_keywords = if let Some(def) = defs.get(&best_type) {
        let lower = message.to_lowercase();
        def.keywords
            .iter()
            .filter(|kw| lower.contains(&kw.to_lowercase()))
            .map(|kw| kw.to_string())
            .collect()
    } else {
        vec![]
    };

    ClassificationResult {
        task_type: best_type,
        confidence,
        scores,
        detected_keywords,
        message_length: message.len(),
        has_code_block,
    }
}

// ── Language Detection ──────────────────────────────────────────────

static LANG_PATTERNS: OnceLock<Vec<(&'static str, Vec<Regex>)>> = OnceLock::new();

fn lang_patterns() -> &'static Vec<(&'static str, Vec<Regex>)> {
    LANG_PATTERNS.get_or_init(|| {
        let raw: &[(&'static str, &[&str])] = &[
            ("rust", &[r"fn\s+\w+\s*\(", r"let\s+mut", r"impl\s+\w+"]),
            (
                "python",
                &[r"def\s+\w+\s*\(", r"import\s+\w+", r"from\s+\w+\s+import"],
            ),
            ("javascript", &[r"function\s+\w+\s*\(", r"const\s+\w+\s*="]),
            ("typescript", &[r"interface\s+\w+", r"type\s+\w+\s*="]),
            (
                "java",
                &[r"public\s+class", r"private\s+void", r"import\s+java"],
            ),
            ("go", &[r"func\s+\w+\s*\(", r"package\s+\w+"]),
            ("clojure", &[r"\(defn", r"\(def\s", r"\(ns\s"]),
        ];
        raw.iter()
            .map(|(lang, pats)| (*lang, compile(pats)))
            .collect()
    })
}

pub fn detect_language(message: &str) -> Option<&'static str> {
    for (lang, patterns) in lang_patterns() {
        if check_patterns(message, patterns) {
            return Some(lang);
        }
    }
    None
}

// ── Code Block Extraction ───────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct CodeBlocks {
    pub fenced: Vec<String>,
    pub inline: Vec<String>,
    pub has_code: bool,
}

pub fn extract_code_blocks(message: &str) -> CodeBlocks {
    let fenced_re = Regex::new(r"(?s)```\w*\n(.*?)```").expect("invalid regex");
    let inline_re = Regex::new(r"`([^`]+)`").expect("invalid regex");

    let fenced: Vec<String> = fenced_re
        .captures_iter(message)
        .filter_map(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .collect();

    let stripped = fenced_re.replace_all(message, "");
    let inline: Vec<String> = inline_re
        .captures_iter(&stripped)
        .filter_map(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .collect();

    let has_code = !fenced.is_empty() || !inline.is_empty();
    CodeBlocks {
        fenced,
        inline,
        has_code,
    }
}

// ── Task Context ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TaskContext {
    pub classification: ClassificationResult,
    pub code_blocks: CodeBlocks,
    pub language_hint: Option<&'static str>,
    pub is_urgent: bool,
}

pub fn extract_task_context(message: &str) -> TaskContext {
    let classification = classify_task(message);
    let code_blocks = extract_code_blocks(message);
    let language_hint = detect_language(message);

    let lower = message.to_lowercase();
    let is_urgent = ["urgent", "asap", "immediately", "紧急", "立即", "马上"]
        .iter()
        .any(|kw| lower.contains(kw));

    TaskContext {
        classification,
        code_blocks,
        language_hint,
        is_urgent,
    }
}

// ── Multiple Task Detection ──────────────────────────────────────────

pub fn detect_multiple_tasks(message: &str) -> Vec<ClassificationResult> {
    let separators = ["and then", "after that", "then", "然后", "接着", "；", ";"];

    let mut parts: Vec<&str> = vec![message];
    for sep in &separators {
        let new_parts: Vec<&str> = parts.iter().flat_map(|p| p.split(sep)).collect();
        if new_parts.len() > parts.len() {
            parts = new_parts;
            break;
        }
    }

    let trimmed: Vec<&str> = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    if trimmed.len() <= 1 {
        return vec![classify_task(message)];
    }

    trimmed.iter().map(|part| classify_task(part)).collect()
}
