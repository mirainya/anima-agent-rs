use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::core::{AgentOrchestrator, LoweredTask, OrchestrationPlan};
use crate::messages::types::ContentBlock;
use crate::provider::{ChatMessage, ChatRequest, ChatRole, Provider};

#[derive(Debug, Deserialize)]
struct LlmContextInferResult {
    needs_question: bool,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    options: Vec<String>,
}

pub fn try_llm_infer_missing_context(
    provider: &Arc<dyn Provider>,
    session_id: &str,
    plan: &OrchestrationPlan,
    lowered_tasks: &[LoweredTask],
    subtask_results: &Value,
    prompt_template: &str,
) -> Option<Value> {
    let mut result_summaries = Vec::new();
    for lowered in lowered_tasks {
        let text = subtask_results
            .get(&lowered.name)
            .and_then(|e| e.get("result"))
            .map(|r| AgentOrchestrator::extract_result_text(Some(r)))
            .unwrap_or_default();
        if !text.is_empty() {
            result_summaries.push(format!("[{}]: {}", lowered.name, text));
        }
    }

    if result_summaries.is_empty() {
        return None;
    }

    let joined = result_summaries.join("\n\n");
    let prompt = prompt_template
        .replace("{request}", &plan.original_request)
        .replace("{results}", &joined);

    let chat_request = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }],
        metadata: json!({ "session_id": session_id }),
        ..Default::default()
    };
    let response = provider.chat(chat_request).ok()?;

    let text = response
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("")
        .trim();

    let json_str = extract_json_object(text)?;
    let infer: LlmContextInferResult = serde_json::from_str(json_str).ok()?;

    if !infer.needs_question || infer.prompt.is_empty() {
        return None;
    }

    Some(json!({
        "type": "question",
        "question": {
            "id": format!("orchestration-context-{}", plan.id),
            "kind": "input",
            "prompt": infer.prompt,
            "options": infer.options,
            "orchestration": {
                "reason": "llm_inferred_missing_context",
                "subtasks": lowered_tasks.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            }
        }
    }))
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}
