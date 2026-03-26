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
