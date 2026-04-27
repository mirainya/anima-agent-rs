use crate::web_channel::{SseEvent, WebChannel};
use anima_runtime::bus::Bus;
use anima_types::message::InternalPayload;
use serde_json::Value;
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
                Ok(msg) => match &msg.payload {
                    InternalPayload::WorkerStatus {
                        event,
                        worker_id,
                        status,
                        task_type,
                        ..
                    } if event == "worker_task_start" || event == "worker_task_end" => {
                        web_channel.broadcast(SseEvent::WorkerStatus {
                            worker_id: worker_id.clone(),
                            status: status.clone(),
                            task_type: Some(task_type.clone()),
                        });
                    }
                    InternalPayload::RuntimeEvent {
                        event,
                        message_id,
                        channel,
                        chat_id,
                        sender_id,
                        payload,
                    } if event == "plan_proposed" => {
                        let proposal_id = payload.get("proposal_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let summary = payload.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let task_count = payload.get("task_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        web_channel.broadcast(SseEvent::PlanProposed {
                            job_id: message_id.clone(),
                            proposal_id,
                            summary,
                            task_count,
                        });
                    }
                    InternalPayload::RuntimeEvent {
                        event,
                        message_id,
                        payload,
                        ..
                    } if event == "sdk_stream_content_block_delta" => {
                        if let Some(sse) = map_stream_delta(message_id, payload) {
                            web_channel.broadcast(sse);
                        }
                    }
                    InternalPayload::RuntimeEvent {
                        event,
                        message_id,
                        payload,
                        ..
                    } if event == "sdk_stream_content_block_started"
                        || event == "sdk_stream_content_block_stopped" =>
                    {
                        let phase = if event.ends_with("started") { "started" } else { "stopped" };
                        let index = payload.get("content_block_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let kind = payload.get("content_block_kind").and_then(|v| v.as_str())
                            .or_else(|| payload.get("delta_kind").and_then(|v| v.as_str()))
                            .unwrap_or("unknown").to_string();
                        web_channel.broadcast(SseEvent::StreamBlockLifecycle {
                            job_id: message_id.clone(),
                            index,
                            phase: phase.to_string(),
                            kind,
                        });
                    }
                    InternalPayload::RuntimeEvent {
                        event,
                        message_id,
                        channel,
                        chat_id,
                        sender_id,
                        payload,
                    } => {
                        web_channel.broadcast(SseEvent::RuntimeEvent {
                            event: event.clone(),
                            message_id: message_id.clone(),
                            channel: channel.clone(),
                            chat_id: chat_id.clone(),
                            sender_id: sender_id.clone(),
                            trace_id: msg.trace_id.clone(),
                            payload: payload.clone(),
                        });
                    }
                    _ => {}
                },
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

fn map_stream_delta(message_id: &str, payload: &Value) -> Option<SseEvent> {
    let index = payload.get("content_block_index").and_then(|v| v.as_u64())? as usize;
    let delta_kind = payload.get("delta_kind").and_then(|v| v.as_str())?;
    let (kind, delta) = match delta_kind {
        "text_delta" => ("text", payload.get("text_delta").and_then(|v| v.as_str())?),
        "thinking_delta" => ("thinking", payload.get("thinking_delta").and_then(|v| v.as_str())?),
        "input_json_delta" => ("tool_input", payload.get("partial_json").and_then(|v| v.as_str())?),
        _ => return None,
    };
    Some(SseEvent::StreamDelta {
        job_id: message_id.to_string(),
        index,
        kind: kind.to_string(),
        delta: delta.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn map_text_delta() {
        let payload = json!({
            "content_block_index": 1,
            "delta_kind": "text_delta",
            "text_delta": "hello"
        });
        let sse = map_stream_delta("msg1", &payload).unwrap();
        assert!(matches!(sse, SseEvent::StreamDelta { kind, delta, index, .. } if kind == "text" && delta == "hello" && index == 1));
    }

    #[test]
    fn map_thinking_delta() {
        let payload = json!({
            "content_block_index": 0,
            "delta_kind": "thinking_delta",
            "thinking_delta": "step 1"
        });
        let sse = map_stream_delta("msg2", &payload).unwrap();
        assert!(matches!(sse, SseEvent::StreamDelta { kind, delta, .. } if kind == "thinking" && delta == "step 1"));
    }

    #[test]
    fn map_tool_input_delta() {
        let payload = json!({
            "content_block_index": 2,
            "delta_kind": "input_json_delta",
            "partial_json": "{\"key\":"
        });
        let sse = map_stream_delta("msg3", &payload).unwrap();
        assert!(matches!(sse, SseEvent::StreamDelta { kind, .. } if kind == "tool_input"));
    }

    #[test]
    fn map_unknown_delta_returns_none() {
        let payload = json!({
            "content_block_index": 0,
            "delta_kind": "unknown_delta"
        });
        assert!(map_stream_delta("msg4", &payload).is_none());
    }
}
