use crate::jobs::{AcceptedJob, JobKind, JobReviewInput};
use crate::AppState;
use anima_runtime::agent::QuestionAnswerInput;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

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

pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendMessageRequest>,
) -> Json<serde_json::Value> {
    send_message_impl(state, req).await
}

pub async fn send_message_for_session(
    Path(session_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendMessageRequest>,
) -> Json<serde_json::Value> {
    let request = SendMessageRequest {
        content: req.content,
        session_id: Some(session_id),
    };
    send_message_impl(state, request).await
}

async fn send_message_impl(
    state: Arc<AppState>,
    req: SendMessageRequest,
) -> Json<serde_json::Value> {
    let user_content = req.content.clone();
    let requested_session_id = req.session_id.clone();
    let mut msg = anima_runtime::bus::make_inbound(anima_runtime::bus::MakeInbound {
        channel: "web".into(),
        sender_id: Some("web-user".into()),
        chat_id: requested_session_id.clone(),
        content: req.content,
        session_key: requested_session_id.clone(),
        ..Default::default()
    });
    let job_id = msg.id.clone();
    let session_id = requested_session_id.unwrap_or_else(|| job_id.clone());
    msg.chat_id = Some(session_id.clone());
    msg.session_key = Some(session_id.clone());
    {
        let mut jobs = state.jobs.lock();
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
            "session_id": session_id,
        })),
        Err(e) => Json(serde_json::json!({"ok": false, "accepted": false, "error": e})),
    }
}

pub async fn review_job(
    Path(job_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(input): Json<JobReviewInput>,
) -> Json<serde_json::Value> {
    let review = state
        .jobs
        .lock()
        .record_review(job_id.clone(), input);
    let snapshot = crate::web_snapshot::build_status_snapshot(state.as_ref());
    let job = snapshot.jobs.into_iter().find(|job| job.job_id == job_id);
    Json(serde_json::json!({
        "ok": true,
        "job_id": job_id,
        "review": review,
        "job": job,
    }))
}

pub async fn answer_job_question(
    Path(job_id): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(input): Json<QuestionAnswerRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let runtime = state.runtime.lock();
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
            let snapshot = crate::web_snapshot::build_status_snapshot(state.as_ref());
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
