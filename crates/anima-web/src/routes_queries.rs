use crate::web_snapshot::{
    build_session_summaries_from_runtime, build_status_snapshot, json_content_preview,
    SessionHistoryItem,
};
use crate::AppState;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use std::collections::HashMap;
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
    let runtime = state.runtime.lock();
    let manager = &runtime.agent.session_manager;
    let runtime_snapshot = runtime.agent.core_agent().runtime_state_snapshot();
    let transcript_sessions = build_session_summaries_from_runtime(&runtime_snapshot);
    let mut sessions_by_id = transcript_sessions
        .into_iter()
        .map(|session| (session.session_id.clone(), session))
        .collect::<HashMap<_, _>>();

    for session in manager.get_all_sessions() {
        let history = manager.get_history(&session.id);
        let last_user_message_preview = history
            .iter()
            .rev()
            .find(|entry: &&serde_json::Value| {
                entry.get("role").and_then(|value| value.as_str()) == Some("user")
            })
            .and_then(|entry: &serde_json::Value| entry.get("content"))
            .map(json_content_preview)
            .unwrap_or_default();

        sessions_by_id
            .entry(session.id.clone())
            .and_modify(|item| {
                if item.last_user_message_preview.is_empty() && !last_user_message_preview.is_empty()
                {
                    item.last_user_message_preview = last_user_message_preview.clone();
                }
                item.history_len = item.history_len.max(history.len());
                item.last_active = item.last_active.max(session.last_active);
            })
            .or_insert_with(|| crate::web_snapshot::SessionListItem {
                chat_id: session.id.clone(),
                session_id: session.id,
                channel: session.channel,
                history_len: history.len(),
                last_user_message_preview,
                last_active: session.last_active,
            });
    }
    drop(runtime);

    let mut sessions = sessions_by_id.into_values().collect::<Vec<_>>();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active));

    Json(serde_json::json!({
        "ok": true,
        "sessions": sessions,
    }))
}

pub async fn session_history(
    Path(session_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let runtime = state.runtime.lock();
    let mut history = runtime.agent.session_manager.get_history(&session_id);
    let exists = runtime.agent.session_manager.session_exists(&session_id);
    if history.is_empty() {
        history = runtime.agent.core_agent().session_context_history(&session_id);
    }
    let history_known = !history.is_empty();
    drop(runtime);

    if !exists && !history_known {
        return Json(serde_json::json!({
            "ok": false,
            "error": "session_not_found",
            "session_id": session_id,
            "history": [],
        }));
    }

    let items = history
        .into_iter()
        .map(|entry: serde_json::Value| SessionHistoryItem {
            role: entry
                .get("role")
                .and_then(|value| value.as_str())
                .map(ToString::to_string),
            content: entry
                .get("content")
                .cloned()
                .unwrap_or_else(|| entry.clone()),
            recorded_at: entry
                .get("timestamp")
                .and_then(|value| value.as_u64())
                .or_else(|| entry.get("recorded_at_ms").and_then(|value| value.as_u64())),
            raw: entry,
        })
        .collect::<Vec<_>>();

    Json(serde_json::json!({
        "ok": true,
        "session_id": session_id,
        "history": items,
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
        "unified_runtime": snapshot.unified_runtime,
        "jobs": snapshot.jobs,
    }))
}
