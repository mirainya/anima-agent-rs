use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::core::{AgentOrchestrator, LoweredTask, OrchestrationPlan};
use crate::worker::executor::TaskExecutor;
use anima_sdk::facade::Client as SdkClient;

#[derive(Debug, Deserialize)]
struct LlmContextInferResult {
    needs_question: bool,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    options: Vec<String>,
}

pub fn try_llm_infer_missing_context(
    executor: &Arc<dyn TaskExecutor>,
    client: &SdkClient,
    session_id: &str,
    plan: &OrchestrationPlan,
    lowered_tasks: &[LoweredTask],
    subtask_results: &Value,
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
    let prompt = format!(
        "你是上下文完整性分析器。以下是多个子任务的执行结果，判断是否有多个子任务因缺少共享上下文信息而无法继续。\n\n\
         原始请求: {}\n\n\
         子任务结果:\n{}\n\n\
         请输出 JSON 对象:\n\
         - needs_question: bool — 是否需要向用户追问\n\
         - prompt: string — 追问内容（简洁精准，一个问题）\n\
         - options: string[] — 3-5 个常见选项\n\n\
         如果子任务结果充分、不需要追问，设 needs_question 为 false。\n\
         只输出 JSON 对象，不要其他内容。",
        plan.original_request, joined
    );

    let content = json!([{"type": "text", "text": prompt}]);
    let result = executor.send_prompt(client, session_id, content).ok()?;

    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .or_else(|| result.get("text").and_then(Value::as_str))
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
