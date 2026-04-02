use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::{
    classify_question_requires_user_confirmation, detect_pending_question, extract_response_text,
    PendingQuestion, QuestionKind,
};

const DEFAULT_MAX_FOLLOWUP_ROUNDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum RequirementJudgement {
    Satisfied,
    NeedsAgentFollowup(AgentFollowupPlan),
    NeedsUserInput(UserInputRequirement),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementJudgeContext {
    pub original_user_request: String,
    pub job_id: String,
    pub trace_id: String,
    pub chat_id: Option<String>,
    pub opencode_session_id: String,
    pub raw_result: Option<Value>,
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub previous_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentFollowupPlan {
    pub reason: String,
    pub missing_requirements: Vec<String>,
    pub followup_prompt: String,
    pub result_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInputRequirement {
    pub reason: String,
    pub missing_requirements: Vec<String>,
    pub pending_question: PendingQuestion,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequirementProgressState {
    pub original_user_request: String,
    pub attempted_rounds: usize,
    pub max_rounds: usize,
    pub last_result_fingerprint: Option<String>,
    pub last_reason: Option<String>,
}

impl RequirementProgressState {
    pub fn new(original_user_request: String) -> Self {
        Self {
            original_user_request,
            attempted_rounds: 0,
            max_rounds: DEFAULT_MAX_FOLLOWUP_ROUNDS,
            last_result_fingerprint: None,
            last_reason: None,
        }
    }
}

pub fn judge_requirement(ctx: &RequirementJudgeContext) -> RequirementJudgement {
    let question = detect_pending_question(ctx.raw_result.as_ref(), &ctx.opencode_session_id);
    if let Some(question) = question {
        if question.requires_user_confirmation
            || matches!(question.question_kind, QuestionKind::Input)
            || classify_question_requires_user_confirmation(&question.prompt, &question.options)
        {
            return RequirementJudgement::NeedsUserInput(UserInputRequirement {
                reason: "上游返回了需要外部用户信息的结构化问题".into(),
                missing_requirements: vec![question.prompt.clone()],
                pending_question: question,
            });
        }

        let fingerprint = fingerprint_result(ctx.raw_result.as_ref());
        return RequirementJudgement::NeedsAgentFollowup(AgentFollowupPlan {
            reason: "上游返回了可由主 agent 继续处理的结构化问题".into(),
            missing_requirements: vec![question.prompt.clone()],
            followup_prompt: build_question_followup_prompt(&ctx.original_user_request, &question),
            result_fingerprint: fingerprint,
        });
    }

    let response_text = extract_response_text(ctx.raw_result.as_ref());
    let normalized = normalize_text(&response_text);
    if response_text.trim().is_empty() {
        return RequirementJudgement::NeedsAgentFollowup(AgentFollowupPlan {
            reason: "上游成功返回，但缺少可交付结果".into(),
            missing_requirements: vec!["给出可验证的最终结果或明确的下一步结论".into()],
            followup_prompt: build_result_followup_prompt(
                &ctx.original_user_request,
                &response_text,
                "请继续推进，直到明确说明需求是否已经完成；若仍缺信息，只在确实必须依赖用户时提出结构化 question。",
            ),
            result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
        });
    }

    let repeated = ctx
        .previous_fingerprint
        .as_ref()
        .map(|previous| previous == &fingerprint_result(ctx.raw_result.as_ref()))
        .unwrap_or(false);
    if repeated {
        return RequirementJudgement::NeedsAgentFollowup(AgentFollowupPlan {
            reason: "自动 follow-up 得到了重复结果，尚未收敛".into(),
            missing_requirements: vec!["避免重复前一轮输出，继续给出真正推进结果".into()],
            followup_prompt: build_result_followup_prompt(
                &ctx.original_user_request,
                &response_text,
                "你刚刚重复了前一轮结果。请基于原始需求继续推进，输出新增结论；如果必须依赖用户，请返回结构化 question。",
            ),
            result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
        });
    }

    if looks_unsatisfied(&normalized) {
        return RequirementJudgement::NeedsAgentFollowup(AgentFollowupPlan {
            reason: "上游回复表明需求尚未真正满足".into(),
            missing_requirements: infer_missing_requirements(&response_text),
            followup_prompt: build_result_followup_prompt(
                &ctx.original_user_request,
                &response_text,
                "请不要只描述限制或保守结论。若能继续分析/执行，请继续推进；只有确实缺少用户外部信息时才返回结构化 question。",
            ),
            result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
        });
    }

    RequirementJudgement::Satisfied
}

pub fn fingerprint_result(result: Option<&Value>) -> String {
    let canonical = result.cloned().unwrap_or(Value::Null).to_string();
    let compact = canonical.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
    if compact.len() > 240 {
        compact.chars().take(240).collect()
    } else {
        compact
    }
}

fn build_question_followup_prompt(original_user_request: &str, question: &PendingQuestion) -> String {
    let options = if question.options.is_empty() {
        "(无 options)".to_string()
    } else {
        question.options.join(" / ")
    };
    format!(
        "原始用户请求：\n{original_user_request}\n\n上游返回了结构化 question，但该问题目前不需要用户介入：\n- prompt: {}\n- options: {}\n\n请由主 agent 自主完成判断并继续推进，直接产出更接近完成态的结果。只有确实需要用户提供新的外部信息时，才返回结构化 question。",
        question.prompt,
        options,
    )
}

fn build_result_followup_prompt(original_user_request: &str, response_text: &str, instruction: &str) -> String {
    format!(
        "原始用户请求：\n{original_user_request}\n\n上一轮上游结果：\n{response_text}\n\n{instruction}"
    )
}

fn normalize_text(text: &str) -> String {
    text.to_ascii_lowercase().replace('，', ",").replace('。', ".")
}

fn looks_unsatisfied(text: &str) -> bool {
    let hints = [
        "need more info",
        "need more information",
        "need user input",
        "need confirmation",
        "please confirm",
        "please choose",
        "i need",
        "cannot complete",
        "can't complete",
        "unable to complete",
        "not enough information",
        "requires user",
        "需要更多信息",
        "需要更多资料",
        "需要用户输入",
        "需要用户确认",
        "请确认",
        "请选择",
        "无法完成",
        "不能完成",
        "信息不足",
        "还不能",
        "继续判断",
        "继续处理",
    ];

    hints.iter().any(|hint| text.contains(hint))
}

fn infer_missing_requirements(response_text: &str) -> Vec<String> {
    let mut items = Vec::new();
    if response_text.contains("确认") || response_text.to_ascii_lowercase().contains("confirm") {
        items.push("缺少对关键决策的明确确认".into());
    }
    if response_text.contains("选择") || response_text.to_ascii_lowercase().contains("choose") {
        items.push("缺少具体选择或继续执行所需分支判断".into());
    }
    if response_text.contains("信息") || response_text.to_ascii_lowercase().contains("information") {
        items.push("当前结果没有形成足够完整的最终答复".into());
    }
    if items.is_empty() {
        items.push("当前回复仍停留在中间态，未明确满足原始需求".into());
    }
    items
}

pub fn requirement_unsatisfied_payload(reason: &str, missing_requirements: &[String], raw_result: Option<&Value>) -> Value {
    json!({
        "reason": reason,
        "missing_requirements": missing_requirements,
        "raw_result": raw_result.cloned(),
    })
}
