use anima_runtime::agent::executor::TaskExecutor;
use anima_runtime::classifier::router::*;
use anima_runtime::orchestrator::specialist_pool::SpecialistPool;
use anima_runtime::agent::worker::WorkerPool;
use anima_runtime::bus::message::{make_inbound, MakeInbound};
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::Arc;

struct TestExecutor;

impl TaskExecutor for TestExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        _content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"content": format!("reply[{session_id}]")}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "test-session-1"}))
    }
}

fn make_test_pool() -> Arc<WorkerPool> {
    let client = SdkClient::new("http://127.0.0.1:9711");
    let executor: Arc<dyn TaskExecutor> = Arc::new(TestExecutor);
    let pool = WorkerPool::new(client, executor, Some(2), None, Some(500));
    pool.start();
    Arc::new(pool)
}

fn make_router() -> IntelligentRouter {
    let pool = make_test_pool();
    let specialist_pool = Arc::new(SpecialistPool::new(pool));
    IntelligentRouter::new(specialist_pool, IntelligentRouterConfig::default())
}

fn test_message(content: &str) -> anima_runtime::bus::InboundMessage {
    make_inbound(MakeInbound {
        channel: "cli".into(),
        content: content.into(),
        metadata: Some(json!({"session-id": "test-session-1"})),
        ..Default::default()
    })
}

// ── Basic routing ───────────────────────────────────────────────────

#[test]
fn router_routes_simple_message() {
    let router = make_router();
    let msg = test_message("hello");
    let dialog = router.route(&msg);

    assert!(!dialog.id.is_empty());
    assert_eq!(dialog.session_id, "test-session-1");
    assert!(dialog.result.is_some());
    assert!(dialog.completed_at_ms.is_some());
}

#[test]
fn router_creates_dialog_context() {
    let router = make_router();
    let msg = test_message("hello");
    router.route(&msg);

    assert_eq!(router.context_count(), 1);
    let ctx = router.get_context("test-session-1").unwrap();
    assert_eq!(ctx.session_id, "test-session-1");
    assert_eq!(ctx.message_history.len(), 1);
}

#[test]
fn router_accumulates_message_history() {
    let router = make_router();
    router.route(&test_message("first"));
    router.route(&test_message("second"));
    router.route(&test_message("third"));

    let ctx = router.get_context("test-session-1").unwrap();
    assert_eq!(ctx.message_history.len(), 3);
}

// ── Metrics ─────────────────────────────────────────────────────────

#[test]
fn router_tracks_metrics() {
    let router = make_router();
    router.route(&test_message("hello"));
    router.route(&test_message("world"));

    let metrics = router.metrics();
    assert_eq!(metrics.total_routed, 2);
    assert_eq!(metrics.total_completed, 2);
}

// ── Multiple sessions ───────────────────────────────────────────────

#[test]
fn router_maintains_separate_contexts_per_session() {
    let router = make_router();

    let msg1 = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "hello".into(),
        metadata: Some(json!({"session-id": "session-a"})),
        ..Default::default()
    });
    let msg2 = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "world".into(),
        metadata: Some(json!({"session-id": "session-b"})),
        ..Default::default()
    });

    router.route(&msg1);
    router.route(&msg2);

    assert_eq!(router.context_count(), 2);
    let ctx_a = router.get_context("session-a").unwrap();
    let ctx_b = router.get_context("session-b").unwrap();
    assert_eq!(ctx_a.message_history.len(), 1);
    assert_eq!(ctx_b.message_history.len(), 1);
}

// ── Specialist routing ──────────────────────────────────────────────

#[test]
fn router_routes_to_specialist_when_specified() {
    let pool = make_test_pool();
    let specialist_pool = Arc::new(SpecialistPool::new(pool.clone()));

    let code_pool = make_test_pool();
    specialist_pool.register("code-review", code_pool);

    let router = IntelligentRouter::new(specialist_pool, IntelligentRouterConfig::default());

    let msg = make_inbound(MakeInbound {
        channel: "cli".into(),
        content: "review this code".into(),
        metadata: Some(json!({
            "session-id": "test-session",
            "specialist": "code-review"
        })),
        ..Default::default()
    });

    let dialog = router.route(&msg);
    assert!(dialog.result.is_some());
    assert_eq!(router.metrics().total_routed, 1);
}

// ── Dialog context unit ─────────────────────────────────────────────

#[test]
fn dialog_context_records_messages_and_tasks() {
    let mut ctx = DialogContext::new("s1", "cli", "user1");
    assert!(ctx.message_history.is_empty());
    assert!(ctx.task_history.is_empty());

    ctx.record_message(json!({"text": "hello"}), 100);
    ctx.record_task(json!({"task_id": "task-1"}), 100);

    assert_eq!(ctx.message_history.len(), 1);
    assert_eq!(ctx.task_history.len(), 1);
}

// ── Lifecycle ───────────────────────────────────────────────────────

#[test]
fn router_lifecycle_start_stop() {
    let router = make_router();
    assert!(!router.is_running());

    router.start();
    assert!(router.is_running());

    router.stop();
    assert!(!router.is_running());
}

// ── Context cleanup ─────────────────────────────────────────────────

#[test]
fn router_cleanup_removes_old_contexts() {
    let pool = make_test_pool();
    let specialist_pool = Arc::new(SpecialistPool::new(pool));
    let config = IntelligentRouterConfig {
        max_context_age_ms: 0, // expire immediately
        ..Default::default()
    };
    let router = IntelligentRouter::new(specialist_pool, config);

    router.route(&test_message("hello"));
    assert_eq!(router.context_count(), 1);

    let removed = router.cleanup_old_contexts();
    assert_eq!(removed, 1);
    assert_eq!(router.context_count(), 0);
}

// ── Classification-based routing ────────────────────────────────────

#[test]
fn router_classifies_and_routes_message() {
    let router = make_router();
    let result = router.process_message("Write a function", "s1", "cli", "user1");
    assert_eq!(result.status, ProcessStatus::Success);
    assert!(result.classification.is_some());
    assert!(result.specialist_result.is_some());
}

// ── Confidence threshold ────────────────────────────────────────────

#[test]
fn router_rejects_low_confidence_without_fallback() {
    let pool = make_test_pool();
    let specialist_pool = Arc::new(SpecialistPool::new(pool));
    let config = IntelligentRouterConfig {
        classification_threshold: 1.0, // impossibly high
        fallback_to_general: false,
        ..Default::default()
    };
    let router = IntelligentRouter::new(specialist_pool, config);
    let result = router.process_message("x", "s1", "cli", "user1");
    assert_eq!(result.status, ProcessStatus::Error);
    assert!(result.error.unwrap().contains("confidence"));
}

// ── Format response ─────────────────────────────────────────────────

#[test]
fn format_response_success() {
    let result = ProcessResult {
        status: ProcessStatus::Success,
        classification: None,
        specialist_result: None,
        error: None,
        processing_time_ms: 42,
    };
    let resp = format_response(&result);
    assert!(resp.success);
    assert_eq!(resp.message, "处理完成");
}

#[test]
fn format_response_error() {
    let result = ProcessResult {
        status: ProcessStatus::Error,
        classification: None,
        specialist_result: None,
        error: Some("boom".into()),
        processing_time_ms: 0,
    };
    let resp = format_response(&result);
    assert!(!resp.success);
    assert!(resp.message.contains("处理失败"));
}

#[test]
fn format_response_timeout() {
    let result = ProcessResult {
        status: ProcessStatus::Timeout,
        classification: None,
        specialist_result: None,
        error: None,
        processing_time_ms: 60000,
    };
    let resp = format_response(&result);
    assert!(!resp.success);
    assert!(resp.message.contains("超时"));
}

// ── Max history trimming ────────────────────────────────────────────

#[test]
fn context_trims_message_history_to_max() {
    use anima_runtime::classifier::task::classify_task;

    let mut ctx = DialogContext::new("s1", "cli", "user1");
    let classification = classify_task("hello");

    for i in 0..10 {
        ctx.record_message_with_classification(
            &format!("msg-{i}"),
            &classification,
            5, // max 5
        );
    }

    assert_eq!(ctx.message_history.len(), 5);
    // Should keep the last 5
    assert!(ctx.message_history[0]["content"]
        .as_str()
        .unwrap()
        .contains("msg-5"));
}

// ── Status ──────────────────────────────────────────────────────────

#[test]
fn router_status_includes_all_fields() {
    let router = make_router();
    router.start();
    let status = router.status();
    assert_eq!(status["running"], true);
    assert!(status["id"].is_string());
    assert!(status["context_count"].is_number());
}
