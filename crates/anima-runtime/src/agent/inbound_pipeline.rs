use serde_json::json;

use crate::bus::InboundMessage;
use crate::execution::context_assembly::ContextAssemblyMode;
use crate::support::now_ms;
use crate::tasks::{TaskStatus, TurnStatus};

use super::context_types::{
    ExecutionContext, ProcessInboundResolutionContext, RuntimeTaskPhase, SuccessSource,
};
use super::core::{CoreAgent, ExecutionStageDurations};
use super::runtime_error::classify_runtime_error;
use super::types::ExecutionPlan;

impl CoreAgent {
    pub(crate) fn process_inbound_message_flow(&self, inbound_msg: InboundMessage) {
        let started = now_ms();
        self.metrics.counter_inc("messages_received");
        self.upsert_runtime_turn(&inbound_msg, "initial", TurnStatus::Running);
        self.upsert_runtime_task(
            &inbound_msg,
            RuntimeTaskPhase::Main,
            TaskStatus::Running,
            "processing inbound message",
            None,
        );
        let context = self.prepare_inbound_context(&inbound_msg);
        let key = context.key;
        let context_ms = context.context_ms;

        let session = match self.prepare_inbound_session(&inbound_msg, &key) {
            Ok(session) => session,
            Err(error_info) => {
                self.emitter.record_failure(&inbound_msg, &error_info);
                self.metrics.counter_inc("messages_failed");
                self.emitter.publish(
                    "session_create_failed",
                    &inbound_msg,
                    json!({
                        "memory_key": key,
                        "error_code": error_info.code,
                        "error_stage": error_info.stage,
                        "error_message": error_info.internal_message,
                    }),
                );
                self.send_error_response(&inbound_msg, &error_info);
                return;
            }
        };
        let opencode_session_id = session.opencode_session_id;
        let session_ms = session.session_ms;

        let plan_preparation = self.prepare_inbound_plan(&inbound_msg, &key);
        let plan = plan_preparation.plan;
        let classify_ms = plan_preparation.classify_ms;
        if plan.plan_type == "direct" {
            self.handle_direct_plan(
                &inbound_msg,
                started,
                &key,
                &plan,
                context_ms,
                session_ms,
                classify_ms,
            );
            return;
        }

        let execution =
            self.execute_initial_message_plan(&inbound_msg, &plan, &opencode_session_id, &key);
        let plan_type = execution.plan_type;
        let cache_hit = execution.cache_hit;
        let result = execution.result;

        let total_ms = now_ms().saturating_sub(started);
        let execute_ms = execution.execute_ms;
        self.metrics.histogram_record("message_latency", total_ms);
        let worker_statuses = self.worker_pool.status().workers;
        let active = worker_statuses
            .iter()
            .filter(|worker| worker.status == "busy")
            .count();
        let idle = worker_statuses
            .iter()
            .filter(|worker| worker.status == "idle")
            .count();
        self.metrics.update_worker_gauges(active, idle);

        let resolution_ctx = ProcessInboundResolutionContext {
            inbound_msg: &inbound_msg,
            key: &key,
            opencode_session_id: &opencode_session_id,
            plan_type: &plan_type,
            context_ms,
            session_ms,
            classify_ms,
            execute_ms,
            total_ms,
            cache_hit,
        };

        if result.status == "success" {
            self.finish_successful_inbound_processing(resolution_ctx, result);
        } else {
            self.finish_failed_inbound_processing(resolution_ctx, result);
        }
    }

    pub(crate) fn finish_successful_inbound_processing(
        &self,
        ctx: ProcessInboundResolutionContext<'_>,
        result: crate::agent::types::TaskResult,
    ) {
        let initial_context = self.assemble_turn_context(
            ctx.inbound_msg,
            ContextAssemblyMode::Initial,
            ctx.opencode_session_id.to_string(),
            None,
            Some(ctx.inbound_msg.content.clone()),
        );
        let exec_ctx = ExecutionContext {
            memory_key: initial_context.metadata.memory_key,
            history_session_id: initial_context.metadata.history_session_id,
            opencode_session_id: initial_context.metadata.opencode_session_id,
            plan_type: ctx.plan_type.to_string(),
            context_ms: ctx.context_ms,
            session_ms: ctx.session_ms,
            classify_ms: ctx.classify_ms,
            execute_ms: ctx.execute_ms,
            total_ms: ctx.total_ms,
            cache_hit: ctx.cache_hit,
        };
        let _ = self.handle_upstream_result(
            ctx.inbound_msg,
            &exec_ctx,
            result,
            SuccessSource::Initial,
            None,
        );
    }

    pub(crate) fn finish_failed_inbound_processing(
        &self,
        ctx: ProcessInboundResolutionContext<'_>,
        result: crate::agent::types::TaskResult,
    ) {
        let error_info = classify_runtime_error(result.error.as_deref(), Some("plan_execute"));
        self.emitter.record_failure(ctx.inbound_msg, &error_info);
        self.metrics.counter_inc("messages_failed");
        self.emitter.record_summary(crate::agent::ExecutionSummary {
            trace_id: ctx.inbound_msg.id.clone(),
            message_id: ctx.inbound_msg.id.clone(),
            channel: ctx.inbound_msg.channel.clone(),
            chat_id: ctx.inbound_msg.chat_id.clone(),
            plan_type: ctx.plan_type.to_string(),
            status: result.status.clone(),
            cache_hit: ctx.cache_hit,
            worker_id: result.worker_id.clone(),
            error_code: Some(error_info.code.to_string()),
            error_stage: Some(error_info.stage.to_string()),
            task_duration_ms: result.duration_ms,
            stages: ExecutionStageDurations {
                context_ms: ctx.context_ms,
                session_ms: ctx.session_ms,
                classify_ms: ctx.classify_ms,
                execute_ms: ctx.execute_ms,
                total_ms: ctx.total_ms,
            },
        });
        self.emitter.publish(
            "message_failed",
            ctx.inbound_msg,
            json!({
                "memory_key": ctx.key,
                "plan_type": ctx.plan_type,
                "status": result.status.clone(),
                "cached": ctx.cache_hit,
                "worker_id": result.worker_id.clone(),
                "task_duration_ms": result.duration_ms,
                "message_latency_ms": ctx.total_ms,
                "error_code": error_info.code,
                "error_stage": error_info.stage,
                "error": error_info.internal_message,
            }),
        );
        self.send_error_response(ctx.inbound_msg, &error_info);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_direct_plan(
        &self,
        inbound_msg: &InboundMessage,
        started: u64,
        key: &str,
        plan: &ExecutionPlan,
        context_ms: u64,
        session_ms: u64,
        classify_ms: u64,
    ) {
        let duration_ms = now_ms().saturating_sub(started);
        self.metrics.counter_inc("messages_processed");
        self.metrics
            .histogram_record("message_latency", duration_ms);
        self.emitter.record_summary(crate::agent::ExecutionSummary {
            trace_id: inbound_msg.id.clone(),
            message_id: inbound_msg.id.clone(),
            channel: inbound_msg.channel.clone(),
            chat_id: inbound_msg.chat_id.clone(),
            plan_type: plan.plan_type.clone(),
            status: "success".into(),
            cache_hit: false,
            worker_id: None,
            error_code: None,
            error_stage: None,
            task_duration_ms: 0,
            stages: ExecutionStageDurations {
                context_ms,
                session_ms,
                classify_ms,
                execute_ms: 0,
                total_ms: duration_ms,
            },
        });
        self.emitter.publish(
            "message_completed",
            inbound_msg,
            json!({
                "memory_key": key,
                "plan_type": plan.plan_type,
                "status": "success",
                "cached": false,
                "duration_ms": duration_ms,
                "response_preview": "Command processed.",
            }),
        );
        self.send_response(inbound_msg, "Command processed.");
    }
}
