use crate::web_channel::{SseEvent, WebChannel};
use anima_runtime::bus::Bus;
use std::sync::Arc;
use std::time::Duration;

/// 监听 Bus internal 通道，将 Worker 状态事件转发给 WebChannel
pub fn start_internal_bus_forwarder(bus: Arc<Bus>, web_channel: Arc<WebChannel>) {
    std::thread::spawn(move || {
        let rx = bus.internal_receiver();
        loop {
            if bus.is_closed() && rx.is_empty() {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(msg) => {
                    // 解析 internal 消息中的 worker 状态事件
                    if let Some(event_type) = msg.payload.get("event").and_then(|v| v.as_str()) {
                        match event_type {
                            "worker_task_start" | "worker_task_end" => {
                                let worker_id = msg
                                    .payload
                                    .get("worker_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let status = msg
                                    .payload
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let task_type = msg
                                    .payload
                                    .get("task_type")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                web_channel.broadcast(SseEvent::WorkerStatus {
                                    worker_id,
                                    status,
                                    task_type,
                                });
                            }
                            "message_received"
                            | "session_ready"
                            | "session_create_failed"
                            | "plan_built"
                            | "cache_hit"
                            | "cache_miss"
                            | "worker_task_assigned"
                            | "api_call_started"
                            | "message_completed"
                            | "message_failed"
                            | "question_asked"
                            | "question_answer_submitted"
                            | "question_resolved"
                            | "question_escalated_to_user"
                            | "upstream_response_observed"
                            | "requirement_evaluation_started"
                            | "requirement_satisfied"
                            | "requirement_unsatisfied"
                            | "requirement_followup_scheduled"
                            | "requirement_followup_exhausted"
                            | "user_input_required"
                            | "tool_call_detected"
                            | "tool_permission_requested"
                            | "tool_permission_resolved"
                            | "orchestration_wait_started"
                            | "orchestration_wait_finished"
                            | "orchestration_plan_finalize_started"
                            | "orchestration_plan_finalize_finished"
                            | "orchestration_plan_cleanup_incomplete"
                            | "worker_api_call_started"
                            | "worker_api_call_finished"
                            | "worker_api_call_failed"
                            | "worker_api_call_streaming_started"
                            | "worker_api_call_streaming_finished"
                            | "worker_api_call_streaming_failed"
                            | "worker_task_cleanup_finished"
                            | "sdk_send_prompt_started"
                            | "sdk_send_prompt_finished"
                            | "sdk_send_prompt_failed"
                            | "sdk_stream_http_request_started"
                            | "sdk_stream_http_response_opened"
                            | "sdk_stream_sse_consume_started"
                            | "sdk_stream_first_event_observed"
                            | "sdk_stream_message_started"
                            | "sdk_stream_content_block_started"
                            | "sdk_stream_content_block_delta"
                            | "sdk_stream_message_delta"
                            | "sdk_stream_content_block_stopped"
                            | "sdk_stream_message_stopped" => {
                                web_channel.broadcast(SseEvent::RuntimeEvent {
                                    event: event_type.to_string(),
                                    message_id: msg
                                        .payload
                                        .get("message_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default()
                                        .to_string(),
                                    channel: msg
                                        .payload
                                        .get("channel")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default()
                                        .to_string(),
                                    chat_id: msg
                                        .payload
                                        .get("chat_id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                    sender_id: msg
                                        .payload
                                        .get("sender_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default()
                                        .to_string(),
                                    trace_id: msg.trace_id.clone(),
                                    payload: msg
                                        .payload
                                        .get("payload")
                                        .cloned()
                                        .unwrap_or_default(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if bus.is_closed() {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}
