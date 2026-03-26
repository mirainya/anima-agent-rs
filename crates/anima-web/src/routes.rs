use crate::AppState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

static INDEX_HTML: &str = include_str!("static/index.html");

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    pub session_id: Option<String>,
}

pub fn create_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(index_page))
        .route("/api/send", post(send_message))
        .route("/api/events", get(sse_events))
        .route("/api/status", get(system_status))
}

/// 首页
async fn index_page() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// 发送消息
async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendMessageRequest>,
) -> Json<serde_json::Value> {
    let msg = anima_runtime::bus::make_inbound(anima_runtime::bus::MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: req.session_id.clone(),
        content: req.content,
        session_key: req.session_id,
        ..Default::default()
    });

    match state.bus.publish_inbound(msg) {
        Ok(_) => Json(serde_json::json!({"ok": true})),
        Err(e) => Json(serde_json::json!({"ok": false, "error": e})),
    }
}

/// SSE 事件流
async fn sse_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.web_channel.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(event) => {
                let data = serde_json::to_string(&event).unwrap_or_default();
                Some(Ok(Event::default().data(data)))
            }
            Err(_) => None, // lagged, skip
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// 系统状态
async fn system_status(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let runtime = state.runtime.lock().unwrap();
    let agent_status = runtime.agent.status();
    let worker_status = agent_status.core.worker_pool;

    Json(serde_json::json!({
        "agent": {
            "running": agent_status.running,
        },
        "workers": worker_status.workers.iter().map(|w| {
            let mut obj = serde_json::json!({
                "id": w.id,
                "status": w.status,
                "metrics": {
                    "tasks_completed": w.metrics.tasks_completed,
                    "errors": w.metrics.errors,
                    "timeouts": w.metrics.timeouts,
                    "total_duration_ms": w.metrics.total_duration_ms,
                }
            });
            if let Some(ct) = &w.current_task {
                let now = anima_runtime::support::now_ms();
                let elapsed = now.saturating_sub(ct.started_ms);
                obj["current_task"] = serde_json::json!({
                    "task_id": ct.task_id,
                    "task_type": ct.task_type,
                    "elapsed_ms": elapsed,
                    "content_preview": ct.content_preview,
                });
            }
            obj
        }).collect::<Vec<_>>(),
        "metrics": agent_status.core.metrics.counters,
    }))
}
