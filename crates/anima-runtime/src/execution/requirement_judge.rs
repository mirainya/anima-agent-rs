use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::debug;

use crate::agent::{
    classify_question_requires_user_confirmation, detect_pending_question, extract_response_text,
    PendingQuestion, QuestionKind,
};
use crate::messages::types::ContentBlock;
use crate::provider::{ChatMessage, ChatRequest, ChatRole, Provider};

const DEFAULT_MAX_FOLLOWUP_ROUNDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum RequirementJudgement {
    Satisfied,
    NeedsAgentFollowup(Box<AgentFollowupPlan>),
    NeedsUserInput(Box<UserInputRequirement>),
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

pub fn judge_requirement(
    ctx: &RequirementJudgeContext,
    provider: Option<&dyn Provider>,
) -> RequirementJudgement {
    let question = detect_pending_question(ctx.raw_result.as_ref(), &ctx.opencode_session_id);
    if let Some(question) = question {
        if question.requires_user_confirmation
            || matches!(
                question.question_kind,
                QuestionKind::Input | QuestionKind::Confirm
            )
            || classify_question_requires_user_confirmation(&question.prompt, &question.options)
        {
            return RequirementJudgement::NeedsUserInput(Box::new(UserInputRequirement {
                reason: "上游返回了需要外部用户信息的结构化问题".into(),
                missing_requirements: vec![question.prompt.clone()],
                pending_question: question,
            }));
        }

        let fingerprint = fingerprint_result(ctx.raw_result.as_ref());
        return RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
            reason: "上游返回了可由主 agent 继续处理的结构化问题".into(),
            missing_requirements: vec![question.prompt.clone()],
            followup_prompt: build_question_followup_prompt(&ctx.original_user_request, &question),
            result_fingerprint: fingerprint,
        }));
    }

    let response_text = extract_response_text(ctx.raw_result.as_ref());
    let normalized = normalize_text(&response_text);
    if response_text.trim().is_empty() {
        return RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
            reason: "上游成功返回，但缺少可交付结果".into(),
            missing_requirements: vec!["给出可验证的最终结果或明确的下一步结论".into()],
            followup_prompt: build_result_followup_prompt(
                &ctx.original_user_request,
                &response_text,
                "请继续推进，直到明确说明需求是否已经完成；若仍缺信息，只在确实必须依赖用户时提出结构化 question。",
            ),
            result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
        }));
    }

    let repeated = ctx
        .previous_fingerprint
        .as_ref()
        .map(|previous| previous == &fingerprint_result(ctx.raw_result.as_ref()))
        .unwrap_or(false);
    if repeated {
        return RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
            reason: "自动 follow-up 得到了重复结果，尚未收敛".into(),
            missing_requirements: vec!["避免重复前一轮输出，继续给出真正推进结果".into()],
            followup_prompt: build_result_followup_prompt(
                &ctx.original_user_request,
                &response_text,
                "你刚刚重复了前一轮结果。请基于原始需求继续推进，输出新增结论；如果必须依赖用户，请返回结构化 question。",
            ),
            result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
        }));
    }

    // Layer 1: structured <completion_status> signal
    if let Some(status) = extract_completion_status(&response_text) {
        debug!(status, "requirement judge: completion_status tag found");
        match status {
            "complete" => return RequirementJudgement::Satisfied,
            "needs_input" | "partial" => {
                return RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
                    reason: format!("completion_status={status}，任务尚未完成"),
                    missing_requirements: infer_missing_requirements(&response_text),
                    followup_prompt: build_result_followup_prompt(
                        &ctx.original_user_request,
                        &response_text,
                        "请继续推进，直到任务完整完成。",
                    ),
                    result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
                }));
            }
            _ => {} // unrecognized value, fall through
        }
    }

    // Layer 2: lightweight LLM judgement
    if let Some(provider) = provider {
        if let Some(satisfied) =
            llm_judge_requirement(provider, &ctx.original_user_request, &response_text)
        {
            if satisfied {
                debug!("requirement judge: LLM says satisfied");
                return RequirementJudgement::Satisfied;
            } else {
                debug!("requirement judge: LLM says unsatisfied");
                return RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
                    reason: "LLM 评估判定需求尚未满足".into(),
                    missing_requirements: infer_missing_requirements(&response_text),
                    followup_prompt: build_result_followup_prompt(
                        &ctx.original_user_request,
                        &response_text,
                        "请继续推进，直到完整满足用户的原始请求。",
                    ),
                    result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
                }));
            }
        }
        debug!("requirement judge: LLM call failed, falling back to heuristics");
    }

    // Layer 3: keyword heuristics (original logic)
    if looks_unsatisfied(&normalized) {
        return RequirementJudgement::NeedsAgentFollowup(Box::new(AgentFollowupPlan {
            reason: "上游回复表明需求尚未真正满足".into(),
            missing_requirements: infer_missing_requirements(&response_text),
            followup_prompt: build_result_followup_prompt(
                &ctx.original_user_request,
                &response_text,
                "请不要只描述限制或保守结论。若能继续分析/执行，请继续推进；只有确实缺少用户外部信息时才返回结构化 question。",
            ),
            result_fingerprint: fingerprint_result(ctx.raw_result.as_ref()),
        }));
    }

    RequirementJudgement::Satisfied
}

fn extract_completion_status(text: &str) -> Option<&str> {
    let open = "<completion_status>";
    let close = "</completion_status>";
    let start = text.rfind(open)? + open.len();
    let end = start + text[start..].find(close)?;
    let value = text[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn llm_judge_requirement(
    provider: &dyn Provider,
    original_request: &str,
    response_text: &str,
) -> Option<bool> {
    let preview: String = response_text.chars().take(500).collect();
    let prompt = format!(
        "你是需求完成度评估器。判断以下 AI 回复是否完整满足了用户的原始请求。\n\n\
         用户请求：{original_request}\n\n\
         AI 回复摘要（前 500 字）：{preview}\n\n\
         只回复 JSON：{{\"satisfied\": true/false, \"reason\": \"一句话原因\"}}"
    );
    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }],
        max_tokens: Some(128),
        metadata: json!({}),
        ..Default::default()
    };
    let response = provider.chat(request).ok()?;
    let text = response.text();
    // extract first JSON object
    let start = text.find('{')?;
    let end = start + text[start..].find('}')? + 1;
    let obj: Value = serde_json::from_str(&text[start..end]).ok()?;
    obj.get("satisfied").and_then(|v| v.as_bool())
}

pub fn fingerprint_result(result: Option<&Value>) -> String {
    let canonical = result.cloned().unwrap_or(Value::Null).to_string();
    let compact = canonical
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if compact.len() > 240 {
        compact.chars().take(240).collect()
    } else {
        compact
    }
}

fn build_question_followup_prompt(
    original_user_request: &str,
    question: &PendingQuestion,
) -> String {
    let options = if question.options.is_empty() {
        String::new()
    } else {
        format!("\n可选项: {}", question.options.join(", "))
    };
    format!(
        "用户原始请求: {original_user_request}\n\n\
         上游提出了一个问题: {}{options}\n\n\
         请根据用户原始请求的上下文，尝试推断出合理答案并继续执行。\
         如果确实无法推断，请返回结构化 question 给用户。",
        question.prompt
    )
}

fn build_result_followup_prompt(
    original_user_request: &str,
    response_text: &str,
    instruction: &str,
) -> String {
    let preview: String = response_text.chars().take(300).collect();
    format!(
        "用户原始请求: {original_user_request}\n\n\
         上一轮结果摘要: {preview}\n\n\
         {instruction}"
    )
}

fn normalize_text(text: &str) -> String {
    text.to_ascii_lowercase()
        .replace('，', ",")
        .replace('。', ".")
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
        "could you provide",
        "could you clarify",
        "please provide",
        "需要更多信息",
        "请提供",
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
    if response_text.contains("信息") || response_text.to_ascii_lowercase().contains("information")
    {
        items.push("当前结果没有形成足够完整的最终答复".into());
    }
    if items.is_empty() {
        items.push("当前回复仍停留在中间态，未明确满足原始需求".into());
    }
    items
}

pub fn requirement_unsatisfied_payload(
    reason: &str,
    missing_requirements: &[String],
    raw_result: Option<&Value>,
) -> Value {
    json!({
        "reason": reason,
        "missing_requirements": missing_requirements,
        "raw_result": raw_result.cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_completion_status_complete() {
        let text = "Here is the answer.\n<completion_status>complete</completion_status>";
        assert_eq!(extract_completion_status(text), Some("complete"));
    }

    #[test]
    fn extract_completion_status_partial() {
        let text = "Still working...\n<completion_status>partial</completion_status>";
        assert_eq!(extract_completion_status(text), Some("partial"));
    }

    #[test]
    fn extract_completion_status_missing() {
        assert_eq!(extract_completion_status("no tag here"), None);
    }

    #[test]
    fn extract_completion_status_uses_last_occurrence() {
        let text = "<completion_status>partial</completion_status> then <completion_status>complete</completion_status>";
        assert_eq!(extract_completion_status(text), Some("complete"));
    }
}
