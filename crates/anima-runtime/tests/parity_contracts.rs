use anima_runtime::bus::{make_inbound, make_outbound, Bus, MakeInbound, MakeOutbound};
use anima_runtime::channel::{ChannelLookupReason, ChannelRegistry, SessionStore, TestChannel};
use anima_runtime::context::ContextManager;
use anima_runtime::dispatcher::{
    Balancer, BalancerMissReason, BalancerOptions, BalancerRuntimeConfig, BalancerStrategy,
    CircuitBreakerConfig, Dispatcher, HealthPolicy, Target, TargetStatus,
};
use anima_runtime::metrics::MetricsCollector;
use serde_json::json;
use std::sync::Arc;

#[test]
fn bus_publish_consume_preserves_message_shape() {
    let bus = Bus::create();
    let inbound = make_inbound(MakeInbound {
        channel: "cli".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "hello".into(),
        session_key: Some("anima.session.chat-1".into()),
        metadata: Some(json!({"source": "test"})),
        ..Default::default()
    });
    let outbound = make_outbound(MakeOutbound {
        channel: "cli".into(),
        account_id: Some("default".into()),
        chat_id: Some("chat-1".into()),
        content: "world".into(),
        reply_target: Some("user-1".into()),
        stage: Some("final".into()),
        ..Default::default()
    });

    bus.publish_inbound(inbound.clone()).unwrap();
    bus.publish_outbound(outbound.clone()).unwrap();

    assert_eq!(bus.consume_inbound(), Some(inbound));
    assert_eq!(bus.consume_outbound(), Some(outbound));
}

#[test]
fn channel_registry_prefers_exact_then_default_then_first_available() {
    let registry = ChannelRegistry::new();
    registry.register(Arc::new(TestChannel::new("cli")), Some("default"));
    registry.register(Arc::new(TestChannel::new("cli")), Some("tenant-a"));

    let (_, exact) = registry.find_channel_with_lookup("cli", Some("tenant-a"));
    assert_eq!(exact.reason, ChannelLookupReason::ExactMatch);
    assert_eq!(exact.matched_account_id.as_deref(), Some("tenant-a"));

    let (_, default_fallback) = registry.find_channel_with_lookup("cli", Some("tenant-b"));
    assert_eq!(
        default_fallback.reason,
        ChannelLookupReason::DefaultFallback
    );
    assert_eq!(
        default_fallback.matched_account_id.as_deref(),
        Some("default")
    );

    let first_available_registry = ChannelRegistry::new();
    first_available_registry.register(Arc::new(TestChannel::new("cli")), Some("tenant-a"));
    first_available_registry.register(Arc::new(TestChannel::new("cli")), Some("tenant-c"));
    let (_, first_available) =
        first_available_registry.find_channel_with_lookup("cli", Some("tenant-b"));
    assert_eq!(
        first_available.reason,
        ChannelLookupReason::FirstAvailableFallback
    );
    assert_eq!(
        first_available.matched_account_id.as_deref(),
        Some("tenant-a")
    );
}

#[test]
fn session_store_finds_existing_session_by_routing_key_before_creating_new_one() {
    let store = SessionStore::new();
    let session = store.create_session(
        "cli",
        anima_runtime::channel::SessionCreateOptions {
            id: Some("session-1".into()),
            routing_key: Some("anima.session.session-1".into()),
            ..Default::default()
        },
    );

    let found = store.find_or_create_session(
        "cli",
        anima_runtime::channel::FindSessionOptions {
            routing_key: Some(session.routing_key.clone()),
            ..Default::default()
        },
    );

    assert_eq!(found.id, session.id);
    assert_eq!(store.session_count(), 1);
}

#[test]
fn context_snapshot_restore_preserves_session_history_key_shape() {
    let context = ContextManager::new(Some(true));
    context.add_to_session_history("session-42", json!({"role": "user", "content": "hi"}));
    let key = "session:session-42:history";

    let snapshot = context.snapshot(key).expect("snapshot should exist");
    context.set_context(key, json!([]));
    let restored = context
        .restore(&snapshot.id)
        .expect("restore should succeed");

    assert_eq!(restored.key, key);
    assert_eq!(context.get_session_history("session-42").len(), 1);
}

#[test]
fn metrics_collector_uses_anima_prefix_and_registers_agent_metrics() {
    let metrics = MetricsCollector::new(None);
    metrics.register_agent_metrics();
    metrics.counter_inc("messages_received");
    metrics.update_session_gauge(2);

    let snapshot = metrics.snapshot();
    assert_eq!(metrics.prefix(), "anima");
    assert_eq!(snapshot.counters.get("messages_received"), Some(&1));
    assert_eq!(snapshot.gauges.get("sessions_active"), Some(&2));
    assert!(snapshot.histograms.contains_key("message_latency"));
}

#[test]
fn dispatcher_uses_round_robin_priority_path_through_balancer() {
    let registry = Arc::new(ChannelRegistry::new());
    registry.register(Arc::new(TestChannel::new("cli")), Some("worker-a"));
    registry.register(Arc::new(TestChannel::new("cli")), Some("worker-b"));

    let dispatcher = Dispatcher::new(registry, None);
    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        ..Default::default()
    }));
    balancer.add_target(Target::new("worker-a"));
    balancer.add_target(Target::new("worker-b"));
    dispatcher.add_balancer("cli", balancer);

    let first = dispatcher.dispatch(&anima_runtime::dispatcher::DispatchMessage {
        id: "m1".into(),
        channel: "cli".into(),
        account_id: "default".into(),
        chat_id: Some("chat-1".into()),
        content: "one".into(),
        media: vec![],
        stage: "final".into(),
        reply_target: Some("target-1".into()),
        sender_id: Some("sender-1".into()),
        priority: 1,
        routing_key: Some("anima.session.chat-1".into()),
        session_key: Some("anima.session.chat-1".into()),
        metadata: json!({}),
        created_at: 1,
    });
    let first_diag = dispatcher.last_dispatch_diagnostic().unwrap();

    let second = dispatcher.dispatch(&anima_runtime::dispatcher::DispatchMessage {
        id: "m2".into(),
        channel: "cli".into(),
        account_id: "default".into(),
        chat_id: Some("chat-2".into()),
        content: "two".into(),
        media: vec![],
        stage: "final".into(),
        reply_target: Some("target-2".into()),
        sender_id: Some("sender-2".into()),
        priority: 9,
        routing_key: Some("anima.session.chat-2".into()),
        session_key: Some("anima.session.chat-2".into()),
        metadata: json!({}),
        created_at: 2,
    });
    let second_diag = dispatcher.last_dispatch_diagnostic().unwrap();

    assert!(first.success);
    assert!(second.success);
    assert_eq!(first_diag.selected_target_id.as_deref(), Some("worker-a"));
    assert_eq!(second_diag.selected_target_id.as_deref(), Some("worker-b"));
}

#[test]
fn balancer_reports_all_circuits_open_when_every_healthy_target_is_open() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: Some(CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_ms: 60_000,
            }),
            health: Some(HealthPolicy {
                heartbeat_ttl_ms: 60_000,
                check_on_select: true,
            }),
        }),
        ..Default::default()
    });
    balancer.add_target(Target::new("worker-a"));
    balancer.record_target_failure("worker-a");

    let selected = balancer.select_target(Some("session-1"));
    let diagnostics = balancer.diagnostics_snapshot();

    assert!(selected.is_none());
    assert_eq!(
        diagnostics
            .last_selection
            .and_then(|selection| selection.miss_reason),
        Some(BalancerMissReason::AllCircuitsOpen)
    );
}

#[test]
fn balancer_excludes_non_available_targets() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::LeastConnections,
        ..Default::default()
    });
    let mut target = Target::new("worker-a");
    target.status = TargetStatus::Busy;
    balancer.add_target(target);

    let selected = balancer.select_target(Some("session-1"));
    let diagnostics = balancer.diagnostics_snapshot();

    assert!(selected.is_none());
    assert_eq!(
        diagnostics
            .last_selection
            .and_then(|selection| selection.miss_reason),
        Some(BalancerMissReason::NoAvailableTargets)
    );
}
