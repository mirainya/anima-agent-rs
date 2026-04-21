use serde_json::{json, Value};

use crate::bus::InboundMessage;
use crate::execution::context_assembly::ContextAssemblyMode;
use crate::execution::driver::{ApiCallExecutionRequest, ExecutionKind};
use crate::execution::requirement_judge::AgentFollowupPlan;
use crate::execution::turn_coordinator::{
    prepare_completed_branch_data, prepare_followup_branch_data, prepare_requirement_evaluation,
    prepare_requirement_evaluation_started_payload, prepare_requirement_satisfied_payload,
    prepare_upstream_response_observed_payload, prepare_waiting_user_input_branch_data,
    QuestionResolvedPayload, SummaryInput, TurnOutcomePlan,
};
use crate::runtime::planning_stalled_current_step;
use crate::tasks::{RequirementStatus, RunStatus, TaskStatus, TurnStatus};

use super::core::{
    turn_source_from_success, AgentFollowupPreparation, CoreAgent, ExecutionContext,
    RuntimeErrorInfo, RuntimeTaskPhase, SuccessSource, UpstreamRequirementContext,
    UpstreamTurnContext,
};
use super::runtime_error::classify_runtime_error;
use super::runtime_helpers::{
    extract_response_text, infer_operation, infer_provider, truncate_preview,
};
use super::suspension::{question_kind_str, PendingQuestion};

impl CoreAgent {
    pub(crate) fn publish_requirement_satisfied(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        response_text: &str,
    ) {
        let satisfied_payload = prepare_requirement_satisfied_payload(response_text);
        self.emitter.publish(
            "requirement_satisfied",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "cached": exec_ctx.cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "response_preview": satisfied_payload.response_preview,
            }),
        );
    }

    pub(crate) fn publish_resolved_question(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        pending: &PendingQuestion,
        resolved_payload: &QuestionResolvedPayload,
    ) {
        self.emitter.publish(
            "question_resolved",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "question_id": resolved_payload.question_id,
                "question_kind": question_kind_str(&pending.question_kind),
                "answer_summary": resolved_payload.answer_summary,
                "resolution_source": resolved_payload.resolution_source,
                "opencode_session_id": resolved_payload.opencode_session_id,
            }),
        );
    }

    pub(crate) fn record_execution_summary(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        summary: &SummaryInput,
    ) {
        self.emitter.record_summary(crate::agent::ExecutionSummary {
            trace_id: inbound_msg.id.clone(),
            message_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            plan_type: exec_ctx.plan_type.clone(),
            status: summary.status.clone(),
            cache_hit: exec_ctx.cache_hit,
            worker_id: summary.worker_id.clone(),
            error_code: summary.error_code.clone(),
            error_stage: summary.error_stage.clone(),
            task_duration_ms: summary.task_duration_ms,
            stages: summary.stages.clone(),
        });
    }

    pub(crate) fn publish_upstream_response_observed(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        result: &crate::agent::types::TaskResult,
    ) {
        let task_type = "api-call".to_string();
        let provider = infer_provider(result.result.as_ref());
        let operation = infer_operation(result.result.as_ref());
        let response_text = extract_response_text(result.result.as_ref());
        let observed_payload = prepare_upstream_response_observed_payload(
            result.worker_id.clone(),
            task_type,
            provider,
            operation,
            exec_ctx.opencode_session_id.clone(),
            truncate_preview(&response_text, 200),
            result.result.clone(),
        );
        self.emitter.publish(
            "upstream_response_observed",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "worker_id": observed_payload.worker_id,
                "task_type": observed_payload.task_type,
                "provider": observed_payload.provider,
                "operation": observed_payload.operation,
                "task_duration_ms": result.duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "opencode_session_id": observed_payload.opencode_session_id,
                "response_preview": observed_payload.response_preview,
                "raw_result": observed_payload.raw_result,
            }),
        );
    }

    pub(crate) fn prepare_upstream_requirement_context(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        result: &crate::agent::types::TaskResult,
        source: SuccessSource,
        resolved_question: Option<&PendingQuestion>,
    ) -> UpstreamRequirementContext {
        let turn_source = turn_source_from_success(source);
        let requirement_preparation = prepare_requirement_evaluation(
            turn_source,
            self.assemble_turn_context(
                inbound_msg,
                crate::execution::turn_coordinator::assembly_mode_for_source(turn_source),
                exec_ctx.opencode_session_id.clone(),
                resolved_question.cloned(),
                None,
            ),
            inbound_msg.id.clone(),
            inbound_msg.id.clone(),
            inbound_msg.chat_id.clone(),
            result.result.clone(),
        );
        let evaluation_started_payload =
            prepare_requirement_evaluation_started_payload(requirement_preparation.source_label);
        self.emitter.publish(
            "requirement_evaluation_started",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "worker_id": result.worker_id.clone(),
                "task_duration_ms": result.duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "source": evaluation_started_payload.source,
            }),
        );

        let judgement = crate::execution::requirement_judge::judge_requirement(
            &requirement_preparation.judge_context,
        );
        let turn_plan = crate::execution::turn_coordinator::plan_turn_outcome(
            judgement,
            resolved_question.is_some(),
        );
        let turn_source_label = match source {
            SuccessSource::Initial => "initial",
            SuccessSource::QuestionContinuation => "question_continuation",
            SuccessSource::AgentFollowup => "agent_followup",
        };

        UpstreamRequirementContext {
            turn_plan,
            turn_source_label,
        }
    }

    pub(crate) fn dispatch_upstream_turn_plan(
        &self,
        ctx: UpstreamTurnContext<'_>,
        turn_plan: TurnOutcomePlan,
    ) -> Result<Option<PendingQuestion>, String> {
        match turn_plan {
            TurnOutcomePlan::Complete {
                should_resolve_question,
            } => {
                let branch = prepare_completed_branch_data(
                    ctx.resolved_question,
                    should_resolve_question,
                    extract_response_text(ctx.result.result.as_ref()),
                    ctx.result.worker_id.clone(),
                    ctx.result.duration_ms,
                    ctx.exec_ctx.context_ms,
                    ctx.exec_ctx.session_ms,
                    ctx.exec_ctx.classify_ms,
                    ctx.exec_ctx.execute_ms,
                    ctx.exec_ctx.total_ms,
                );
                self.handle_upstream_result_complete(ctx, branch)
            }
            TurnOutcomePlan::AskUserInput {
                should_resolve_question,
                requirement,
            } => {
                let branch = prepare_waiting_user_input_branch_data(
                    ctx.resolved_question,
                    should_resolve_question,
                    *requirement,
                    ctx.inbound_msg,
                    ctx.result.worker_id.clone(),
                    ctx.result.duration_ms,
                    ctx.exec_ctx.context_ms,
                    ctx.exec_ctx.session_ms,
                    ctx.exec_ctx.classify_ms,
                    ctx.exec_ctx.execute_ms,
                    ctx.exec_ctx.total_ms,
                    ctx.result.result.as_ref(),
                );
                self.handle_upstream_result_waiting_user_input(ctx, branch)
            }
            TurnOutcomePlan::ScheduleFollowup { plan } => {
                self.handle_upstream_result_schedule_followup(ctx, *plan)
            }
        }
    }

    pub(crate) fn update_followup_requirement_progress(
        &self,
        inbound_msg: &InboundMessage,
        plan: &AgentFollowupPlan,
        preparation: &AgentFollowupPreparation,
    ) {
        self.requirement.update(
            &inbound_msg.id,
            preparation.next_round,
            Some(preparation.fingerprint.clone()),
            Some(plan.reason.clone()),
        );
    }

    pub(crate) fn publish_followup_exhausted_events(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        plan: &AgentFollowupPlan,
        preparation: &AgentFollowupPreparation,
    ) {
        let followup_exhausted_payload = preparation.followup_branch.exhausted.clone();
        self.emitter.publish(
            "requirement_followup_exhausted",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "cached": exec_ctx.cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "attempted_rounds": followup_exhausted_payload.attempted_rounds,
                "max_rounds": followup_exhausted_payload.max_rounds,
                "reason": followup_exhausted_payload.reason,
                "missing_requirements": followup_exhausted_payload.missing_requirements,
                "result_fingerprint": followup_exhausted_payload.result_fingerprint,
            }),
        );
        self.emitter.record_job_lifecycle_hint(
            &inbound_msg.id,
            "stalled",
            planning_stalled_current_step(),
        );
        self.emitter.publish(
            "message_failed",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "status": "followup_exhausted",
                "cached": exec_ctx.cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "error_code": "requirement_followup_exhausted",
                "error_stage": "requirement_judge",
                "error": plan.reason,
            }),
        );
    }

    pub(crate) fn build_followup_request(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        followup_prompt: &str,
    ) -> ApiCallExecutionRequest {
        let followup_assembly = self.assemble_turn_context(
            inbound_msg,
            ContextAssemblyMode::Followup,
            exec_ctx.opencode_session_id.clone(),
            None,
            Some(followup_prompt.to_string()),
        );
        ApiCallExecutionRequest {
            trace_id: inbound_msg.id.clone(),
            session_id: followup_assembly.metadata.opencode_session_id.clone(),
            content: followup_assembly.prompt_text.clone(),
            kind: ExecutionKind::Followup,
            metadata: Some(json!({})),
        }
    }

    pub(crate) fn handle_followup_request_failure(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        followup_result: &crate::agent::types::TaskResult,
        error_info: &RuntimeErrorInfo,
    ) {
        self.emitter.record_failure(inbound_msg, error_info);
        self.emitter.publish(
            "message_failed",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "status": followup_result.status,
                "cached": false,
                "worker_id": followup_result.worker_id,
                "task_duration_ms": followup_result.duration_ms,
                "message_latency_ms": exec_ctx.total_ms.saturating_add(followup_result.duration_ms),
                "error_code": error_info.code,
                "error_stage": error_info.stage,
                "error": error_info.internal_message,
            }),
        );
        self.upsert_runtime_task(
            inbound_msg,
            RuntimeTaskPhase::Followup,
            TaskStatus::Failed,
            "followup request failed",
            Some(error_info.internal_message.clone()),
        );
        self.upsert_runtime_turn(inbound_msg, "agent_followup", TurnStatus::Failed);
        self.send_error_response(inbound_msg, error_info);
    }

    pub(crate) fn prepare_agent_followup(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        plan: &AgentFollowupPlan,
    ) -> AgentFollowupPreparation {
        let progress = self.requirement.ensure(
            inbound_msg,
            super::core::DEFAULT_MAX_REQUIREMENT_FOLLOWUP_ROUNDS,
        );
        let next_round = progress.attempted_rounds + 1;
        let fingerprint = plan.result_fingerprint.clone();
        let followup_branch = prepare_followup_branch_data(
            plan.reason.clone(),
            plan.missing_requirements.clone(),
            plan.followup_prompt.clone(),
            fingerprint.clone(),
            next_round,
            progress.max_rounds,
            worker_id.clone(),
            task_duration_ms,
            exec_ctx.context_ms,
            exec_ctx.session_ms,
            exec_ctx.classify_ms,
            exec_ctx.execute_ms,
            exec_ctx.total_ms,
        );
        self.upsert_runtime_turn(inbound_msg, "agent_followup", TurnStatus::Running);
        self.upsert_runtime_task(
            inbound_msg,
            RuntimeTaskPhase::Followup,
            TaskStatus::Running,
            format!("followup scheduled: {}", plan.reason),
            None,
        );
        self.requirement.upsert_runtime_requirement(
            inbound_msg,
            &progress,
            RequirementStatus::FollowupScheduled,
        );
        self.emitter.publish(
            "requirement_unsatisfied",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "cached": exec_ctx.cache_hit,
                "worker_id": worker_id,
                "task_duration_ms": task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "attempted_rounds": progress.attempted_rounds,
                "max_rounds": progress.max_rounds,
                "result_fingerprint": fingerprint,
                "payload": followup_branch.unsatisfied.payload,
            }),
        );

        AgentFollowupPreparation {
            progress,
            next_round,
            fingerprint,
            followup_branch,
        }
    }

    pub(crate) fn handle_exhausted_followup(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        plan: &AgentFollowupPlan,
        preparation: &AgentFollowupPreparation,
    ) {
        self.update_followup_requirement_progress(inbound_msg, plan, preparation);
        self.record_execution_summary(
            inbound_msg,
            exec_ctx,
            &preparation.followup_branch.exhausted_summary,
        );
        self.publish_followup_exhausted_events(
            inbound_msg,
            exec_ctx,
            worker_id,
            task_duration_ms,
            plan,
            preparation,
        );
        self.upsert_runtime_task(
            inbound_msg,
            RuntimeTaskPhase::Followup,
            TaskStatus::Failed,
            "followup exhausted",
            Some(plan.reason.clone()),
        );
        self.requirement.upsert_runtime_requirement(
            inbound_msg,
            &preparation.progress,
            RequirementStatus::Exhausted,
        );
        self.upsert_runtime_turn(inbound_msg, "agent_followup", TurnStatus::Failed);
        self.send_error_response(
            inbound_msg,
            &RuntimeErrorInfo {
                code: "requirement_followup_exhausted",
                stage: "requirement_judge",
                user_message:
                    "主 agent 多轮自动补充后仍未能确认需求已满足，请查看任务事件诊断原因。".into(),
                internal_message: plan.reason.clone(),
            },
        );
    }

    pub(crate) fn execute_scheduled_followup(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        plan: &AgentFollowupPlan,
        preparation: &AgentFollowupPreparation,
    ) -> Result<(), String> {
        self.update_followup_requirement_progress(inbound_msg, plan, preparation);
        self.record_execution_summary(
            inbound_msg,
            exec_ctx,
            &preparation.followup_branch.pending_summary,
        );
        let followup_scheduled_payload = preparation.followup_branch.scheduled.clone();
        self.emitter.publish(
            "requirement_followup_scheduled",
            inbound_msg,
            json!({
                "memory_key": exec_ctx.memory_key,
                "plan_type": exec_ctx.plan_type,
                "cached": exec_ctx.cache_hit,
                "worker_id": preparation.followup_branch.pending_summary.worker_id.clone(),
                "task_duration_ms": preparation.followup_branch.pending_summary.task_duration_ms,
                "message_latency_ms": exec_ctx.total_ms,
                "attempted_rounds": followup_scheduled_payload.attempted_rounds,
                "max_rounds": followup_scheduled_payload.max_rounds,
                "reason": followup_scheduled_payload.reason,
                "missing_requirements": followup_scheduled_payload.missing_requirements,
                "followup_prompt": followup_scheduled_payload.followup_prompt,
            }),
        );
        self.upsert_runtime_task(
            inbound_msg,
            RuntimeTaskPhase::Followup,
            TaskStatus::Running,
            "executing followup request",
            None,
        );

        let followup_request =
            self.build_followup_request(inbound_msg, exec_ctx, &plan.followup_prompt);
        let followup_result = self.execute_api_call_request(
            inbound_msg,
            &exec_ctx.memory_key,
            &exec_ctx.plan_type,
            &followup_request,
            "主 agent 触发自动 followup，派发给 worker 继续推进",
            json!({}),
        )
        .map_err(|err| err.internal_message)?;

        if followup_result.status != "success" {
            let error_info =
                classify_runtime_error(followup_result.error.as_deref(), Some("plan_execute"));
            self.handle_followup_request_failure(
                inbound_msg,
                exec_ctx,
                &followup_result,
                &error_info,
            );
            return Ok(());
        }

        let followup_ctx = ExecutionContext {
            memory_key: exec_ctx.memory_key.clone(),
            history_session_id: exec_ctx.history_session_id.clone(),
            opencode_session_id: exec_ctx.opencode_session_id.clone(),
            plan_type: exec_ctx.plan_type.clone(),
            context_ms: exec_ctx.context_ms,
            session_ms: exec_ctx.session_ms,
            classify_ms: exec_ctx.classify_ms,
            execute_ms: exec_ctx
                .execute_ms
                .saturating_add(followup_result.duration_ms),
            total_ms: exec_ctx
                .total_ms
                .saturating_add(followup_result.duration_ms),
            cache_hit: false,
        };
        self.handle_upstream_result(
            inbound_msg,
            &followup_ctx,
            followup_result,
            SuccessSource::AgentFollowup,
            None,
        )?;
        Ok(())
    }

    pub(crate) fn schedule_agent_followup(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        worker_id: Option<String>,
        task_duration_ms: u64,
        plan: AgentFollowupPlan,
    ) -> Result<(), String> {
        let preparation = self.prepare_agent_followup(
            inbound_msg,
            exec_ctx,
            worker_id.clone(),
            task_duration_ms,
            &plan,
        );

        if preparation.next_round > preparation.progress.max_rounds
            || preparation
                .progress
                .last_result_fingerprint
                .as_ref()
                .map(|previous| previous == &preparation.fingerprint)
                .unwrap_or(false)
        {
            self.handle_exhausted_followup(
                inbound_msg,
                exec_ctx,
                worker_id,
                task_duration_ms,
                &plan,
                &preparation,
            );
            return Ok(());
        }

        self.execute_scheduled_followup(inbound_msg, exec_ctx, &plan, &preparation)
    }

    pub(crate) fn handle_upstream_result_complete(
        &self,
        ctx: UpstreamTurnContext<'_>,
        branch: crate::execution::turn_coordinator::CompletedBranchData,
    ) -> Result<Option<PendingQuestion>, String> {
        if let Some(resolved_payload) = branch.resolved_question.as_ref() {
            let pending = ctx.resolved_question.expect(
                "resolved question should exist when completed branch includes resolved payload",
            );
            self.publish_resolved_question(
                ctx.inbound_msg,
                ctx.exec_ctx,
                pending,
                resolved_payload,
            );
        }
        let satisfied_progress = self.requirement.state(&ctx.inbound_msg.id, false);
        self.suspension.clear_question(&ctx.inbound_msg.id);
        self.requirement.clear(&ctx.inbound_msg.id);
        let completed_turn_id = self.upsert_runtime_turn(
            ctx.inbound_msg,
            ctx.turn_source_label,
            TurnStatus::Completed,
        );
        self.upsert_runtime_run(
            ctx.inbound_msg,
            RunStatus::Completed,
            Some(completed_turn_id),
        );
        self.upsert_runtime_task(
            ctx.inbound_msg,
            RuntimeTaskPhase::Main,
            TaskStatus::Completed,
            "upstream result completed successfully",
            None,
        );
        if let Some(progress) = satisfied_progress {
            self.requirement.upsert_runtime_requirement(
                ctx.inbound_msg,
                &progress,
                RequirementStatus::Satisfied,
            );
        }
        self.append_history(
            &ctx.exec_ctx.memory_key,
            branch.completion.assistant_entry.clone(),
        );
        self.append_session_store_history(
            &ctx.exec_ctx.history_session_id,
            branch.completion.assistant_entry.clone(),
        );
        self.context_manager.add_to_session_history(
            &ctx.exec_ctx.history_session_id,
            branch.completion.assistant_entry.clone(),
        );
        self.metrics.counter_inc("messages_processed");
        self.record_execution_summary(ctx.inbound_msg, ctx.exec_ctx, &branch.summary);
        self.publish_requirement_satisfied(
            ctx.inbound_msg,
            ctx.exec_ctx,
            ctx.result.worker_id.clone(),
            ctx.result.duration_ms,
            &branch.completion.response_text,
        );
        self.emitter.record_job_lifecycle_hint(
            &ctx.inbound_msg.id,
            "completed",
            crate::runtime::completed_current_step(),
        );
        self.emitter.publish(
            "message_completed",
            ctx.inbound_msg,
            json!({
                "memory_key": ctx.exec_ctx.memory_key,
                "plan_type": ctx.exec_ctx.plan_type,
                "status": branch.message_completed.status,
                "cached": ctx.exec_ctx.cache_hit,
                "worker_id": ctx.result.worker_id.clone(),
                "task_duration_ms": ctx.result.duration_ms,
                "message_latency_ms": ctx.exec_ctx.total_ms,
                "response_preview": branch.message_completed.response_preview,
                "response_text": branch.message_completed.response_text,
            }),
        );
        self.send_response(ctx.inbound_msg, &branch.completion.response_text);
        Ok(ctx.resolved_question.cloned())
    }

    pub(crate) fn handle_upstream_result_waiting_user_input(
        &self,
        ctx: UpstreamTurnContext<'_>,
        branch: crate::execution::turn_coordinator::WaitingUserInputBranchData,
    ) -> Result<Option<PendingQuestion>, String> {
        if let Some(resolved_payload) = branch.resolved_question.as_ref() {
            let resolved = ctx.resolved_question.expect(
                "resolved question should exist when waiting branch includes resolved payload",
            );
            self.publish_resolved_question(
                ctx.inbound_msg,
                ctx.exec_ctx,
                resolved,
                resolved_payload,
            );
        }
        self.emitter.publish(
            "requirement_unsatisfied",
            ctx.inbound_msg,
            json!({
                "memory_key": ctx.exec_ctx.memory_key,
                "plan_type": ctx.exec_ctx.plan_type,
                "cached": ctx.exec_ctx.cache_hit,
                "worker_id": ctx.result.worker_id.clone(),
                "task_duration_ms": ctx.result.duration_ms,
                "message_latency_ms": ctx.exec_ctx.total_ms,
                "payload": branch.unsatisfied.payload,
            }),
        );
        self.emitter.publish(
            "user_input_required",
            ctx.inbound_msg,
            json!({
                "memory_key": ctx.exec_ctx.memory_key,
                "plan_type": ctx.exec_ctx.plan_type,
                "reason": branch.user_input_required.reason,
                "missing_requirements": branch.user_input_required.missing_requirements,
                "question_id": branch.user_input_required.question_id,
                "question_kind": question_kind_str(&branch.waiting.question.question_kind),
                "prompt": branch.user_input_required.prompt,
            }),
        );
        self.emitter.record_job_lifecycle_hint(
            &ctx.inbound_msg.id,
            "waiting_user_input",
            branch.user_input_required.prompt.clone(),
        );
        let question = branch.waiting.question;
        self.suspension
            .store_question(ctx.inbound_msg.id.clone(), question.clone());
        let waiting_turn_id =
            self.upsert_runtime_turn(ctx.inbound_msg, ctx.turn_source_label, TurnStatus::Waiting);
        self.upsert_runtime_run(ctx.inbound_msg, RunStatus::Running, Some(waiting_turn_id));
        self.upsert_runtime_task(
            ctx.inbound_msg,
            RuntimeTaskPhase::Main,
            TaskStatus::Suspended,
            "waiting for required user input",
            None,
        );
        if let Some(progress) = self.requirement.state(&ctx.inbound_msg.id, false) {
            self.requirement.upsert_runtime_requirement(
                ctx.inbound_msg,
                &progress,
                RequirementStatus::WaitingUserInput,
            );
        }
        self.record_execution_summary(ctx.inbound_msg, ctx.exec_ctx, &branch.summary);
        self.suspension.publish_question_asked(
            &|event: &str, msg: &InboundMessage, payload: Value| {
                self.emitter.publish(event, msg, payload);
            },
            ctx.inbound_msg,
            &ctx.exec_ctx.memory_key,
            &question,
            ctx.result.worker_id.clone(),
            ctx.exec_ctx.total_ms,
            ctx.result.duration_ms,
            ctx.exec_ctx.cache_hit,
            &ctx.exec_ctx.plan_type,
        );
        Ok(Some(question))
    }

    pub(crate) fn handle_upstream_result_schedule_followup(
        &self,
        ctx: UpstreamTurnContext<'_>,
        plan: AgentFollowupPlan,
    ) -> Result<Option<PendingQuestion>, String> {
        self.schedule_agent_followup(
            ctx.inbound_msg,
            ctx.exec_ctx,
            ctx.result.worker_id.clone(),
            ctx.result.duration_ms,
            plan,
        )?;
        Ok(ctx.resolved_question.cloned())
    }

    pub(crate) fn handle_upstream_result(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        result: crate::agent::types::TaskResult,
        source: SuccessSource,
        resolved_question: Option<&PendingQuestion>,
    ) -> Result<Option<PendingQuestion>, String> {
        if let Some(blocked_value) = result.blocked_reason.as_ref() {
            if let Ok(reason) =
                serde_json::from_value::<crate::tasks::SubtaskBlockedReason>(blocked_value.clone())
            {
                if let Some(resolved) =
                    self.try_auto_resolve_blocked(inbound_msg, exec_ctx, &reason)
                {
                    self.emitter
                        .append_subtask_blocked_transcript(inbound_msg, &reason, true);
                    return self.continue_with_auto_resolved(
                        inbound_msg, exec_ctx, &reason, &resolved,
                    );
                }
                self.emitter
                    .append_subtask_blocked_transcript(inbound_msg, &reason, false);
                if let Some(resolved) =
                    self.try_llm_resolve_blocked(inbound_msg, exec_ctx, &reason)
                {
                    self.emitter
                        .append_subtask_blocked_transcript(inbound_msg, &reason, true);
                    return self.continue_with_auto_resolved(
                        inbound_msg, exec_ctx, &reason, &resolved,
                    );
                }
                self.suspension.register_subtask_blocked(
                    &inbound_msg.id,
                    inbound_msg,
                    &exec_ctx.opencode_session_id,
                    &reason,
                );
                return Ok(self.suspension.question_state(&inbound_msg.id, false));
            }
        }

        self.publish_upstream_response_observed(inbound_msg, exec_ctx, &result);
        let requirement_ctx = self.prepare_upstream_requirement_context(
            inbound_msg,
            exec_ctx,
            &result,
            source,
            resolved_question,
        );
        self.upsert_runtime_turn(
            inbound_msg,
            requirement_ctx.turn_source_label,
            TurnStatus::Running,
        );
        self.upsert_runtime_task(
            inbound_msg,
            RuntimeTaskPhase::Main,
            TaskStatus::Running,
            format!(
                "handling upstream result via {}",
                requirement_ctx.turn_source_label
            ),
            None,
        );

        let ctx = UpstreamTurnContext {
            inbound_msg,
            exec_ctx,
            result: &result,
            resolved_question,
            turn_source_label: requirement_ctx.turn_source_label,
        };

        self.dispatch_upstream_turn_plan(ctx, requirement_ctx.turn_plan)
    }

    pub(crate) fn try_auto_resolve_blocked(
        &self,
        inbound_msg: &InboundMessage,
        _exec_ctx: &ExecutionContext,
        reason: &crate::tasks::SubtaskBlockedReason,
    ) -> Option<String> {
        let user_request = &inbound_msg.content;
        if user_request.trim().is_empty() {
            return None;
        }
        match reason {
            crate::tasks::SubtaskBlockedReason::MissingParameter { name, .. } => {
                let lower_request = user_request.to_lowercase();
                let lower_name = name.to_lowercase();
                if lower_request.contains(&lower_name) {
                    Some(format!(
                        "参数 `{name}` 可从用户原始请求中获取: {user_request}"
                    ))
                } else {
                    None
                }
            }
            crate::tasks::SubtaskBlockedReason::MissingContext { what_needed } => {
                let lower_request = user_request.to_lowercase();
                let lower_needed = what_needed.to_lowercase();
                if lower_request.contains(&lower_needed) {
                    Some(format!(
                        "所需上下文 `{what_needed}` 可从用户原始请求中获取: {user_request}"
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn continue_with_auto_resolved(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        reason: &crate::tasks::SubtaskBlockedReason,
        resolved: &str,
    ) -> Result<Option<PendingQuestion>, String> {
        let reason_desc = match reason {
            crate::tasks::SubtaskBlockedReason::MissingParameter { name, .. } => {
                format!("缺失参数 `{name}`")
            }
            crate::tasks::SubtaskBlockedReason::MissingContext { what_needed } => {
                format!("缺失上下文: {what_needed}")
            }
            _ => "子任务阻塞".to_string(),
        };
        let prompt = format!(
            "[子任务阻塞自动恢复] 原因: {reason_desc}\n补充信息: {resolved}\n请基于此信息继续执行。"
        );
        let key = super::core::memory_key(inbound_msg);
        let continuation_ctx = self.assemble_turn_context(
            inbound_msg,
            ContextAssemblyMode::SubtaskBlockedContinuation,
            exec_ctx.opencode_session_id.clone(),
            None,
            Some(prompt),
        );
        let request = ApiCallExecutionRequest {
            trace_id: inbound_msg.id.clone(),
            session_id: continuation_ctx.metadata.opencode_session_id.clone(),
            content: continuation_ctx.prompt_text.clone(),
            kind: ExecutionKind::QuestionContinuation,
            metadata: Some(json!({})),
        };
        let result = self
            .execute_api_call_request(
                inbound_msg,
                &key,
                "single",
                &request,
                "子任务阻塞自动恢复",
                json!({ "auto_resolved": true }),
            )
            .map_err(|err| err.internal_message)?;
        if result.status != "success" {
            return Err(result
                .error
                .unwrap_or_else(|| "Auto-resolved continuation failed".into()));
        }
        let new_exec_ctx = ExecutionContext {
            memory_key: continuation_ctx.metadata.memory_key,
            history_session_id: continuation_ctx.metadata.history_session_id,
            opencode_session_id: continuation_ctx.metadata.opencode_session_id,
            plan_type: "single".into(),
            context_ms: 0,
            session_ms: 0,
            classify_ms: 0,
            execute_ms: result.duration_ms,
            total_ms: result.duration_ms,
            cache_hit: false,
        };
        self.handle_upstream_result(
            inbound_msg,
            &new_exec_ctx,
            result,
            SuccessSource::QuestionContinuation,
            None,
        )
    }
}
