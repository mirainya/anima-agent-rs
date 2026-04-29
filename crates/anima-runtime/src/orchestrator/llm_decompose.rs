use indexmap::IndexMap;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

use super::core::{AgentOrchestrator, OrchestrationPlan, PlanProgress, SubTask};
use crate::messages::types::ContentBlock;
use crate::provider::{ChatMessage, ChatRequest, ChatRole};
use crate::support::now_ms;

#[derive(Debug, Deserialize)]
pub(crate) struct LlmSubtaskSpec {
    name: String,
    #[serde(default = "default_task_type")]
    task_type: String,
    #[serde(default = "default_specialist")]
    specialist_type: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    description: String,
}

fn default_task_type() -> String {
    "generic".into()
}
fn default_specialist() -> String {
    "default".into()
}

impl AgentOrchestrator {
    pub(crate) fn try_llm_decompose(
        &self,
        request: &str,
        session_id: &str,
    ) -> Option<Vec<LlmSubtaskSpec>> {
        let provider = match self.provider.as_ref() {
            Some(p) => p,
            None => {
                warn!("[llm_decompose] no provider configured, skipping decomposition");
                return None;
            }
        };

        let prompts = self.prompts.read();
        let prompt = prompts.task_decompose.replace("{request}", request);
        debug!(
            "[llm_decompose] sending decompose request, prompt_len={}",
            prompt.len()
        );

        let chat_request = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: vec![ContentBlock::Text { text: prompt }],
            }],
            max_tokens: Some(4096),
            metadata: json!({ "session_id": session_id }),
            ..Default::default()
        };
        let response = match provider.chat(chat_request) {
            Ok(r) => r,
            Err(e) => {
                warn!("[llm_decompose] provider.chat failed: {e}");
                return None;
            }
        };

        let text = response
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("")
            .trim();

        debug!(
            "[llm_decompose] raw LLM response (first 500 chars): {}",
            &text[..text.len().min(500)]
        );

        let json_str = match extract_json_array(text) {
            Some(s) => s,
            None => {
                warn!(
                    "[llm_decompose] failed to extract JSON array from response: {}",
                    &text[..text.len().min(200)]
                );
                return None;
            }
        };

        let specs: Vec<LlmSubtaskSpec> = match serde_json::from_str(json_str) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "[llm_decompose] JSON parse failed: {e}, json_str: {}",
                    &json_str[..json_str.len().min(300)]
                );
                return None;
            }
        };

        if specs.is_empty() {
            debug!("[llm_decompose] LLM returned empty array, no decomposition needed");
            return None;
        }

        let count = specs.len().min(6);
        debug!(
            "[llm_decompose] decomposed into {count} subtasks: {:?}",
            specs.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        let mut specs = specs;
        specs.truncate(6);
        Some(specs)
    }

    pub(crate) fn build_plan_from_llm_specs(
        &self,
        specs: Vec<LlmSubtaskSpec>,
        request: &str,
        trace_id: &str,
        parent_job_id: &str,
    ) -> OrchestrationPlan {
        let plan_id = Uuid::new_v4().to_string();
        let mut subtasks: IndexMap<String, Arc<SubTask>> = IndexMap::new();

        for spec in &specs {
            let sub_id = Uuid::new_v4().to_string();
            let subtask = Arc::new(SubTask {
                id: sub_id,
                parent_id: plan_id.clone(),
                parent_job_id: parent_job_id.to_string(),
                trace_id: trace_id.to_string(),
                name: spec.name.clone(),
                task_type: spec.task_type.clone(),
                description: if spec.description.is_empty() {
                    format!("{} for: {}", spec.name, request)
                } else {
                    spec.description.clone()
                },
                dependencies: spec.dependencies.iter().cloned().collect(),
                priority: 5,
                specialist_type: spec.specialist_type.clone(),
                payload: json!({
                    "request": request,
                    "subtask": spec.name,
                    "rule": "llm-decompose",
                }),
                result: Mutex::new(None),
                started_at: Mutex::new(None),
                completed_at: Mutex::new(None),
            });
            subtasks.insert(spec.name.clone(), subtask);
        }

        let execution_order = Self::topological_sort(&subtasks);
        let parallel_groups = Self::compute_parallel_groups(&subtasks, &execution_order);
        let total = subtasks.len() as u32;

        OrchestrationPlan {
            id: plan_id,
            trace_id: trace_id.to_string(),
            parent_job_id: parent_job_id.to_string(),
            original_request: request.to_string(),
            matched_rule: Some("llm-decompose".into()),
            subtasks,
            execution_order,
            parallel_groups,
            progress: Mutex::new(PlanProgress {
                completed_count: 0,
                total_count: total,
                failed_count: 0,
            }),
            created_at: now_ms(),
        }
    }

    /// Build an OrchestrationPlan from a pre-parsed JSON array string.
    /// Used as a fallback when the agentic loop output contains subtask specs.
    pub fn build_plan_from_parsed_specs(
        &self,
        json_str: &str,
        request: &str,
        trace_id: &str,
        parent_job_id: &str,
    ) -> Option<OrchestrationPlan> {
        let mut specs: Vec<LlmSubtaskSpec> = serde_json::from_str(json_str).ok()?;
        if specs.is_empty() {
            return None;
        }
        specs.truncate(6);
        Some(self.build_plan_from_llm_specs(specs, request, trace_id, parent_job_id))
    }
}

pub(crate) fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let mut depth = 0;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
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
