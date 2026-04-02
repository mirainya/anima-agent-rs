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
                            "message_received" | "session_ready" | "session_create_failed" | "plan_built" | "cache_hit" | "cache_miss" | "worker_task_assigned" | "api_call_started" | "message_completed" | "message_failed" | "question_asked" | "question_answer_submitted" | "question_resolved" | "question_escalated_to_user" | "upstream_response_observed" | "requirement_evaluation_started" | "requirement_satisfied" | "requirement_unsatisfied" | "requirement_followup_scheduled" | "requirement_followup_exhausted" | "user_input_required" => {
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
