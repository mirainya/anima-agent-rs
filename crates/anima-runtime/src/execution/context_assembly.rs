use crate::agent::{ExecutionSummary, PendingQuestion};

/// 上下文装配模式，对应主 agent 的三类核心回合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAssemblyMode {
    Initial,
    QuestionContinuation,
    SubtaskBlockedContinuation,
    Followup,
}

/// 提供给 requirement judge 的稳定上下文快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementContextSnapshot {
    pub original_user_request: String,
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub previous_fingerprint: Option<String>,
    pub pending_question_prompt: Option<String>,
    pub pending_question_answer_summary: Option<String>,
    pub latest_summary_status: Option<String>,
}

/// 运行时内部会重复使用的上下文元信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextAssemblyMetadata {
    pub memory_key: String,
    pub history_session_id: String,
    pub opencode_session_id: String,
}

/// 装配上下文所需的最小输入。
#[derive(Debug, Clone, PartialEq)]
pub struct ContextAssemblyRequest {
    pub mode: ContextAssemblyMode,
    pub inbound_id: String,
    pub channel: String,
    pub chat_id: Option<String>,
    pub original_user_request: String,
    pub opencode_session_id: String,
    pub pending_question: Option<PendingQuestion>,
    pub latest_summary: Option<ExecutionSummary>,
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub previous_fingerprint: Option<String>,
    pub followup_prompt: Option<String>,
}

/// 统一上下文装配结果。
#[derive(Debug, Clone, PartialEq)]
pub struct ContextAssemblyResult {
    pub prompt_text: String,
    pub requirement_snapshot: RequirementContextSnapshot,
    pub metadata: ContextAssemblyMetadata,
}

pub fn assemble_context(request: ContextAssemblyRequest) -> ContextAssemblyResult {
    let history_session_id = request
        .chat_id
        .clone()
        .unwrap_or_else(|| request.inbound_id.clone());
    let memory_key = format!("{}:{}", request.channel, history_session_id);
    let pending_question_prompt = request
        .pending_question
        .as_ref()
        .map(|question| question.prompt.clone());
    let pending_question_answer_summary = request
        .pending_question
        .as_ref()
        .and_then(|question| question.answer_summary.clone());
    let latest_summary_status = request
        .latest_summary
        .as_ref()
        .map(|summary| summary.status.clone());

    ContextAssemblyResult {
        prompt_text: build_prompt_text(&request),
        requirement_snapshot: RequirementContextSnapshot {
            original_user_request: request.original_user_request,
            attempted_rounds: request.attempted_rounds,
            max_rounds: request.max_rounds,
            previous_fingerprint: request.previous_fingerprint,
            pending_question_prompt,
            pending_question_answer_summary,
            latest_summary_status,
        },
        metadata: ContextAssemblyMetadata {
            memory_key,
            history_session_id,
            opencode_session_id: request.opencode_session_id,
        },
    }
}

fn build_prompt_text(request: &ContextAssemblyRequest) -> String {
    if let Some(prompt) = request.followup_prompt.as_ref() {
        return prompt.clone();
    }

    match request.mode {
        ContextAssemblyMode::Initial => request.original_user_request.clone(),
        ContextAssemblyMode::QuestionContinuation => build_question_continuation_prompt(request),
        ContextAssemblyMode::SubtaskBlockedContinuation => request.original_user_request.clone(),
        ContextAssemblyMode::Followup => request.original_user_request.clone(),
    }
}

fn build_question_continuation_prompt(request: &ContextAssemblyRequest) -> String {
    let Some(question) = request.pending_question.as_ref() else {
        return request.original_user_request.clone();
    };

    match question.answer_summary.as_ref() {
        Some(answer_summary) if !answer_summary.trim().is_empty() => answer_summary.clone(),
        _ => request.original_user_request.clone(),
    }
}
