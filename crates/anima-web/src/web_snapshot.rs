use crate::jobs::build_job_views_with_projection;
use crate::AppState;
use anima_runtime::messages::types::MessageRole;
use anima_runtime::runtime::build_projection;
use anima_runtime::transcript::ContentBlock;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct SessionListItem {
    pub session_id: String,
    pub chat_id: String,
    pub channel: String,
    pub history_len: usize,
    pub last_user_message_preview: String,
    pub last_active: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionHistoryItem {
    pub role: Option<String>,
    pub content: Value,
    pub recorded_at: Option<u64>,
    pub raw: Value,
}

pub struct StatusSnapshot {
    pub agent: serde_json::Value,
    pub workers: Vec<serde_json::Value>,
    pub worker_pool: serde_json::Value,
    pub recent_sessions: Vec<serde_json::Value>,
    pub failures: serde_json::Value,
    pub runtime_timeline: Vec<serde_json::Value>,
    pub recent_execution_summaries: Vec<serde_json::Value>,
    pub metrics: serde_json::Value,
    pub warnings: serde_json::Value,
    pub unified_runtime: serde_json::Value,
    pub jobs: Vec<crate::jobs::JobView>,
}

pub fn json_content_preview(content: &Value) -> String {
    match content {
        Value::String(text) => text.chars().take(80).collect(),
        other => serde_json::to_string(other)
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect(),
    }
}

pub fn transcript_content_preview(blocks: &[ContentBlock]) -> String {
    let text = blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    text.chars().take(80).collect()
}

pub fn build_session_summaries_from_runtime(
    snapshot: &anima_runtime::runtime::RuntimeStateSnapshot,
) -> Vec<SessionListItem> {
    let mut sessions = HashMap::<String, SessionListItem>::new();

    for run in snapshot.runs.values() {
        let Some(chat_id) = run.chat_id.clone() else {
            continue;
        };

        let item = sessions
            .entry(chat_id.clone())
            .or_insert_with(|| SessionListItem {
                session_id: chat_id.clone(),
                chat_id: chat_id.clone(),
                channel: run.channel.clone(),
                history_len: 0,
                last_user_message_preview: String::new(),
                last_active: run.updated_at_ms,
            });
        item.last_active = item.last_active.max(run.updated_at_ms);
    }

    for message in &snapshot.transcript {
        let Some(run) = snapshot.runs.get(&message.run_id) else {
            continue;
        };
        let Some(chat_id) = run.chat_id.clone() else {
            continue;
        };

        let preview = transcript_content_preview(&message.blocks);
        let item = sessions
            .entry(chat_id.clone())
            .or_insert_with(|| SessionListItem {
                session_id: chat_id.clone(),
                chat_id: chat_id.clone(),
                channel: run.channel.clone(),
                history_len: 0,
                last_user_message_preview: String::new(),
                last_active: message.appended_at_ms,
            });
        item.history_len += 1;
        item.last_active = item.last_active.max(message.appended_at_ms);
        if message.role == MessageRole::User && !preview.is_empty() {
            item.last_user_message_preview = preview;
        }
    }

    sessions.into_values().collect()
}

pub fn build_status_snapshot(state: &AppState) -> StatusSnapshot {
    let runtime = state.runtime.lock();
    let agent_status = runtime.agent.status();
    let runtime_snapshot = runtime.agent.core_agent().runtime_state_snapshot();
    let runtime_projection = build_projection(&runtime_snapshot);
    let worker_status = agent_status.core.worker_pool.clone();
    let bus_telemetry = state.bus.telemetry_snapshot();
    let failure_list = agent_status
        .core
        .failures
        .last_failure
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let jobs = {
        let store = state.jobs.lock();
        build_job_views_with_projection(
            &agent_status.core.runtime_timeline,
            &agent_status.core.recent_execution_summaries,
            &failure_list,
            &worker_status.workers,
            &store,
            &runtime_snapshot,
            &runtime_projection,
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
            let last_user_message = session.history.iter().rev().find(|entry: &&serde_json::Value| {
                entry.get("role").and_then(|v| v.as_str()) == Some("user")
            }).and_then(|entry: &serde_json::Value| entry.get("content")).and_then(|v| v.as_str()).unwrap_or("");
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
        metrics: {
            let mut counters = agent_status.core.metrics.counters.clone();
            counters.insert("bus_inbound_dropped_total".into(), bus_telemetry.inbound_dropped_total);
            counters.insert("bus_outbound_dropped_total".into(), bus_telemetry.outbound_dropped_total);
            counters.insert("bus_internal_dropped_total".into(), bus_telemetry.internal_dropped_total);
            counters.insert("bus_control_dropped_total".into(), bus_telemetry.control_dropped_total);

            let mut gauges = agent_status.core.metrics.gauges.clone();
            gauges.insert("bus_inbound_queue_depth".into(), bus_telemetry.inbound_queue_depth as i64);
            gauges.insert("bus_outbound_queue_depth".into(), bus_telemetry.outbound_queue_depth as i64);
            gauges.insert("bus_internal_queue_depth".into(), bus_telemetry.internal_queue_depth as i64);
            gauges.insert("bus_control_queue_depth".into(), bus_telemetry.control_queue_depth as i64);

            serde_json::json!({
                "counters": counters,
                "gauges": gauges,
                "histograms": agent_status.core.metrics.histograms.iter().map(|entry| {
                    let (name, histogram) = entry;
                    (name.clone(), serde_json::json!({
                        "buckets": histogram.buckets,
                        "counts": histogram.counts,
                        "sum": histogram.sum,
                        "count": histogram.count,
                    }))
                }).collect::<serde_json::Map<String, serde_json::Value>>(),
            })
        },
        warnings: serde_json::json!({
            "bus_overflow_active": bus_telemetry.inbound_dropped_total > 0
                || bus_telemetry.outbound_dropped_total > 0
                || bus_telemetry.internal_dropped_total > 0
                || bus_telemetry.control_dropped_total > 0,
            "bus_drop_total": bus_telemetry.inbound_dropped_total
                + bus_telemetry.outbound_dropped_total
                + bus_telemetry.internal_dropped_total
                + bus_telemetry.control_dropped_total,
        }),
        unified_runtime: serde_json::json!({
            "runs": runtime_projection.runs,
            "turns": runtime_projection.turns,
            "tasks": runtime_projection.tasks,
            "suspensions": runtime_projection.suspensions,
            "tool_invocations": runtime_projection.tool_invocations,
            "requirements": runtime_projection.requirements,
            "transcript": runtime_projection.transcript,
            "execution_summaries": runtime_projection.execution_summaries,
            "failures": runtime_projection.failures,
            "orchestration": runtime_projection.orchestration,
            "pending_questions": runtime_projection.pending_questions,
            "tool_states": runtime_projection.tool_states,
            "job_statuses": runtime_projection.job_statuses,
            "recent_events": runtime_snapshot.recent_events,
        }),
        jobs,
    }
}
