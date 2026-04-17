use crate::web_snapshot::{
    build_session_history_snapshot, build_sessions_snapshot, build_status_snapshot,
};
use crate::AppState;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub async fn sse_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.web_channel.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let data = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn list_jobs(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let snapshot = build_status_snapshot(state.as_ref());
    Json(serde_json::json!({
        "ok": true,
        "jobs": snapshot.jobs,
    }))
}

pub async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sessions = build_sessions_snapshot(state.as_ref());

    Json(serde_json::json!({
        "ok": true,
        "sessions": sessions,
    }))
}

pub async fn session_history(
    Path(session_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let Some(history) = build_session_history_snapshot(state.as_ref(), &session_id) else {
        return Json(serde_json::json!({
            "ok": false,
            "error": "session_not_found",
            "session_id": session_id,
            "history": [],
        }));
    };

    Json(serde_json::json!({
        "ok": true,
        "session_id": session_id,
        "history": history,
    }))
}

pub async fn system_status(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let snapshot = build_status_snapshot(state.as_ref());
    Json(serde_json::json!({
        "agent": snapshot.agent,
        "workers": snapshot.workers,
        "worker_pool": snapshot.worker_pool,
        "recent_sessions": snapshot.recent_sessions,
        "failures": snapshot.failures,
        "runtime_timeline": snapshot.runtime_timeline,
        "recent_execution_summaries": snapshot.recent_execution_summaries,
        "metrics": snapshot.metrics,
        "warnings": snapshot.warnings,
        "unified_runtime": snapshot.unified_runtime,
        "jobs": snapshot.jobs,
    }))
}
