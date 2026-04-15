use crate::{routes_commands, routes_queries, routes_static, AppState};
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

pub fn create_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(routes_static::index_page))
        .route("/assets/{*path}", get(routes_static::static_asset))
        .route("/api/send", post(routes_commands::send_message))
        .route("/api/sessions", get(routes_queries::list_sessions))
        .route(
            "/api/sessions/{session_id}/history",
            get(routes_queries::session_history),
        )
        .route(
            "/api/sessions/{session_id}/send",
            post(routes_commands::send_message_for_session),
        )
        .route("/api/events", get(routes_queries::sse_events))
        .route("/api/status", get(routes_queries::system_status))
        .route("/api/jobs", get(routes_queries::list_jobs))
        .route(
            "/api/jobs/{job_id}/review",
            post(routes_commands::review_job),
        )
        .route(
            "/api/jobs/{job_id}/question-answer",
            post(routes_commands::answer_job_question),
        )
        .fallback(routes_static::spa_fallback)
}
