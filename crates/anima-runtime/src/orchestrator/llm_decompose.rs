use indexmap::IndexMap;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use super::core::{AgentOrchestrator, OrchestrationPlan, PlanProgress, SubTask};
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
        let executor = self.executor.as_ref()?;
        let client = self.client.as_ref()?;

        let prompt = format!(
            "你是任务分解引擎。分析用户请求，将其拆解为可并行/串行执行的子任务。\n\n\
             用户请求: {request}\n\n\
             请输出 JSON 数组，每个元素包含:\n\
             - name: 子任务名称（英文短横线命名）\n\
             - task_type: 类型（design/frontend/backend/testing/data-collection/analysis/generic）\n\
             - specialist_type: 专家类型（designer/frontend-dev/backend-dev/tester/data-engineer/analyst/default）\n\
             - dependencies: 依赖的子任务 name 数组\n\
             - description: 简短描述\n\n\
             只输出 JSON 数组，不要其他内容。如果请求太简单不需要拆解，输出空数组 []。"
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

        let json_str = extract_json_array(text)?;
        let specs: Vec<LlmSubtaskSpec> = serde_json::from_str(json_str).ok()?;
        if specs.is_empty() {
            return None;
        }
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
}

fn extract_json_array(text: &str) -> Option<&str> {
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
