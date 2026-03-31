use anima_runtime::agent::{Agent, TaskExecutor, WorkerAgent, WorkerPool};
use anima_runtime::bus::{make_inbound, Bus, MakeInbound};
use anima_runtime::channel::{start_outbound_dispatch, ChannelRegistry, DispatchStats, TestChannel};
use anima_runtime::Channel;
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Default)]
struct SlowExecutor;

#[derive(Debug, Default)]
struct MockExecutor;

impl TaskExecutor for MockExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        Ok(json!({
            "content": format!("reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-1"}))
    }
}

#[derive(Debug, Default)]
struct FailingExecutor;

impl TaskExecutor for FailingExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("upstream exploded".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-err"}))
    }
}

#[derive(Debug, Default)]
struct SessionCreateErrorExecutor;

impl TaskExecutor for SessionCreateErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({"content": "unused"}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Err("create session upstream exploded".into())
    }
}

#[derive(Debug, Default)]
struct MissingSessionIdExecutor;

impl TaskExecutor for MissingSessionIdExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({"content": "unused"}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"ok": true}))
    }
}

#[derive(Debug, Default)]
struct SessionTransportErrorExecutor;

impl TaskExecutor for SessionTransportErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({"content": "unused"}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Err("HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)".into())
    }
}

#[derive(Debug, Default)]
struct UpstreamStreamErrorExecutor;

impl TaskExecutor for UpstreamStreamErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("empty_stream: upstream stream closed before first payload".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-stream-fail"}))
    }
}

#[derive(Debug, Default)]
struct UpstreamTimeoutExecutor;

impl TaskExecutor for UpstreamTimeoutExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("HTTP 408 Request Timeout: stream disconnected before completion: stream closed before response.completed".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-timeout"}))
    }
}

impl TaskExecutor for SlowExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        thread::sleep(Duration::from_millis(200));
        Ok(json!({
            "content": format!("slow-reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        thread::sleep(Duration::from_millis(50));
        Ok(json!({"id": "slow-session-1"}))
    }
}

#[test]
fn worker_executes_api_call_task() {
    let worker = Arc::new(WorkerAgent::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        None,
    ));
    worker.start();

    let rx = worker.submit_task(anima_runtime::agent::make_task(
        anima_runtime::agent::MakeTask {
            task_type: "api-call".into(),
            payload: Some(json!({
                "opencode-session-id": "session-123",
                "content": "Hello"
            })),
            ..Default::default()
        },
    ), None);

    let result = rx.recv().unwrap();
    assert_eq!(result.status, "success");
    assert_eq!(
        result.result.unwrap()["content"],
        "reply[session-123]: Hello"
    );
}

#[test]
fn worker_pool_submits_task_to_available_worker() {
    let pool = WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(50),
    );
    pool.start();

    let rx = pool.submit_task(anima_runtime::agent::make_task(
        anima_runtime::agent::MakeTask {
            task_type: "session-create".into(),
            payload: Some(json!({})),
            ..Default::default()
        },
    ));

    let result = rx.recv().unwrap();
    assert_eq!(result.status, "success");
    assert_eq!(
        result.result.unwrap()["opencode-session-id"],
        "mock-session-1"
    );
}

#[test]
fn agent_processes_messages_and_publishes_outbound() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "Hello, world!".into(),
        ..Default::default()
    }));

    let mut outbound = None;
    for _ in 0..20 {
        if let Some(msg) = bus
            .outbound_receiver()
            .recv_timeout(Duration::from_millis(50))
            .ok()
        {
            outbound = Some(msg);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    let outbound = outbound.expect("expected outbound message");
    assert_eq!(outbound.channel, "test");
    assert_eq!(outbound.reply_target.as_deref(), Some("user-1"));
    assert_eq!(outbound.content, "reply[mock-session-1]: Hello, world!");

    agent.stop();
}

#[test]
fn agent_reuses_created_session_for_same_chat() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "First".into(),
        ..Default::default()
    }));
    let _ = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "Second".into(),
        ..Default::default()
    }));
    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert_eq!(outbound.content, "reply[mock-session-1]: Second");
    assert!(agent.status().running);
    assert_eq!(agent.status().core.sessions_count, 1);
    assert_eq!(agent.status().core.context_status, "running");
    assert!(agent.status().core.metrics.counters["messages_received"] >= 2);
    assert!(agent.status().core.metrics.counters["messages_processed"] >= 2);

    agent.stop();
}

#[test]
fn channel_to_agent_to_dispatch_to_channel_round_trip_succeeds() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let channel = Arc::new(TestChannel::new("test"));
    channel.start();
    registry.register(channel.clone(), None);

    let dispatch_handle = start_outbound_dispatch(bus.clone(), registry.clone(), stats.clone());

    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "Hello, world!".into(),
        ..Default::default()
    }));

    let mut sent = Vec::new();
    for _ in 0..20 {
        sent = channel.sent_messages();
        if !sent.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, "user-1");
    assert_eq!(sent[0].message, "reply[mock-session-1]: Hello, world!");
    assert_eq!(sent[0].opts.stage.as_deref(), Some("final"));

    let status = agent.status();
    assert!(status.running);
    assert_eq!(status.core.sessions_count, 1);
    assert_eq!(status.core.context_status, "running");

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.channel_not_found, 0);

    agent.stop();
    bus.close();
    dispatch_handle.join().unwrap();
}

#[test]
fn agent_publishes_error_response_when_worker_execution_fails() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(FailingExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-err".into()),
        chat_id: Some("chat-err".into()),
        content: "Boom".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert_eq!(outbound.channel, "test");
    assert_eq!(outbound.reply_target.as_deref(), Some("user-err"));
    assert_eq!(outbound.stage, "final");
    assert!(outbound.content.starts_with("Error [task_execution_failed]: "));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("message_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "task_execution_failed");
                assert_eq!(payload["error_stage"], "plan_execute");
                assert!(payload["error"].as_str().unwrap_or("").contains("upstream exploded"));
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected message_failed runtime event");

    let status = agent.status();
    assert!(status.running);
    assert!(status.core.metrics.counters["messages_failed"] >= 1);
    assert!(status.core.metrics.counters["tasks_failed"] >= 1);
    assert_eq!(status.core.failures.counts_by_error_code["task_execution_failed"], 1);
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "task_execution_failed");
    assert_eq!(last_failure.error_stage, "plan_execute");
    assert_eq!(last_failure.channel, "test");
    assert_eq!(last_failure.chat_id.as_deref(), Some("chat-err"));
    assert!(last_failure.internal_message.contains("upstream exploded"));

    agent.stop();
}

#[test]
fn agent_classifies_upstream_stream_error_during_plan_execution() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(UpstreamStreamErrorExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-stream-err".into()),
        chat_id: Some("chat-stream-err".into()),
        content: "Explain HTTP".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [upstream_stream_failed]: "));
    assert!(outbound.content.contains("流式响应异常中断"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("message_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "upstream_stream_failed");
                assert_eq!(payload["error_stage"], "plan_execute");
                assert_eq!(payload["error"], "empty_stream: upstream stream closed before first payload");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected message_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "upstream_stream_failed");
    assert_eq!(last_failure.error_stage, "plan_execute");
    assert_eq!(last_failure.internal_message, "empty_stream: upstream stream closed before first payload");

    agent.stop();
}

#[test]
fn agent_classifies_upstream_timeout_during_plan_execution() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(UpstreamTimeoutExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-timeout-err".into()),
        chat_id: Some("chat-timeout-err".into()),
        content: "Explain HTTP".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [upstream_timeout]: "));
    assert!(outbound.content.contains("响应超时"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("message_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "upstream_timeout");
                assert_eq!(payload["error_stage"], "plan_execute");
                assert!(payload["error"].as_str().unwrap_or("").contains("Request Timeout"));
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected message_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "upstream_timeout");
    assert_eq!(last_failure.error_stage, "plan_execute");
    assert!(last_failure.internal_message.contains("Request Timeout"));

    agent.stop();
}

#[test]
fn agent_emits_runtime_events_and_recent_sessions_in_status() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-obs".into()),
        chat_id: Some("chat-obs".into()),
        content: "Observe me".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(outbound.content, "reply[mock-session-1]: Observe me");

    let mut events = Vec::new();
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if let Some(event) = msg.payload.get("event").and_then(|v| v.as_str()) {
                events.push(event.to_string());
            }
            if events.iter().any(|e| e == "message_completed") {
                break;
            }
        }
    }

    assert!(events.iter().any(|e| e == "message_received"));
    assert!(events.iter().any(|e| e == "session_ready"));
    assert!(events.iter().any(|e| e == "plan_built"));
    assert!(events.iter().any(|e| e == "message_completed"));

    let status = agent.status();
    assert_eq!(status.core.recent_sessions.len(), 1);
    assert_eq!(status.core.recent_sessions[0].chat_id, "chat-obs");
    assert_eq!(status.core.recent_sessions[0].channel, "test");
    assert_eq!(status.core.recent_sessions[0].session_id.as_deref(), Some("mock-session-1"));
    assert_eq!(status.core.metrics.gauges["sessions_active"], 1);

    let timeline_events = status
        .core
        .runtime_timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(timeline_events.contains(&"message_received"));
    assert!(timeline_events.contains(&"session_ready"));
    assert!(timeline_events.contains(&"plan_built"));
    assert!(timeline_events.contains(&"message_completed"));

    let message_received_pos = timeline_events.iter().position(|event| *event == "message_received").unwrap();
    let session_ready_pos = timeline_events.iter().position(|event| *event == "session_ready").unwrap();
    let plan_built_pos = timeline_events.iter().position(|event| *event == "plan_built").unwrap();
    let message_completed_pos = timeline_events.iter().position(|event| *event == "message_completed").unwrap();
    assert!(message_received_pos < session_ready_pos);
    assert!(session_ready_pos < plan_built_pos);
    assert!(plan_built_pos < message_completed_pos);

    assert_eq!(status.core.recent_execution_summaries.len(), 1);
    let summary = &status.core.recent_execution_summaries[0];
    assert_eq!(summary.plan_type, "single");
    assert_eq!(summary.status, "success");
    assert!(!summary.cache_hit);
    assert!(summary.stages.total_ms >= summary.stages.execute_ms);
    assert!(summary.stages.total_ms >= summary.stages.context_ms);
    assert!(summary.stages.total_ms >= summary.stages.session_ms);
    assert!(summary.stages.total_ms >= summary.stages.classify_ms);

    agent.stop();
}

#[test]
fn agent_preserves_executor_session_create_error_across_failure_surfaces() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(SessionCreateErrorExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-session-create-err".into()),
        chat_id: Some("chat-session-create-err".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(outbound.channel, "test");
    assert_eq!(outbound.reply_target.as_deref(), Some("user-session-create-err"));
    assert_eq!(outbound.stage, "final");
    assert!(outbound.content.starts_with("Error [session_create_failed]: "));
    assert!(outbound.content.contains("无法创建上游会话"));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "session_create_failed");
                assert_eq!(payload["error_stage"], "session_create");
                assert_eq!(payload["error_message"], "create session upstream exploded");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "session_create_failed");
    assert_eq!(last_failure.error_stage, "session_create");
    assert_eq!(last_failure.channel, "test");
    assert_eq!(last_failure.chat_id.as_deref(), Some("chat-session-create-err"));
    assert_eq!(last_failure.internal_message, "create session upstream exploded");
    assert!(!last_failure.internal_message.contains("Unknown runtime error"));

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(timeline_event.payload["error_message"], "create session upstream exploded");
    assert!(!timeline_event.payload["error_message"]
        .as_str()
        .unwrap_or("")
        .contains("Unknown runtime error"));

    agent.stop();
}

#[test]
fn agent_preserves_missing_session_id_error_across_failure_surfaces() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MissingSessionIdExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-missing-session-id".into()),
        chat_id: Some("chat-missing-session-id".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [session_create_failed]: "));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "session_create_failed");
                assert_eq!(payload["error_stage"], "session_create");
                assert_eq!(payload["error_message"], "Failed to create session: no ID returned");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "session_create_failed");
    assert_eq!(last_failure.error_stage, "session_create");
    assert_eq!(last_failure.internal_message, "Failed to create session: no ID returned");
    assert!(!last_failure.internal_message.contains("Unknown runtime error"));

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(timeline_event.payload["error_message"], "Failed to create session: no ID returned");

    agent.stop();
}

#[test]
fn agent_classifies_session_transport_error_as_session_create_failure() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(SessionTransportErrorExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-session-transport-error".into()),
        chat_id: Some("chat-session-transport-error".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [session_create_failed]: "));
    assert!(outbound.content.contains("无法创建上游会话"));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "session_create_failed");
                assert_eq!(payload["error_stage"], "session_create");
                assert_eq!(
                    payload["error_message"],
                    "HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)"
                );
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "session_create_failed");
    assert_eq!(last_failure.error_stage, "session_create");
    assert_eq!(
        last_failure.internal_message,
        "HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)"
    );

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(
        timeline_event.payload["error_message"],
        "HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)"
    );

    agent.stop();
}

#[test]
fn agent_preserves_worker_pool_unavailable_error_during_session_create() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );

    agent.core_agent().process_inbound_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-worker-unavailable".into()),
        chat_id: Some("chat-worker-unavailable".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [worker_unavailable]: "));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "worker_unavailable");
                assert_eq!(payload["error_stage"], "worker_pool");
                assert_eq!(payload["error_message"], "Worker pool is not running");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "worker_unavailable");
    assert_eq!(last_failure.error_stage, "worker_pool");
    assert_eq!(last_failure.internal_message, "Worker pool is not running");
    assert!(!last_failure.internal_message.contains("Unknown runtime error"));

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(timeline_event.payload["error_message"], "Worker pool is not running");

    agent.stop();
}

#[test]
fn agent_exposes_busy_worker_current_task_during_long_execution() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(SlowExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-slow".into()),
        chat_id: Some("chat-slow".into()),
        content: "summarize a very long task".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    let mut saw_busy = false;
    for _ in 0..20 {
        let status = agent.status();
        if let Some(worker) = status.core.worker_pool.workers.iter().find(|worker| worker.status == "busy") {
            let current_task = worker.current_task.as_ref().expect("busy worker should expose current task");
            assert_eq!(current_task.trace_id, job_id);
            assert!(current_task.started_ms > 0);
            saw_busy = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(saw_busy, "expected a busy worker during slow execution");

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert!(outbound.content.contains("slow-reply[slow-session-1]"));

    agent.stop();
}

#[test]
fn integration_routes_to_named_channel_when_multiple_channels_are_registered() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let primary = Arc::new(TestChannel::new("test"));
    let secondary = Arc::new(TestChannel::new("test"));
    primary.start();
    secondary.start();
    registry.register(primary.clone(), Some("primary"));
    registry.register(secondary.clone(), Some("secondary"));

    let dispatch_handle = start_outbound_dispatch(bus.clone(), registry.clone(), stats.clone());

    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-multi".into()),
        chat_id: Some("chat-multi".into()),
        content: "Route me".into(),
        metadata: Some(json!({"account-id": "secondary"})),
        ..Default::default()
    }));

    for _ in 0..20 {
        if secondary.sent_messages().len() == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let primary_sent = primary.sent_messages();
    let secondary_sent = secondary.sent_messages();

    assert!(primary_sent.is_empty());
    assert_eq!(secondary_sent.len(), 1);
    assert_eq!(secondary_sent[0].target, "user-multi");
    assert_eq!(secondary_sent[0].message, "reply[mock-session-1]: Route me");
    assert_eq!(secondary_sent[0].opts.stage.as_deref(), Some("final"));

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.channel_not_found, 0);

    agent.stop();
    bus.close();
    dispatch_handle.join().unwrap();
}

#[test]
fn integration_mixed_dispatch_outcomes_do_not_block_successful_messages() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let ok_channel = Arc::new(TestChannel::new("test"));
    let failing_channel = Arc::new(TestChannel::failing("test"));
    ok_channel.start();
    failing_channel.start();
    registry.register(ok_channel.clone(), Some("ok-account"));
    registry.register(failing_channel.clone(), Some("bad-account"));

    let dispatch_handle = start_outbound_dispatch(bus.clone(), registry.clone(), stats.clone());

    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-ok".into()),
        chat_id: Some("chat-ok".into()),
        content: "deliver".into(),
        metadata: Some(json!({"account-id": "ok-account"})),
        ..Default::default()
    }));

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-bad".into()),
        chat_id: Some("chat-bad".into()),
        content: "fail-route".into(),
        metadata: Some(json!({"account-id": "bad-account"})),
        ..Default::default()
    }));

    agent.process_message(make_inbound(MakeInbound {
        channel: "missing".into(),
        sender_id: Some("user-missing".into()),
        chat_id: Some("chat-missing".into()),
        content: "missing-route".into(),
        metadata: Some(json!({"account-id": "missing-account"})),
        ..Default::default()
    }));

    for _ in 0..20 {
        if ok_channel.sent_messages().len() == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let ok_sent = ok_channel.sent_messages();
    assert_eq!(ok_sent.len(), 1);
    assert_eq!(ok_sent[0].target, "user-ok");
    assert_eq!(ok_sent[0].message, "reply[mock-session-1]: deliver");
    assert_eq!(ok_sent[0].opts.stage.as_deref(), Some("final"));

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 1);
    assert_eq!(snapshot.channel_not_found, 1);

    agent.stop();
    bus.close();
    dispatch_handle.join().unwrap();
}
