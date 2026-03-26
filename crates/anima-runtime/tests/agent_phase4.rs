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
    assert!(outbound.content.starts_with("Error: "));
    assert!(outbound.content.contains("upstream exploded"));

    let status = agent.status();
    assert!(status.running);
    assert!(status.core.metrics.counters["messages_failed"] >= 1);
    assert!(status.core.metrics.counters["tasks_failed"] >= 1);

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
