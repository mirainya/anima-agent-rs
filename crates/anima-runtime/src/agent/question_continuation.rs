use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::bus::InboundMessage;
use crate::execution::agentic_loop::{
    continue_agentic_loop, resume_suspended_tool_invocation, AgenticLoopOutcome,
};
use crate::execution::context_assembly::ContextAssemblyMode;
use crate::execution::driver::{ApiCallExecutionRequest, ExecutionKind};
use crate::prompt::PromptAssembler;
use crate::support::now_ms;
use crate::tasks::{SuspensionKind, SuspensionStatus, TaskStatus};

use super::core::{
    memory_key, CoreAgent, ExecutionContext, RuntimeTaskPhase, SuccessSource,
    ToolPermissionResumePreparation,
};
use super::runtime_helpers::truncate_preview;
use super::runtime_ids::runtime_task_id;
use super::suspension::{
    permission_risk_to_question_risk, question_kind_str, question_risk_level_str, PendingQuestion,
    PendingQuestionSourceKind, QuestionAnswerInput, QuestionDecisionMode, QuestionKind,
    SuspendedToolInvocationState,
};
use super::types::{make_task_result, MakeTaskResult};

impl CoreAgent {
    pub(crate) fn continue_upstream_question_answer(
        &self,
        inbound: &InboundMessage,
        key: &str,
        pending: &PendingQuestion,
    ) -> Result<PendingQuestion, String> {
        let continuation_ctx = self.assemble_turn_context(
            inbound,
            ContextAssemblyMode::QuestionContinuation,
            pending.opencode_session_id.clone(),
            Some(pending.clone()),
            None,
        );
        let continuation_request = ApiCallExecutionRequest {
            trace_id: inbound.id.clone(),
            session_id: continuation_ctx.metadata.opencode_session_id.clone(),
            content: continuation_ctx.prompt_text.clone(),
            kind: ExecutionKind::QuestionContinuation,
            metadata: Some(json!({})),
        };
        let continuation_result = self.execute_api_call_request(
            inbound,
            key,
            "single",
            &continuation_request,
            "主 agent 已收到问题答案，派发给 worker 继续执行",
            json!({
                "question_id": pending.question_id,
            }),
        )?;

        if continuation_result.status != "success" {
            return Err(continuation_result
                .error
                .unwrap_or_else(|| "Continuation execution failed".to_string()));
        }

        let exec_ctx = ExecutionContext {
            memory_key: continuation_ctx.metadata.memory_key,
            history_session_id: continuation_ctx.metadata.history_session_id,
            opencode_session_id: continuation_ctx.metadata.opencode_session_id,
            plan_type: "single".into(),
            context_ms: 0,
            session_ms: 0,
            classify_ms: 0,
            execute_ms: continuation_result.duration_ms,
            total_ms: continuation_result.duration_ms,
            cache_hit: false,
        };
        let _ = self.handle_upstream_result(
            inbound,
            &exec_ctx,
            continuation_result,
            SuccessSource::QuestionContinuation,
            Some(pending),
        )?;
        Ok(pending.clone())
    }

    pub(crate) fn continue_tool_permission_answer(
        &self,
        job_id: &str,
        pending: &PendingQuestion,
        answer: &str,
    ) -> Result<PendingQuestion, String> {
        self.resume_suspended_tool_permission(job_id, pending.clone(), answer)?;
        Ok(pending.clone())
    }

    pub(crate) fn prepare_tool_permission_resume(
        &self,
        job_id: &str,
        pending: &PendingQuestion,
        answer: &str,
    ) -> Result<ToolPermissionResumePreparation, String> {
        let suspended = self
            .suspension
            .tool_invocation_state(&pending.question_id)
            .ok_or_else(|| {
                format!(
                    "Missing suspended tool invocation for question: {}",
                    pending.question_id
                )
            })?;
        let allow = matches!(answer, "allow" | "ALLOW" | "Allow");

        let inbound_for_tool_events = suspended.inbound.clone();
        let emitter_for_tool_events: Arc<super::event_emitter::RuntimeEventEmitter> =
            Arc::clone(&self.emitter);
        let tool_event_publisher = Arc::new(move |event: &str, payload: Value| {
            emitter_for_tool_events.publish_tool_lifecycle(
                event,
                &inbound_for_tool_events,
                payload,
            );
        });

        let config = crate::execution::agentic_loop::AgenticLoopConfig {
            max_iterations: 10,
            session_id: suspended.opencode_session_id.clone(),
            trace_id: suspended.inbound.id.clone(),
            system_prompt: Some({
                let mut asm = PromptAssembler::new();
                asm.add_text("identity", "你是 Anima 智能助手。", 0);
                asm.build().text
            }),
            tool_definitions: Some(self.tool_registry.tool_definitions()),
            streaming: true,
            on_stream_event: None,
            on_tool_lifecycle_event: Some(tool_event_publisher),
            ..Default::default()
        };

        let resumed_messages = resume_suspended_tool_invocation(
            &suspended.suspension,
            allow,
            &self.tool_registry,
            self.hook_registry.as_deref(),
            &config,
        )
        .map_err(|err| err.to_string())?;

        self.upsert_runtime_suspension(
            &suspended.inbound,
            pending,
            Some(runtime_task_id(job_id, RuntimeTaskPhase::ToolPermission)),
            Some(
                suspended
                    .suspension
                    .suspended_tool
                    .invocation
                    .invocation_id
                    .clone(),
            ),
            SuspensionKind::ToolPermission,
            SuspensionStatus::Resolved,
        );
        self.upsert_runtime_task(
            &suspended.inbound,
            RuntimeTaskPhase::ToolPermission,
            TaskStatus::Running,
            format!(
                "tool permission resolved for {}",
                suspended.suspension.suspended_tool.tool_name
            ),
            None,
        );
        self.emitter.publish(
            "tool_permission_resolved",
            &suspended.inbound,
            json!({
                "question_id": pending.question_id,
                "tool_use_id": suspended.suspension.suspended_tool.tool_use_id,
                "tool_name": suspended.suspension.suspended_tool.tool_name,
                "decision": if allow { "allow" } else { "deny" },
                "source": pending.resolution_source,
                "invocation_id": suspended.suspension.suspended_tool.invocation.invocation_id,
                "tool_invocation": self.emitter.tool_lifecycle_payload(
                    &suspended.suspension.suspended_tool.invocation,
                    json!({
                        "decision": if allow { "allow" } else { "deny" },
                        "source": pending.resolution_source,
                    }),
                ),
            }),
        );

        Ok(ToolPermissionResumePreparation {
            suspended,
            config,
            resumed_messages,
        })
    }

    pub(crate) fn finish_tool_permission_continuation(
        &self,
        job_id: &str,
        pending: &PendingQuestion,
        suspended: &SuspendedToolInvocationState,
        loop_result: &crate::execution::agentic_loop::AgenticLoopResult,
    ) -> Result<(), String> {
        self.append_transcript_messages(&suspended.inbound, &loop_result.messages);
        self.emitter.publish(
            "question_resolved",
            &suspended.inbound,
            json!({
                "memory_key": memory_key(&suspended.inbound),
                "question_id": pending.question_id,
                "question_kind": question_kind_str(&pending.question_kind),
                "answer_summary": pending.answer_summary,
                "resolution_source": pending.resolution_source,
                "opencode_session_id": pending.opencode_session_id,
            }),
        );
        self.suspension.clear_question(job_id);
        self.suspension.clear_tool_invocation(&pending.question_id);
        let _ = self.send_response(&suspended.inbound, &loop_result.final_text);
        self.emitter.publish(
            "message_completed",
            &suspended.inbound,
            json!({
                "memory_key": memory_key(&suspended.inbound),
                "plan_type": "single",
                "status": "success",
                "response_preview": truncate_preview(&loop_result.final_text, 160),
            }),
        );
        Ok(())
    }

    pub(crate) fn resuspend_tool_permission_continuation(
        &self,
        job_id: &str,
        pending: &PendingQuestion,
        suspended: SuspendedToolInvocationState,
        next_suspension: crate::execution::agentic_loop::AgenticLoopSuspension,
    ) -> Result<(), String> {
        self.append_transcript_messages(&suspended.inbound, &next_suspension.messages);
        let next_question_id = Uuid::new_v4().to_string();
        let raw_question = json!({
            "type": "tool_permission",
            "tool_name": next_suspension.suspended_tool.tool_name,
            "tool_use_id": next_suspension.suspended_tool.tool_use_id,
            "tool_input": next_suspension.suspended_tool.tool_input,
            "raw_input": next_suspension.suspended_tool.permission_request.raw_input,
            "prompt": next_suspension.suspended_tool.permission_request.prompt,
            "input_preview": next_suspension.suspended_tool.permission_request.input_preview,
            "invocation_id": next_suspension.suspended_tool.invocation.invocation_id,
            "opencode_session_id": suspended.opencode_session_id,
            "original_user_request": suspended.inbound.content,
            "sender_id": suspended.inbound.sender_id,
            "chat_id": suspended.inbound.chat_id,
            "channel": suspended.inbound.channel,
            "iterations": next_suspension.iterations,
            "compact_count": next_suspension.compact_count,
            "risk_level": question_risk_level_str(&permission_risk_to_question_risk(
                &next_suspension.suspended_tool.permission_request.risk_level,
            )),
            "tool_invocation": self.emitter.tool_lifecycle_payload(
                &next_suspension.suspended_tool.invocation,
                json!({"source": "resuspended"}),
            ),
        });
        let next_pending = PendingQuestion {
            question_id: next_question_id.clone(),
            job_id: job_id.to_string(),
            opencode_session_id: suspended.opencode_session_id.clone(),
            question_kind: QuestionKind::Confirm,
            prompt: next_suspension
                .suspended_tool
                .permission_request
                .prompt
                .clone(),
            options: vec!["allow".into(), "deny".into()],
            raw_question,
            decision_mode: QuestionDecisionMode::UserRequired,
            risk_level: permission_risk_to_question_risk(
                &next_suspension.suspended_tool.permission_request.risk_level,
            ),
            requires_user_confirmation: true,
            source_kind: PendingQuestionSourceKind::ToolPermission,
            continuation_token: Some(next_question_id.clone()),
            asked_at_ms: now_ms(),
            answer_submitted: false,
            answer_summary: None,
            resolution_source: None,
            inbound: Some(suspended.inbound.clone()),
        };
        self.suspension
            .store_question(job_id.to_string(), next_pending.clone());
        self.suspension.store_tool_invocation(
            next_question_id,
            SuspendedToolInvocationState {
                suspension: next_suspension,
                inbound: suspended.inbound.clone(),
                opencode_session_id: suspended.opencode_session_id,
            },
        );
        self.suspension.clear_tool_invocation(&pending.question_id);
        self.suspension.publish_question_asked(
            &|event: &str, msg: &InboundMessage, payload: Value| {
                self.emitter.publish(event, msg, payload);
            },
            &suspended.inbound,
            &memory_key(&suspended.inbound),
            &next_pending,
            None,
            0,
            0,
            false,
            "single",
        );
        Ok(())
    }

    pub(crate) fn handle_initial_tool_permission_suspension(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
        suspension: crate::execution::agentic_loop::AgenticLoopSuspension,
        started: u64,
    ) -> crate::agent::types::TaskResult {
        self.append_transcript_messages(inbound_msg, &suspension.messages);
        let question_id = Uuid::new_v4().to_string();
        let raw_question = json!({
            "type": "tool_permission",
            "tool_name": suspension.suspended_tool.tool_name,
            "tool_use_id": suspension.suspended_tool.tool_use_id,
            "tool_input": suspension.suspended_tool.tool_input,
            "raw_input": suspension.suspended_tool.permission_request.raw_input,
            "prompt": suspension.suspended_tool.permission_request.prompt,
            "input_preview": suspension.suspended_tool.permission_request.input_preview,
            "invocation_id": suspension.suspended_tool.invocation.invocation_id,
            "opencode_session_id": opencode_session_id,
            "original_user_request": inbound_msg.content,
            "sender_id": inbound_msg.sender_id,
            "chat_id": inbound_msg.chat_id,
            "channel": inbound_msg.channel,
            "iterations": suspension.iterations,
            "compact_count": suspension.compact_count,
            "risk_level": question_risk_level_str(&permission_risk_to_question_risk(
                &suspension.suspended_tool.permission_request.risk_level,
            )),
            "tool_invocation": self.emitter.tool_lifecycle_payload(
                &suspension.suspended_tool.invocation,
                json!({"source": "suspend"}),
            ),
        });
        let pending_question = PendingQuestion {
            question_id: question_id.clone(),
            job_id: inbound_msg.id.clone(),
            opencode_session_id: opencode_session_id.to_string(),
            question_kind: QuestionKind::Confirm,
            prompt: suspension.suspended_tool.permission_request.prompt.clone(),
            options: vec!["allow".into(), "deny".into()],
            raw_question,
            decision_mode: QuestionDecisionMode::UserRequired,
            risk_level: permission_risk_to_question_risk(
                &suspension.suspended_tool.permission_request.risk_level,
            ),
            requires_user_confirmation: true,
            source_kind: PendingQuestionSourceKind::ToolPermission,
            continuation_token: Some(question_id.clone()),
            asked_at_ms: now_ms(),
            answer_submitted: false,
            answer_summary: None,
            resolution_source: None,
            inbound: Some(inbound_msg.clone()),
        };
        self.suspension
            .store_question(inbound_msg.id.clone(), pending_question.clone());
        self.suspension.store_tool_invocation(
            question_id.clone(),
            SuspendedToolInvocationState {
                suspension: suspension.clone(),
                inbound: inbound_msg.clone(),
                opencode_session_id: opencode_session_id.to_string(),
            },
        );
        self.emitter.publish(
            "tool_permission_requested",
            inbound_msg,
            json!({
                "question_id": question_id,
                "tool_use_id": suspension.suspended_tool.tool_use_id,
                "tool_name": suspension.suspended_tool.tool_name,
                "tool_input": suspension.suspended_tool.tool_input,
                "prompt": suspension.suspended_tool.permission_request.prompt,
                "risk_level": question_risk_level_str(&permission_risk_to_question_risk(
                    &suspension.suspended_tool.permission_request.risk_level,
                )),
                "source": "agentic_loop",
                "invocation_id": suspension.suspended_tool.invocation.invocation_id,
                "tool_invocation": self.emitter.tool_lifecycle_payload(
                    &suspension.suspended_tool.invocation,
                    json!({"source": "pending_question"}),
                ),
            }),
        );
        self.suspension.publish_question_asked(
            &|event: &str, msg: &InboundMessage, payload: Value| {
                self.emitter.publish(event, msg, payload);
            },
            inbound_msg,
            &memory_key(inbound_msg),
            &pending_question,
            None,
            now_ms().saturating_sub(started),
            now_ms().saturating_sub(started),
            false,
            "single",
        );
        make_task_result(MakeTaskResult {
            task_id: Uuid::new_v4().to_string(),
            trace_id: inbound_msg.id.clone(),
            status: "success".into(),
            result: Some(json!({
                "status": "waiting_user_input",
                "question_id": pending_question.question_id,
                "tool_name": pending_question
                    .raw_question
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            })),
            error: None,
            duration_ms: now_ms().saturating_sub(started),
            worker_id: None,
        })
    }

    pub(crate) fn resume_suspended_tool_permission(
        &self,
        job_id: &str,
        pending: PendingQuestion,
        answer: &str,
    ) -> Result<(), String> {
        let preparation = self.prepare_tool_permission_resume(job_id, &pending, answer)?;
        let suspended = preparation.suspended;
        let config = preparation.config;
        let outcome = continue_agentic_loop(
            &self._client,
            self.executor.as_ref(),
            &self.tool_registry,
            self.permission_checker.as_deref(),
            self.hook_registry.as_deref(),
            preparation.resumed_messages,
            suspended.suspension.iterations,
            suspended.suspension.compact_count,
            &config,
        )
        .map_err(|err| err.to_string())?;

        self.handle_tool_permission_continuation_outcome(job_id, &pending, suspended, outcome)
    }

    pub(crate) fn handle_tool_permission_continuation_outcome(
        &self,
        job_id: &str,
        pending: &PendingQuestion,
        suspended: SuspendedToolInvocationState,
        outcome: AgenticLoopOutcome,
    ) -> Result<(), String> {
        match outcome {
            AgenticLoopOutcome::Completed(loop_result) => {
                self.finish_tool_permission_continuation(job_id, pending, &suspended, &loop_result)
            }
            AgenticLoopOutcome::Suspended(next_suspension) => self
                .resuspend_tool_permission_continuation(
                    job_id,
                    pending,
                    suspended,
                    *next_suspension,
                ),
        }
    }

    pub(crate) fn publish_question_answer_submitted(
        &self,
        inbound: &InboundMessage,
        key: &str,
        pending: &PendingQuestion,
        answer: &QuestionAnswerInput,
    ) {
        self.emitter.publish(
            "question_answer_submitted",
            inbound,
            json!({
                "memory_key": key,
                "question_id": pending.question_id,
                "question_kind": question_kind_str(&pending.question_kind),
                "answer_type": answer.answer_type,
                "answer": answer.answer,
                "answer_summary": pending.answer_summary,
                "resolution_source": pending.resolution_source,
                "opencode_session_id": pending.opencode_session_id,
            }),
        );
    }

    pub(crate) fn continue_submitted_question_answer(
        &self,
        job_id: &str,
        inbound: &InboundMessage,
        key: &str,
        pending: &PendingQuestion,
        answer: &QuestionAnswerInput,
    ) -> Result<PendingQuestion, String> {
        match pending.source_kind {
            PendingQuestionSourceKind::UpstreamQuestion => {
                self.continue_upstream_question_answer(inbound, key, pending)
            }
            PendingQuestionSourceKind::ToolPermission => {
                self.continue_tool_permission_answer(job_id, pending, answer.answer.trim())
            }
        }
    }

    pub fn submit_question_answer(
        &self,
        job_id: &str,
        answer: QuestionAnswerInput,
    ) -> Result<PendingQuestion, String> {
        let pending = self.suspension.submit_answer(
            job_id,
            &answer,
            truncate_preview(&answer.answer, 120),
        )?;

        let inbound = pending.inbound.clone().ok_or_else(|| {
            format!("Missing inbound context for pending question on job: {job_id}")
        })?;
        let key = memory_key(&inbound);
        self.publish_question_answer_submitted(&inbound, &key, &pending, &answer);
        self.continue_submitted_question_answer(job_id, &inbound, &key, &pending, &answer)
    }
}
