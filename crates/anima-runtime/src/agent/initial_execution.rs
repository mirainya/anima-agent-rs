use serde_json::{json, Value};
use uuid::Uuid;

use crate::bus::InboundMessage;
use crate::execution::driver::{
    build_api_call_task, execute_api_call, ApiCallExecutionRequest, ExecutionKind,
};
use crate::support::{make_api_cache_key, now_ms};

use super::context_types::{InitialExecutionOutcome, InitialPlanDispatchContext};
use super::core::CoreAgent;
use super::runtime_error::{AgentError, RuntimeError};
use super::runtime_helpers::truncate_preview;
use super::runtime_ids::execution_kind_label;
use super::types::ExecutionPlan;
use super::types::{make_task_result, MakeTaskResult, TaskResult};

impl CoreAgent {
    pub(crate) fn execute_api_call_request(
        &self,
        inbound_msg: &InboundMessage,
        memory_key: &str,
        plan_type: &str,
        request: &ApiCallExecutionRequest,
        task_summary: &str,
        extra_payload: Value,
    ) -> Result<TaskResult, RuntimeError> {
        let task = build_api_call_task(request);
        let extra_payload_for_started = extra_payload.clone();
        let mut assigned_payload = json!({
            "memory_key": memory_key,
            "plan_type": plan_type,
            "task_id": task.id.clone(),
            "task_type": "api-call",
            "execution_kind": execution_kind_label(request.kind),
            "task_summary": task_summary,
            "task_preview": truncate_preview(&request.content, 160),
            "opencode_session_id": request.session_id.clone(),
        });
        if let Some(map) = assigned_payload.as_object_mut() {
            if let Some(extra) = extra_payload.as_object() {
                for (key, value) in extra {
                    map.insert(key.clone(), value.clone());
                }
            }
        }
        self.emitter
            .publish("worker_task_assigned", inbound_msg, assigned_payload);
        self.emitter.publish(
            "api_call_started",
            inbound_msg,
            json!({
                "memory_key": memory_key,
                "plan_type": plan_type,
                "task_id": task.id.clone(),
                "task_type": "api-call",
                "execution_kind": execution_kind_label(request.kind),
                "request_preview": truncate_preview(&request.content, 160),
                "opencode_session_id": request.session_id.clone(),
                "question_id": extra_payload_for_started
                    .get("question_id")
                    .cloned()
                    .unwrap_or(Value::Null),
            }),
        );
        execute_api_call(&self.worker_pool, request.clone())
    }

    pub(crate) fn publish_initial_plan_dispatch_events(&self, ctx: InitialPlanDispatchContext<'_>) {
        let plan_type = ctx.plan.plan_type.clone();
        if ctx.plan.tasks.len() == 1 {
            if let Some(task) = ctx.plan.tasks.front() {
                let task_preview = task
                    .payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|text| truncate_preview(text, 160));
                self.emitter.publish(
                    "worker_task_assigned",
                    ctx.inbound_msg,
                    json!({
                        "memory_key": ctx.memory_key,
                        "plan_type": plan_type.clone(),
                        "task_id": task.id.clone(),
                        "task_type": task.task_type.clone(),
                        "execution_kind": execution_kind_label(ctx.execution_kind),
                        "task_summary": "主 agent 已将任务派发给 worker",
                        "task_preview": task_preview,
                        "opencode_session_id": ctx.opencode_session_id,
                    }),
                );
                if task.task_type == "api-call" {
                    self.emitter.publish(
                        "api_call_started",
                        ctx.inbound_msg,
                        json!({
                            "memory_key": ctx.memory_key,
                            "plan_type": plan_type,
                            "task_id": task.id.clone(),
                            "task_type": task.task_type.clone(),
                            "execution_kind": execution_kind_label(ctx.execution_kind),
                            "request_preview": task_preview,
                            "opencode_session_id": ctx.opencode_session_id,
                        }),
                    );
                }
            }
        }
    }

    pub(crate) fn execute_initial_plan_run(
        &self,
        inbound_msg: &InboundMessage,
        plan: &ExecutionPlan,
        opencode_session_id: &str,
    ) -> TaskResult {
        if !self.tool_registry.is_empty() {
            self.run_agentic_loop_for_plan(inbound_msg, opencode_session_id)
        } else {
            self.orchestrator
                .execute_plan_instance(plan, opencode_session_id)
        }
    }

    pub(crate) fn finalize_initial_plan_execution(
        &self,
        inbound_msg: &InboundMessage,
        plan_type: &str,
        result: &TaskResult,
    ) {
        self.emitter.publish_worker(
            "task_end",
            inbound_msg,
            &self.worker_pool.status().workers,
            plan_type,
        );
        if result.status == "success" {
            self.metrics.counter_inc("tasks_completed");
        } else {
            self.metrics.counter_inc("tasks_failed");
        }
    }

    pub(crate) fn execute_initial_plan(
        &self,
        inbound_msg: &InboundMessage,
        plan: &ExecutionPlan,
        opencode_session_id: &str,
        memory_key: &str,
        execution_kind: ExecutionKind,
    ) -> TaskResult {
        let plan_type = plan.plan_type.clone();
        self.publish_initial_plan_dispatch_events(InitialPlanDispatchContext {
            inbound_msg,
            plan,
            opencode_session_id,
            memory_key,
            execution_kind,
        });

        self.metrics.counter_inc("tasks_submitted");
        self.emitter.publish_worker(
            "task_start",
            inbound_msg,
            &self.worker_pool.status().workers,
            &plan_type,
        );

        let result = self.execute_initial_plan_run(inbound_msg, plan, opencode_session_id);
        self.finalize_initial_plan_execution(inbound_msg, &plan_type, &result);
        result
    }

    pub(crate) fn execute_initial_message_plan(
        &self,
        inbound_msg: &InboundMessage,
        plan: &ExecutionPlan,
        opencode_session_id: &str,
        key: &str,
    ) -> InitialExecutionOutcome {
        let cache_key = make_api_cache_key(opencode_session_id, &inbound_msg.content);
        let execute_started = now_ms();
        let mut cache_hit = false;

        // 尝试 LLM 分解：编排器内部判断是否需要拆分
        let orch_result = self.orchestrator.execute_orchestration_for_main_chain(
            &inbound_msg.content,
            &inbound_msg.id,
            &inbound_msg.id,
            opencode_session_id,
            |event, payload| self.emitter.publish(event, inbound_msg, payload),
        );

        let (plan_type, result) = match orch_result {
            Ok(execution) if execution.was_decomposed => {
                self.emitter.publish(
                    "orchestration_selected",
                    inbound_msg,
                    json!({
                        "memory_key": key,
                        "plan_type": plan.plan_type.clone(),
                        "selected_plan_type": "orchestration-v1",
                        "reason": "llm_decomposed",
                    }),
                );
                ("orchestration-v1".to_string(), execution.result)
            }
            Err(ref error) if !matches!(error, AgentError::OrchestrationNoDecomposition) => {
                let orch_plan_type = "orchestration-v1".to_string();
                self.emitter.publish(
                    "orchestration_selected",
                    inbound_msg,
                    json!({
                        "memory_key": key,
                        "plan_type": plan.plan_type.clone(),
                        "selected_plan_type": orch_plan_type,
                        "reason": "llm_decomposed",
                    }),
                );
                self.emitter.publish(
                    "orchestration_fallback",
                    inbound_msg,
                    json!({
                        "memory_key": key,
                        "plan_type": orch_plan_type,
                        "fallback_plan_type": plan.plan_type.clone(),
                        "reason": error.to_string(),
                    }),
                );
                let result = self.execute_initial_plan(
                    inbound_msg,
                    plan,
                    opencode_session_id,
                    key,
                    ExecutionKind::Initial,
                );
                (plan.plan_type.clone(), result)
            }
            Ok(_) | Err(_) => {
                // LLM 判断不需要拆分或分解失败 → 走原有 single 路径
                let plan_type = plan.plan_type.clone();
                let result = if plan_type == "single" {
                    if let Some(cached) = self.result_cache.get(&cache_key) {
                        cache_hit = true;
                        self.metrics.counter_inc("cache_hits");
                        self.emitter.publish(
                            "cache_hit",
                            inbound_msg,
                            json!({
                                "memory_key": key,
                                "cache_key": cache_key.clone(),
                                "plan_type": plan_type.clone(),
                            }),
                        );
                        make_task_result(MakeTaskResult {
                            task_id: Uuid::new_v4().to_string(),
                            trace_id: inbound_msg.id.clone(),
                            status: "success".into(),
                            result: Some(cached),
                            error: None,
                            duration_ms: 0,
                            worker_id: None,
                        })
                    } else {
                        self.metrics.counter_inc("cache_misses");
                        self.emitter.publish(
                            "cache_miss",
                            inbound_msg,
                            json!({
                                "memory_key": key,
                                "cache_key": cache_key.clone(),
                                "plan_type": plan_type.clone(),
                            }),
                        );
                        let result = self.execute_initial_plan(
                            inbound_msg,
                            plan,
                            opencode_session_id,
                            key,
                            ExecutionKind::Initial,
                        );
                        if result.status == "success" {
                            if let Some(value) = result.result.clone() {
                                self.result_cache.set(&cache_key, value, None);
                            }
                        }
                        result
                    }
                } else {
                    self.execute_initial_plan(
                        inbound_msg,
                        plan,
                        opencode_session_id,
                        key,
                        ExecutionKind::Initial,
                    )
                };
                (plan_type, result)
            }
        };

        InitialExecutionOutcome {
            plan_type,
            cache_hit,
            execute_ms: now_ms().saturating_sub(execute_started),
            result,
        }
    }
}
