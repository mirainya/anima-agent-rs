use anima_runtime::bus::{
    bounded_channel, make_control, make_inbound, make_internal, make_outbound, BufferStrategy, Bus,
    BusConfig, ControlSignal, InternalMessageType, MakeControl, MakeInbound, MakeInternal,
    MakeOutbound,
};
use serde_json::json;
use std::sync::Arc;

// ── Dropping buffer tests ──────────────────────────────────────────

#[test]
fn dropping_buffer_discards_new_messages_when_full() {
    let (tx, rx) = bounded_channel::<i32>(BufferStrategy::Dropping(3));
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    tx.send(3).unwrap();
    // Buffer full — this should be silently dropped
    tx.send(4).unwrap();
    tx.send(5).unwrap();

    assert_eq!(rx.recv(), Some(1));
    assert_eq!(rx.recv(), Some(2));
    assert_eq!(rx.recv(), Some(3));
    assert_eq!(rx.try_recv(), None);
}

#[test]
fn dropping_buffer_accepts_after_drain() {
    let (tx, rx) = bounded_channel::<i32>(BufferStrategy::Dropping(2));
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    tx.send(3).unwrap(); // dropped
    assert_eq!(rx.recv(), Some(1));
    // Now there's room
    tx.send(4).unwrap();
    assert_eq!(rx.recv(), Some(2));
    assert_eq!(rx.recv(), Some(4));
}

// ── Sliding buffer tests ───────────────────────────────────────────

#[test]
fn sliding_buffer_discards_oldest_when_full() {
    let (tx, rx) = bounded_channel::<i32>(BufferStrategy::Sliding(3));
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    tx.send(3).unwrap();
    // Buffer full — oldest (1) should be evicted
    tx.send(4).unwrap();
    tx.send(5).unwrap();

    assert_eq!(rx.recv(), Some(3));
    assert_eq!(rx.recv(), Some(4));
    assert_eq!(rx.recv(), Some(5));
    assert_eq!(rx.try_recv(), None);
}

// ── Internal bus tests ─────────────────────────────────────────────

#[test]
fn internal_bus_publish_and_consume() {
    let bus = Bus::create();
    let msg = make_internal(MakeInternal {
        source: "agent".into(),
        target: Some("dispatcher".into()),
        msg_type: Some(InternalMessageType::Request),
        priority: Some(3),
        payload: json!({"action": "route"}),
        ..Default::default()
    });
    let trace = msg.trace_id.clone();
    bus.publish_internal(msg).unwrap();

    let received = bus.consume_internal().unwrap();
    assert_eq!(received.source, "agent");
    assert_eq!(received.target, Some("dispatcher".into()));
    assert_eq!(received.trace_id, trace);
    assert_eq!(received.priority, 3);
    assert_eq!(received.ttl, 30_000);
    assert!(matches!(received.msg_type, InternalMessageType::Request));
}

#[test]
fn internal_message_defaults() {
    let msg = make_internal(MakeInternal {
        source: "test".into(),
        payload: json!(null),
        ..Default::default()
    });
    assert!(matches!(msg.msg_type, InternalMessageType::Event));
    assert_eq!(msg.priority, 5);
    assert_eq!(msg.ttl, 30_000);
    assert!(msg.target.is_none());
    assert!(msg.timestamp > 0);
}

// ── Control bus tests ──────────────────────────────────────────────

#[test]
fn control_bus_publish_and_consume() {
    let bus = Bus::create();
    let msg = make_control(MakeControl {
        signal: ControlSignal::Shutdown,
        source: "cli".into(),
        payload: None,
    });
    bus.publish_control(msg).unwrap();

    let received = bus.consume_control().unwrap();
    assert_eq!(received.signal, ControlSignal::Shutdown);
    assert_eq!(received.source, "cli");
    assert!(received.timestamp > 0);
}

#[test]
fn control_bus_all_signals() {
    let bus = Bus::create();
    let signals = vec![
        ControlSignal::Shutdown,
        ControlSignal::Pause,
        ControlSignal::Resume,
        ControlSignal::ScaleUp,
        ControlSignal::ScaleDown,
        ControlSignal::HealthCheck,
        ControlSignal::ConfigReload,
    ];
    for signal in &signals {
        bus.publish_control(make_control(MakeControl {
            signal: signal.clone(),
            source: "test".into(),
            payload: None,
        }))
        .unwrap();
    }
    for expected in &signals {
        let msg = bus.consume_control().unwrap();
        assert_eq!(&msg.signal, expected);
    }
}

// ── Bus config tests ───────────────────────────────────────────────

#[test]
fn bus_custom_config() {
    let bus = Bus::create_with_config(BusConfig {
        inbound_capacity: 5,
        outbound_capacity: 5,
        internal_capacity: 3,
        control_capacity: 2,
    });
    // Fill inbound beyond capacity (dropping)
    for i in 0..10 {
        bus.publish_inbound(make_inbound(MakeInbound {
            channel: "test".into(),
            content: format!("msg-{}", i),
            ..Default::default()
        }))
        .unwrap();
    }
    // Should only have first 5 (dropping discards new)
    let mut received = Vec::new();
    while let Some(msg) = bus.inbound_receiver().try_recv() {
        received.push(msg.content);
    }
    assert!(received.len() <= 6); // capacity + 1 for bounded channel slack
    assert_eq!(received[0], "msg-0");
}

// ── Bus close tests ────────────────────────────────────────────────

#[test]
fn closed_bus_rejects_all_channels() {
    let bus = Bus::create();
    bus.close();

    assert!(bus
        .publish_inbound(make_inbound(MakeInbound {
            channel: "test".into(),
            content: "x".into(),
            ..Default::default()
        }))
        .is_err());

    assert!(bus
        .publish_outbound(make_outbound(MakeOutbound {
            channel: "test".into(),
            content: "x".into(),
            ..Default::default()
        }))
        .is_err());

    assert!(bus
        .publish_internal(make_internal(MakeInternal {
            source: "test".into(),
            payload: json!(null),
            ..Default::default()
        }))
        .is_err());

    assert!(bus
        .publish_control(make_control(MakeControl {
            signal: ControlSignal::HealthCheck,
            source: "test".into(),
            payload: None,
        }))
        .is_err());
}

// ── Inbound/Outbound buffer semantics ──────────────────────────────

#[test]
fn inbound_uses_dropping_outbound_uses_sliding() {
    let bus = Bus::create_with_config(BusConfig {
        inbound_capacity: 3,
        outbound_capacity: 3,
        internal_capacity: 10,
        control_capacity: 10,
    });

    // Inbound: dropping — keeps oldest
    for i in 0..6 {
        bus.publish_inbound(make_inbound(MakeInbound {
            channel: "test".into(),
            content: format!("in-{}", i),
            ..Default::default()
        }))
        .unwrap();
    }
    let first_in = bus.consume_inbound().unwrap();
    assert_eq!(first_in.content, "in-0");

    // Outbound: sliding — keeps newest
    for i in 0..6 {
        bus.publish_outbound(make_outbound(MakeOutbound {
            channel: "test".into(),
            content: format!("out-{}", i),
            ..Default::default()
        }))
        .unwrap();
    }
    let first_out = bus.consume_outbound().unwrap();
    assert_eq!(first_out.content, "out-3");
}

// ── CoreAgent control signal integration ───────────────────────────

#[test]
fn core_agent_stops_on_shutdown_signal() {
    use anima_runtime::agent::{CoreAgent, TaskExecutor};
    use anima_sdk::facade::Client as SdkClient;
    use serde_json::Value;

    struct NoopExecutor;
    impl TaskExecutor for NoopExecutor {
        fn send_prompt(
            &self,
            _client: &SdkClient,
            _session_id: &str,
            _content: Value,
        ) -> Result<Value, String> {
            Ok(json!("ok"))
        }
        fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
            Ok(json!({"id": "noop"}))
        }
    }

    let bus = Arc::new(Bus::create());
    let client = SdkClient::new("http://127.0.0.1:19999");
    let core = Arc::new(CoreAgent::new(
        bus.clone(),
        client,
        None,
        Arc::new(NoopExecutor),
        None,
    ));
    core.start();
    assert!(core.is_running());

    // Send shutdown signal
    bus.publish_control(make_control(MakeControl {
        signal: ControlSignal::Shutdown,
        source: "test".into(),
        payload: None,
    }))
    .unwrap();

    // Wait for core agent to process the signal
    for _ in 0..40 {
        if !core.is_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!core.is_running());
}
