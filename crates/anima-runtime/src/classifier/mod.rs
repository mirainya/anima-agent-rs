//! 分类与路由域：规则分类、AI 分类、任务分类、智能路由

pub mod ai;
pub mod router;
pub mod rule;
pub mod task;

pub use ai::{
    ai_to_rule_classification, quick_classify, smart_classify, AiClassificationResult,
    AiClassificationType, AiClassifier, DEFAULT_CLASSIFICATION_PROMPT,
};
pub use router::{
    format_response, DialogContext, FormattedResponse, IntelligentRouter,
    IntelligentRouterConfig, ProcessResult, ProcessStatus, RouterMetrics, RoutedDialog,
    RoutedDialogStatus, SpecialistDef, DEFAULT_SPECIALISTS,
};
pub use rule::{AgentClassifier, ClassificationDecision, ClassificationKind};
pub use task::{
    classify_task, detect_language, detect_multiple_tasks, extract_code_blocks,
    extract_task_context, ClassificationResult, CodeBlocks, TaskContext, TaskType,
};
