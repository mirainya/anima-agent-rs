use crate::jobs::{build_job_views, AcceptedJob, JobKind, JobReviewInput};
use anima_runtime::agent::QuestionAnswerInput;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::convert::Infallible;
use std::path::{Component, Path as StdPath, PathBuf};
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

static FRONTEND_BUILD_REQUIRED_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Anima Agent</title>
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      font-family: 'Segoe UI', system-ui, sans-serif;
      background: #0f0f13;
      color: #e0e0e8;
    }
    .panel {
      max-width: 560px;
      padding: 24px 28px;
      border-radius: 14px;
      border: 1px solid #2a2a3a;
      background: #1a1a24;
      box-shadow: 0 10px 30px rgba(0,0,0,0.25);
    }
    h1 { margin: 0 0 12px; font-size: 20px; }
    p { margin: 8px 0; line-height: 1.6; color: #b8b8c8; }
    code {
      display: inline-block;
      margin-top: 8px;
      padding: 2px 8px;
      border-radius: 999px;
      background: rgba(162, 155, 254, 0.15);
      color: #c8c4ff;
    }
  </style>
</head>
<body>
  <main class="panel">
    <h1>前端构建产物不存在</h1>
    <p>当前 anima-web 已切换为 React + Vite 工作台。请先构建前端资源后再刷新页面。</p>
    <p><code>cd crates/anima-web/frontend && npm install && npm run build</code></p>
  </main>
</body>
</html>
"#;

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    pub session_id: Option<String>,
}

#[derive(Deserialize)]
pub struct QuestionAnswerRequest {
    pub question_id: String,
    pub source: String,
    pub answer_type: String,
    pub answer: String,
}

pub fn create_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(index_page))
        .route("/assets/{*path}", get(static_asset))
        .route("/api/send", post(send_message))
        .route("/api/events", get(sse_events))
        .route("/api/status", get(system_status))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/{job_id}/review", post(review_job))
        .route("/api/jobs/{job_id}/question-answer", post(answer_job_question))
        .fallback(spa_fallback)
}

fn frontend_dist_dir() -> PathBuf {
    StdPath::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("static")
        .join("dist")
}

fn frontend_dist_index_path() -> PathBuf {
    frontend_dist_dir().join("index.html")
}

async fn render_frontend_index() -> Response {
    match tokio::fs::read_to_string(frontend_dist_index_path()).await {
        Ok(content) => Html(content).into_response(),
        Err(_) => Html(FRONTEND_BUILD_REQUIRED_HTML).into_response(),
    }
}

/// 首页
async fn index_page() -> Response {
    render_frontend_index().await
}

async fn spa_fallback(uri: Uri) -> Response {
    if uri.path().starts_with("/api/") || uri.path().starts_with("/assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }

    render_frontend_index().await
}

async fn static_asset(Path(path): Path<String>) -> Response {
    let Some(safe_path) = sanitize_relative_path(&path) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let asset_path = frontend_dist_dir().join("assets").join(safe_path);
    let Ok(bytes) = tokio::fs::read(&asset_path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut response = Response::new(bytes.into_response().into_body());
    let content_type = content_type_for_path(&asset_path);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type),
    );
    response
}

fn sanitize_relative_path(path: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        return None;
    }

    let mut sanitized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }

    Some(sanitized)
}

fn content_type_for_path(path: &StdPath) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// 发送消息
async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendMessageRequest>,
) -> Json<serde_json::Value> {
    let user_content = req.content.clone();
    let msg = anima_runtime::bus::make_inbound(anima_runtime::bus::MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: req.session_id.clone(),
        content: req.content,
        session_key: req.session_id.clone(),
        ..Default::default()
    });
    let job_id = msg.id.clone();
    let session_id = req.session_id.clone().unwrap_or_else(|| job_id.clone());
    {
        let mut jobs = state.jobs.lock().unwrap();
        jobs.register_accepted_job(AcceptedJob {
            job_id: job_id.clone(),
            trace_id: job_id.clone(),
            message_id: job_id.clone(),
            kind: JobKind::Main,
            parent_job_id: None,
            channel: "web".into(),
            chat_id: Some(session_id.clone()),
            sender_id: "web-user".into(),
            user_content,
            accepted_at_ms: anima_runtime::support::now_ms(),
        });
    }

    match state.bus.publish_inbound(msg) {
        Ok(_) => Json(serde_json::json!({
            "ok": true,
            "accepted": true,
            "job_id": job_id,
            "chat_id": session_id,
            "session_id": req.session_id,
        })),
        Err(e) => Json(serde_json::json!({"ok": false, "accepted": false, "error": e})),
    }
}

/// SSE 事件流
async fn sse_events(
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

async fn list_jobs(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let snapshot = build_status_snapshot(state.as_ref());
    Json(serde_json::json!({
        "ok": true,
        "jobs": snapshot.jobs,
    }))
}

async fn review_job(
    Path(job_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(input): Json<JobReviewInput>,
) -> Json<serde_json::Value> {
    let review = state.jobs.lock().unwrap().record_review(job_id.clone(), input);
    let snapshot = build_status_snapshot(state.as_ref());
    let job = snapshot.jobs.into_iter().find(|job| job.job_id == job_id);
    Json(serde_json::json!({
        "ok": true,
        "job_id": job_id,
        "review": review,
        "job": job,
    }))
}

async fn answer_job_question(
    Path(job_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(input): Json<QuestionAnswerRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let runtime = state.runtime.lock().unwrap();
        runtime.agent.submit_question_answer(
            &job_id,
            QuestionAnswerInput {
                question_id: input.question_id.clone(),
                source: input.source.clone(),
                answer_type: input.answer_type,
                answer: input.answer.clone(),
            },
        )
    };

    match result {
        Ok(question) => {
            let snapshot = build_status_snapshot(state.as_ref());
            let job = snapshot.jobs.into_iter().find(|job| job.job_id == job_id);
            Json(serde_json::json!({
                "ok": true,
                "job_id": job_id,
                "question_id": input.question_id,
                "question": question,
                "job": job,
            }))
        }
        Err(error) => Json(serde_json::json!({
            "ok": false,
            "job_id": job_id,
            "question_id": input.question_id,
            "error": error,
        })),
    }
}

/// 系统状态
async fn system_status(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
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
        "jobs": snapshot.jobs,
    }))
}

struct StatusSnapshot {
    agent: serde_json::Value,
    workers: Vec<serde_json::Value>,
    worker_pool: serde_json::Value,
    recent_sessions: Vec<serde_json::Value>,
    failures: serde_json::Value,
    runtime_timeline: Vec<serde_json::Value>,
    recent_execution_summaries: Vec<serde_json::Value>,
    metrics: serde_json::Value,
    jobs: Vec<crate::jobs::JobView>,
}

fn build_status_snapshot(state: &AppState) -> StatusSnapshot {
    let runtime = state.runtime.lock().unwrap();
    let agent_status = runtime.agent.status();
    let worker_status = agent_status.core.worker_pool.clone();
    let failure_list = agent_status
        .core
        .failures
        .last_failure
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let jobs = {
        let store = state.jobs.lock().unwrap();
        build_job_views(
            &agent_status.core.runtime_timeline,
            &agent_status.core.recent_execution_summaries,
            &failure_list,
            &worker_status.workers,
            &store,
        )
    };

    StatusSnapshot {
        agent: serde_json::json!({
            "running": agent_status.running,
            "status": agent_status.core.status,
            "context_status": agent_status.core.context_status,
            "sessions_count": agent_status.core.sessions_count,
            "cache_entries": agent_status.core.cache_entries,
        }),
        workers: worker_status.workers.iter().map(|w| {
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
                    "trace_id": ct.trace_id,
                    "task_type": ct.task_type,
                    "elapsed_ms": elapsed,
                    "content_preview": ct.content_preview,
                });
            }
            obj
        }).collect::<Vec<_>>(),
        worker_pool: serde_json::json!({
            "status": worker_status.status,
            "size": worker_status.size,
            "active": worker_status.workers.iter().filter(|w| w.status == "busy").count(),
            "idle": worker_status.workers.iter().filter(|w| w.status == "idle").count(),
            "stopped": worker_status.workers.iter().filter(|w| w.status == "stopped").count(),
        }),
        recent_sessions: agent_status.core.recent_sessions.iter().map(|session| {
            let last_user_message = session.history.iter().rev().find(|entry| {
                entry.get("role").and_then(|v| v.as_str()) == Some("user")
            }).and_then(|entry| entry.get("content")).and_then(|v| v.as_str()).unwrap_or("");
            serde_json::json!({
                "chat_id": session.chat_id,
                "channel": session.channel,
                "session_id": session.session_id,
                "history_len": session.history.len(),
                "last_user_message_preview": last_user_message.chars().take(80).collect::<String>(),
            })
        }).collect::<Vec<_>>(),
        failures: serde_json::json!({
            "last_failure": agent_status.core.failures.last_failure.as_ref().map(|failure| serde_json::json!({
                "error_code": failure.error_code,
                "error_stage": failure.error_stage,
                "message_id": failure.message_id,
                "channel": failure.channel,
                "chat_id": failure.chat_id,
                "occurred_at_ms": failure.occurred_at_ms,
                "internal_message": failure.internal_message,
            })),
            "counts_by_error_code": agent_status.core.failures.counts_by_error_code,
        }),
        runtime_timeline: agent_status.core.runtime_timeline.iter().map(|event| serde_json::json!({
            "event": event.event,
            "trace_id": event.trace_id,
            "message_id": event.message_id,
            "channel": event.channel,
            "chat_id": event.chat_id,
            "sender_id": event.sender_id,
            "recorded_at_ms": event.recorded_at_ms,
            "payload": event.payload,
        })).collect::<Vec<_>>(),
        recent_execution_summaries: agent_status.core.recent_execution_summaries.iter().map(|summary| serde_json::json!({
            "trace_id": summary.trace_id,
            "message_id": summary.message_id,
            "channel": summary.channel,
            "chat_id": summary.chat_id,
            "plan_type": summary.plan_type,
            "status": summary.status,
            "cache_hit": summary.cache_hit,
            "worker_id": summary.worker_id,
            "error_code": summary.error_code,
            "error_stage": summary.error_stage,
            "task_duration_ms": summary.task_duration_ms,
            "stages": {
                "context_ms": summary.stages.context_ms,
                "session_ms": summary.stages.session_ms,
                "classify_ms": summary.stages.classify_ms,
                "execute_ms": summary.stages.execute_ms,
                "total_ms": summary.stages.total_ms,
            }
        })).collect::<Vec<_>>(),
        metrics: serde_json::json!({
            "counters": agent_status.core.metrics.counters,
            "gauges": agent_status.core.metrics.gauges,
            "histograms": agent_status.core.metrics.histograms.iter().map(|(name, histogram)| {
                (name.clone(), serde_json::json!({
                    "buckets": histogram.buckets,
                    "counts": histogram.counts,
                    "sum": histogram.sum,
                    "count": histogram.count,
                }))
            }).collect::<serde_json::Map<String, serde_json::Value>>(),
        }),
        jobs,
    }
}
