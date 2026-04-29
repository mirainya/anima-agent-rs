use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::bus::{
    make_internal, make_outbound, InboundMessage, InternalPayload, MakeInternal, MakeOutbound,
};
use crate::execution::agentic_loop::{
    continue_agentic_loop, resume_suspended_tool_invocation, run_agentic_loop, AgenticLoopConfig,
    AgenticLoopOutcome, AgenticLoopSuspension,
};
use crate::prompt::PromptAssembler;
use crate::streaming::types::{ContentBlock, ContentDelta, StreamEvent};
use crate::support::now_ms;

use crate::orchestrator::llm_decompose::extract_json_array;

use super::context_types::InitialAgenticLoopRunPreparation;
use super::core::CoreAgent;
use super::runtime_error::RuntimeError;
use super::types::{make_task_result, MakeTaskResult, TaskResult};

impl CoreAgent {
    pub(crate) fn prepare_initial_agentic_loop_run(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
    ) -> InitialAgenticLoopRunPreparation {
        use crate::messages::types::{InternalMsg, MessageRole};

        let initial_messages = vec![InternalMsg {
            role: MessageRole::User,
            blocks: vec![crate::messages::types::ContentBlock::Text {
                text: inbound_msg.content.clone(),
            }],
            message_id: inbound_msg.id.clone(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }];

        let system_prompt = {
            let mut asm = PromptAssembler::new();
            asm.add_text("identity", &self.prompts.read().agentic_loop_system, 0);
            asm.add_section(crate::prompt::sections::completion_status_section());
            Some(asm.build().text)
        };

        let bus = self.bus.clone();
        let out_channel = inbound_msg.channel.clone();
        let out_chat_id = inbound_msg.chat_id.clone();
        let out_sender = inbound_msg.sender_id.clone();
        let out_trace_id = inbound_msg.id.clone();
        let on_stream_event: Arc<dyn Fn(StreamEvent) + Send + Sync> =
            Arc::new(move |event| match &event {
                StreamEvent::ContentBlockDelta {
                    delta: ContentDelta::TextDelta { text },
                    ..
                } => {
                    let _ = bus.publish_outbound(make_outbound(MakeOutbound {
                        channel: out_channel.clone(),
                        chat_id: out_chat_id.clone(),
                        content: text.clone(),
                        reply_target: Some(out_sender.clone()),
                        stage: Some("streaming".into()),
                        ..Default::default()
                    }));
                }
                StreamEvent::ContentBlockStart {
                    content_block: ContentBlock::ToolUse { name, .. },
                    ..
                } => {
                    let _ = bus.publish_internal(make_internal(MakeInternal {
                        source: "agentic-loop".into(),
                        trace_id: Some(out_trace_id.clone()),
                        payload: InternalPayload::RuntimeEvent {
                            event: "tool_call_detected".to_string(),
                            message_id: out_trace_id.clone(),
                            channel: out_channel.clone(),
                            chat_id: out_chat_id.clone(),
                            sender_id: out_sender.clone(),
                            payload: json!({ "tool_name": name }),
                        },
                        ..Default::default()
                    }));
                }
                _ => {}
            });

        let inbound_for_tool_events = inbound_msg.clone();
        let emitter_for_tool_events: Arc<super::event_emitter::RuntimeEventEmitter> =
            Arc::clone(&self.emitter);
        let tool_event_publisher = Arc::new(move |event: &str, payload: Value| {
            emitter_for_tool_events.publish_tool_lifecycle(
                event,
                &inbound_for_tool_events,
                payload,
            );
        });

        let config = AgenticLoopConfig {
            max_iterations: 10,
            session_id: opencode_session_id.to_string(),
            trace_id: inbound_msg.id.clone(),
            system_prompt,
            tool_definitions: Some(self.tool_registry.tool_definitions()),
            streaming: true,
            on_stream_event: Some(on_stream_event),
            on_tool_lifecycle_event: Some(tool_event_publisher),
            ..Default::default()
        };

        InitialAgenticLoopRunPreparation {
            initial_messages,
            config,
        }
    }

    pub(crate) fn finalize_completed_agentic_loop_run(
        &self,
        inbound_msg: &InboundMessage,
        started: u64,
        loop_result: crate::execution::agentic_loop::AgenticLoopResult,
    ) -> TaskResult {
        let skip_initial_user = if loop_result.messages.first().map(|m| &m.role)
            == Some(&crate::messages::types::MessageRole::User)
        {
            &loop_result.messages[1..]
        } else {
            &loop_result.messages
        };
        self.append_transcript_messages(inbound_msg, skip_initial_user);
        let duration_ms = now_ms().saturating_sub(started);
        make_task_result(MakeTaskResult {
            task_id: Uuid::new_v4().to_string(),
            trace_id: inbound_msg.id.clone(),
            status: "success".into(),
            result: Some(json!({
                "text": loop_result.final_text,
                "iterations": loop_result.iterations,
                "hit_limit": loop_result.hit_limit,
            })),
            error: None,
            duration_ms,
            worker_id: None,
        })
    }

    pub(crate) fn build_failed_agentic_loop_task_result(
        &self,
        inbound_msg: &InboundMessage,
        started: u64,
        error: impl ToString,
    ) -> TaskResult {
        let duration_ms = now_ms().saturating_sub(started);
        make_task_result(MakeTaskResult {
            task_id: Uuid::new_v4().to_string(),
            trace_id: inbound_msg.id.clone(),
            status: "error".into(),
            result: None,
            error: Some(error.to_string()),
            duration_ms,
            worker_id: None,
        })
    }

    pub(crate) fn build_failed_agentic_loop_task_result_from_runtime_error(
        &self,
        inbound_msg: &InboundMessage,
        started: u64,
        error: &RuntimeError,
    ) -> TaskResult {
        self.build_failed_agentic_loop_task_result(inbound_msg, started, &error.internal_message)
    }

    pub(crate) fn run_agentic_loop_for_followup(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
        followup_prompt: &str,
    ) -> TaskResult {
        use crate::messages::types::{InternalMsg, MessageRole};

        let started = now_ms();
        let messages = vec![InternalMsg {
            role: MessageRole::User,
            blocks: vec![crate::messages::types::ContentBlock::Text {
                text: followup_prompt.to_string(),
            }],
            message_id: Uuid::new_v4().to_string(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }];

        let system_prompt = {
            let mut asm = PromptAssembler::new();
            asm.add_text("identity", &self.prompts.read().agentic_loop_system, 0);
            asm.add_section(crate::prompt::sections::completion_status_section());
            Some(asm.build().text)
        };

        let config = AgenticLoopConfig {
            max_iterations: 10,
            session_id: opencode_session_id.to_string(),
            trace_id: inbound_msg.id.clone(),
            system_prompt,
            tool_definitions: Some(self.tool_registry.tool_definitions()),
            streaming: true,
            on_stream_event: None,
            ..Default::default()
        };

        match run_agentic_loop(
            self.provider.as_ref(),
            &self.tool_registry,
            self.permission_checker.as_deref(),
            self.hook_registry.as_deref(),
            messages,
            &config,
        ) {
            Ok(AgenticLoopOutcome::Completed(loop_result)) => {
                self.finalize_completed_agentic_loop_run(inbound_msg, started, loop_result)
            }
            Ok(AgenticLoopOutcome::Suspended(_)) => {
                self.build_failed_agentic_loop_task_result(
                    inbound_msg,
                    started,
                    "followup suspended on tool permission",
                )
            }
            Err(err) => {
                let runtime_error = err.to_runtime_error();
                self.build_failed_agentic_loop_task_result_from_runtime_error(
                    inbound_msg,
                    started,
                    &runtime_error,
                )
            }
        }
    }

    pub(crate) fn run_agentic_loop_for_plan(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
    ) -> TaskResult {
        let started = now_ms();
        let preparation = self.prepare_initial_agentic_loop_run(inbound_msg, opencode_session_id);

        match run_agentic_loop(
            self.provider.as_ref(),
            &self.tool_registry,
            self.permission_checker.as_deref(),
            self.hook_registry.as_deref(),
            preparation.initial_messages,
            &preparation.config,
        ) {
            Ok(AgenticLoopOutcome::Completed(loop_result)) => {
                // Check if the agentic loop output looks like a multi-task decomposition plan.
                // This happens when try_llm_decompose failed but the AI still produced a plan.
                if let Some(orch_result) = self.try_dispatch_as_orchestration(
                    &loop_result.final_text,
                    inbound_msg,
                    opencode_session_id,
                ) {
                    debug!(
                        "[agentic_loop] detected multi-task plan in loop output, dispatched via orchestrator"
                    );
                    return orch_result;
                }
                self.finalize_completed_agentic_loop_run(inbound_msg, started, loop_result)
            }
            Ok(AgenticLoopOutcome::Suspended(suspension)) => {
                if self.is_auto_approval_mode() {
                    self.auto_resume_suspended_tool(
                        inbound_msg,
                        opencode_session_id,
                        *suspension,
                        &preparation.config,
                        started,
                    )
                } else {
                    self.handle_initial_tool_permission_suspension(
                        inbound_msg,
                        opencode_session_id,
                        *suspension,
                        started,
                    )
                }
            }
            Err(err) => {
                let runtime_error = err.to_runtime_error();
                self.build_failed_agentic_loop_task_result_from_runtime_error(
                    inbound_msg,
                    started,
                    &runtime_error,
                )
            }
        }
    }

    /// Try to parse the agentic loop output as a multi-task decomposition plan.
    /// If successful, execute it via the orchestrator and return the result.
    fn try_dispatch_as_orchestration(
        &self,
        final_text: &str,
        inbound_msg: &InboundMessage,
        session_id: &str,
    ) -> Option<TaskResult> {
        let text = final_text.trim();
        // Extract JSON array from text — it may be preceded by reasoning blocks etc.
        let json_str = extract_json_array(text)?;

        let specs: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
        // Must have more than 1 subtask to be a decomposition plan
        if specs.len() <= 1 {
            return None;
        }
        // Validate that items look like subtask specs (must have "name" and "description")
        let all_valid = specs.iter().all(|s| {
            s.get("name").and_then(|v| v.as_str()).filter(|v| !v.is_empty()).is_some()
                && s.get("description").and_then(|v| v.as_str()).filter(|v| !v.is_empty()).is_some()
        });
        if !all_valid {
            return None;
        }
        // The JSON array must dominate the text — reject if there's too much surrounding prose
        let non_json_len = text.len().saturating_sub(json_str.len());
        if non_json_len > 200 {
            return None;
        }
        if !all_valid {
            return None;
        }

        debug!(
            "[agentic_loop] final_text contains {} subtask specs, building plan directly",
            specs.len()
        );

        // Build the orchestration plan directly from the parsed specs,
        // instead of calling try_llm_decompose again (which may have already failed).
        let plan = self.orchestrator.build_plan_from_parsed_specs(
            json_str,
            &inbound_msg.content,
            &inbound_msg.id,
            &inbound_msg.id,
        );
        let plan = match plan {
            Some(p) => Arc::new(p),
            None => {
                warn!("[agentic_loop] failed to build plan from parsed specs");
                return None;
            }
        };

        if plan.subtasks.len() <= 1 {
            return None;
        }

        self.emitter.publish(
            "orchestration_selected",
            inbound_msg,
            json!({
                "plan_type": "single",
                "selected_plan_type": "orchestration-v1",
                "reason": "agentic_loop_fallback_decompose",
                "subtask_count": plan.subtasks.len(),
            }),
        );

        let result = self.orchestrator.execute_existing_plan(
            plan,
            &inbound_msg.content,
            &inbound_msg.id,
            session_id,
            |event, payload| self.emitter.publish(event, inbound_msg, payload),
        );

        match result {
            Ok(r) => Some(r),
            Err(e) => {
                warn!("[agentic_loop] orchestrator execution failed: {e}");
                None
            }
        }
    }

    fn is_auto_approval_mode(&self) -> bool {
        self.auto_approve_tools.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn auto_resume_suspended_tool(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
        suspension: AgenticLoopSuspension,
        config: &AgenticLoopConfig,
        started: u64,
    ) -> TaskResult {
        debug!(
            "[agentic_loop] auto-approving tool '{}' (approval_mode=auto)",
            suspension.suspended_tool.tool_name
        );
        let resumed_messages = match resume_suspended_tool_invocation(
            &suspension,
            true,
            &self.tool_registry,
            self.hook_registry.as_deref(),
            config,
        ) {
            Ok(msgs) => msgs,
            Err(err) => {
                return self.build_failed_agentic_loop_task_result_from_runtime_error(
                    inbound_msg,
                    started,
                    &err.to_runtime_error(),
                );
            }
        };

        match continue_agentic_loop(
            self.provider.as_ref(),
            &self.tool_registry,
            self.permission_checker.as_deref(),
            self.hook_registry.as_deref(),
            resumed_messages,
            suspension.iterations,
            suspension.compact_count,
            config,
        ) {
            Ok(AgenticLoopOutcome::Completed(loop_result)) => {
                self.finalize_completed_agentic_loop_run(inbound_msg, started, loop_result)
            }
            Ok(AgenticLoopOutcome::Suspended(next_suspension)) => self
                .auto_resume_suspended_tool(
                    inbound_msg,
                    opencode_session_id,
                    *next_suspension,
                    config,
                    started,
                ),
            Err(err) => self.build_failed_agentic_loop_task_result_from_runtime_error(
                inbound_msg,
                started,
                &err.to_runtime_error(),
            ),
        }
    }
}
