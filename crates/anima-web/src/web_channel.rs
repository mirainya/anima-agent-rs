use anima_runtime::channel::adapter::{ok, Channel, SendOptions, SendResult};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;

/// SSE 事件类型
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SseEvent {
    /// AI 回复消息
    #[serde(rename = "message")]
    Message {
        content: String,
        stage: String,
        target: String,
    },
    /// Worker 状态变更
    #[serde(rename = "worker_status")]
    WorkerStatus {
        worker_id: String,
        status: String,
        task_type: Option<String>,
    },
    /// 主链路运行时事件
    #[serde(rename = "runtime_event")]
    RuntimeEvent {
        event: String,
        message_id: String,
        channel: String,
        chat_id: Option<String>,
        sender_id: String,
        trace_id: String,
        payload: serde_json::Value,
    },
    /// 系统指标更新
    #[serde(rename = "metrics")]
    Metrics { data: serde_json::Value },
    /// 执行计划待审批
    #[serde(rename = "plan_proposed")]
    PlanProposed {
        job_id: String,
        proposal_id: String,
        summary: String,
        task_count: usize,
    },
    /// 流式 token 增量
    #[serde(rename = "stream_delta")]
    StreamDelta {
        job_id: String,
        index: usize,
        kind: String,
        delta: String,
    },
    /// 流式内容块生命周期
    #[serde(rename = "stream_block_lifecycle")]
    StreamBlockLifecycle {
        job_id: String,
        index: usize,
        phase: String,
        kind: String,
    },
}

/// Web 通道：通过 SSE 将消息推送给浏览器
pub struct WebChannel {
    running: AtomicBool,
    tx: broadcast::Sender<SseEvent>,
}

impl std::fmt::Debug for WebChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebChannel")
            .field("running", &self.running.load(Ordering::SeqCst))
            .finish()
    }
}

impl Default for WebChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl WebChannel {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            running: AtomicBool::new(true),
            tx,
        }
    }

    /// 获取一个新的 SSE 事件接收器
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
    }

    /// 广播任意 SSE 事件
    pub fn broadcast(&self, event: SseEvent) {
        let _ = self.tx.send(event);
    }
}

impl Channel for WebChannel {
    fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn send_message(&self, target: &str, message: &str, opts: SendOptions) -> SendResult {
        let stage = opts.stage.clone().unwrap_or_else(|| "final".into());
        self.broadcast(SseEvent::Message {
            content: message.to_string(),
            stage,
            target: target.to_string(),
        });
        ok(None)
    }

    fn channel_name(&self) -> &str {
        "web"
    }

    fn health_check(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_delta_serializes_correctly() {
        let event = SseEvent::StreamDelta {
            job_id: "j1".into(),
            index: 0,
            kind: "text".into(),
            delta: "hello".into(),
        };
        let json: serde_json::Value = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(json["type"], "stream_delta");
        assert_eq!(json["kind"], "text");
        assert_eq!(json["delta"], "hello");
        assert_eq!(json["index"], 0);
    }

    #[test]
    fn stream_block_lifecycle_serializes_correctly() {
        let event = SseEvent::StreamBlockLifecycle {
            job_id: "j1".into(),
            index: 1,
            phase: "started".into(),
            kind: "thinking".into(),
        };
        let json: serde_json::Value = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(json["type"], "stream_block_lifecycle");
        assert_eq!(json["phase"], "started");
        assert_eq!(json["kind"], "thinking");
    }
}
