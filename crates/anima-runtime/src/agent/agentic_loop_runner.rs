use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::bus::{make_internal, make_outbound, InboundMessage, MakeInternal, MakeOutbound};
use crate::execution::agentic_loop::{run_agentic_loop, AgenticLoopConfig, AgenticLoopOutcome};
use crate::prompt::PromptAssembler;
use crate::streaming::types::{ContentBlock, ContentDelta, StreamEvent};
use crate::support::now_ms;

use super::core::{CoreAgent, InitialAgenticLoopRunPreparation};
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
            content: json!(inbound_msg.content.clone()),
            message_id: inbound_msg.id.clone(),
            tool_use_id: None,
            filtered: false,
            metadata: json!({}),
        }];

        let system_prompt = {
            let mut asm = PromptAssembler::new();
            asm.add_text("identity", "你是 Anima 智能助手。", 0);
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
                        payload: json!({
                            "event": "tool_call_detected",
                            "tool_name": name,
                            "message_id": out_trace_id.clone(),
                            "channel": out_channel.clone(),
                            "chat_id": out_chat_id.clone(),
                            "sender_id": out_sender.clone(),
                        }),
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
        self.append_transcript_messages(inbound_msg, &loop_result.messages);
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

    pub(crate) fn run_agentic_loop_for_plan(
        &self,
        inbound_msg: &InboundMessage,
        opencode_session_id: &str,
    ) -> TaskResult {
        let started = now_ms();
        let preparation = self.prepare_initial_agentic_loop_run(inbound_msg, opencode_session_id);

        match run_agentic_loop(
            &self._client,
            self.executor.as_ref(),
            &self.tool_registry,
            self.permission_checker.as_deref(),
            self.hook_registry.as_deref(),
            preparation.initial_messages,
            &preparation.config,
        ) {
            Ok(AgenticLoopOutcome::Completed(loop_result)) => {
                self.finalize_completed_agentic_loop_run(inbound_msg, started, loop_result)
            }
            Ok(AgenticLoopOutcome::Suspended(suspension)) => self
                .handle_initial_tool_permission_suspension(
                    inbound_msg,
                    opencode_session_id,
                    *suspension,
                    started,
                ),
            Err(err) => self.build_failed_agentic_loop_task_result(inbound_msg, started, err),
        }
    }
}
