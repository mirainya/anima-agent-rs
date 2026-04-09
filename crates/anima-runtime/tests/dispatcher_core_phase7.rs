use anima_runtime::bus::{make_outbound, Bus, MakeOutbound, OutboundMessage};
use anima_runtime::channel::{
    dispatch_outbound_message, ChannelLookupReason, ChannelRegistry, DispatchStats, TestChannel,
};
use anima_runtime::dispatcher::{
    make_target, start_dispatcher_outbound_loop, Balancer, BalancerMissReason, BalancerOptions,
    BalancerRuntimeConfig, BalancerStrategy, CircuitBreakerConfig, CircuitState,
    DispatchFailureStage, DispatchMessage, DispatchOutcomeReason, Dispatcher, DispatcherConfig,
    DispatcherState, HealthPolicy, Target, TargetOptions,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

fn outbound_message() -> OutboundMessage {
    make_outbound(MakeOutbound {
        channel: "test".into(),
        account_id: Some("default".into()),
        chat_id: Some("chat-1".into()),
        content: "hello dispatcher".into(),
        media: Some(vec!["file-1".into()]),
        stage: Some("final".into()),
        reply_target: Some("user-1".into()),
        sender_id: Some("sender-1".into()),
    })
}

#[test]
fn dispatch_message_from_outbound_preserves_bridge_fields() {
    let outbound = outbound_message();
    let dispatch = DispatchMessage::from_outbound(&outbound);

    assert_eq!(dispatch.id, outbound.id);
    assert_eq!(dispatch.channel, "test");
    assert_eq!(dispatch.account_id, "default");
    assert_eq!(dispatch.chat_id.as_deref(), Some("chat-1"));
    assert_eq!(dispatch.content, "hello dispatcher");
    assert_eq!(dispatch.media, vec!["file-1"]);
    assert_eq!(dispatch.stage, "final");
    assert_eq!(dispatch.reply_target.as_deref(), Some("user-1"));
    assert_eq!(dispatch.sender_id.as_deref(), Some("sender-1"));
    assert_eq!(dispatch.priority, 5);
    assert_eq!(
        dispatch.routing_key.as_deref(),
        Some("anima.session.chat-1")
    );
    assert_eq!(
        dispatch.session_key.as_deref(),
        Some("anima.session.chat-1")
    );
    assert_eq!(dispatch.target(), "user-1");
    assert_eq!(dispatch.metadata, json!({}));
    assert!(dispatch.created_at > 0);
}

#[test]
fn dispatcher_defaults_expose_empty_status_and_stats() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry, None);

    assert_eq!(dispatcher.config(), &DispatcherConfig::default());

    let status = dispatcher.status();
    assert_eq!(status.state, DispatcherState::Stopped);
    assert_eq!(status.last_dispatch_at, None);
    assert_eq!(status.last_error_at, None);
    assert_eq!(status.queue_depth, 0);
    assert!(status.routes.is_empty());

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.dispatched, 0);
    assert_eq!(stats.errors, 0);
    assert_eq!(stats.channel_not_found, 0);
    assert_eq!(stats.balancer_selected, 0);
    assert_eq!(stats.balancer_misses, 0);
    assert!(stats.balancer_miss_reasons.is_empty());
    assert!(stats.channel_lookup_failures.is_empty());
    assert!(stats.target_stats.is_empty());
    assert!(dispatcher.last_dispatch_diagnostic().is_none());
}

#[test]
fn dispatcher_passthrough_matches_legacy_dispatch_behavior() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);
    let dispatcher_channel = Arc::new(TestChannel::new("test"));
    let legacy_channel = Arc::new(TestChannel::new("legacy"));

    registry.register(dispatcher_channel.clone(), None);
    registry.register(legacy_channel.clone(), None);

    let dispatch_msg = DispatchMessage::from_outbound(&outbound_message());
    let dispatch_result = dispatcher.dispatch(&dispatch_msg);
    assert!(dispatch_result.success);

    let legacy_msg = make_outbound(MakeOutbound {
        channel: "legacy".into(),
        account_id: Some("default".into()),
        chat_id: Some("chat-1".into()),
        content: "hello dispatcher".into(),
        media: Some(vec!["file-1".into()]),
        stage: Some("final".into()),
        reply_target: Some("user-1".into()),
        sender_id: Some("sender-1".into()),
    });
    let legacy_stats = DispatchStats::new();
    let legacy_result = dispatch_outbound_message(&legacy_msg, &registry, &legacy_stats);
    assert!(legacy_result.success);

    let dispatcher_sent = dispatcher_channel.sent_messages();
    let legacy_sent = legacy_channel.sent_messages();
    assert_eq!(dispatcher_sent.len(), 1);
    assert_eq!(legacy_sent.len(), 1);
    assert_eq!(dispatcher_sent[0].target, legacy_sent[0].target);
    assert_eq!(dispatcher_sent[0].message, legacy_sent[0].message);
    assert_eq!(dispatcher_sent[0].opts, legacy_sent[0].opts);

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.dispatched, 1);
    assert_eq!(stats.errors, 0);
    assert_eq!(stats.channel_not_found, 0);

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(diagnostic.outcome, DispatchOutcomeReason::SelectedAndSent);
    assert_eq!(
        diagnostic.channel_lookup_reason,
        Some(ChannelLookupReason::ExactMatch)
    );
}

#[test]
fn dispatcher_balancer_bridges_target_id_to_account_lookup() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);
    let default_channel = Arc::new(TestChannel::new("test"));
    let prod_channel = Arc::new(TestChannel::new("test"));

    registry.register(default_channel.clone(), None);
    registry.register(prod_channel.clone(), Some("prod"));

    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::Hashing,
        ..Default::default()
    }));
    balancer.add_target(Target::new("prod"));
    dispatcher.add_balancer("test", balancer.clone());

    let mut msg = DispatchMessage::from_outbound(&outbound_message());
    msg.session_key = Some("session-42".into());
    let result = dispatcher.dispatch(&msg);
    assert!(result.success);

    assert!(default_channel.sent_messages().is_empty());
    let sent = prod_channel.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, "user-1");

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.dispatched, 1);
    assert_eq!(stats.balancer_selected, 1);
    assert_eq!(stats.balancer_misses, 0);
    assert_eq!(stats.target_stats["prod"].selected, 1);
    assert_eq!(stats.target_stats["prod"].success, 1);

    let status = dispatcher.status();
    assert_eq!(status.state, DispatcherState::Running);
    assert!(status.routes.contains_key("test"));
    assert_eq!(status.routes["test"].target_count, 1);
    assert_eq!(status.routes["test"].available_target_count, 1);
    assert_eq!(status.routes["test"].healthy_target_count, 1);
    assert_eq!(status.routes["test"].open_circuit_count, 0);
    assert_eq!(status.routes["test"].half_open_target_count, 0);
    assert_eq!(status.routes["test"].last_miss_reason, None);

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(diagnostic.selected_target_id.as_deref(), Some("prod"));
    assert_eq!(
        diagnostic.channel_lookup_reason,
        Some(ChannelLookupReason::ExactMatch)
    );
}

#[test]
fn dispatcher_records_not_found_send_failure_and_balancer_miss() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);

    let missing = DispatchMessage::from_outbound(&outbound_message());
    let missing_result = dispatcher.dispatch(&missing);
    assert!(!missing_result.success);
    assert_eq!(missing_result.error.as_deref(), Some("Channel not found"));
    let missing_diag = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(
        missing_diag.failure_stage,
        Some(DispatchFailureStage::ChannelLookup)
    );
    assert_eq!(missing_diag.outcome, DispatchOutcomeReason::ChannelNotFound);
    assert_eq!(
        missing_diag.channel_lookup_reason,
        Some(ChannelLookupReason::ChannelMissing)
    );

    let failing_channel = Arc::new(TestChannel::failing("test"));
    registry.register(failing_channel.clone(), None);
    let failing = DispatchMessage::from_outbound(&outbound_message());
    let failing_result = dispatcher.dispatch(&failing);
    assert!(!failing_result.success);
    assert_eq!(failing_result.error.as_deref(), Some("Mock send failed"));
    let failing_diag = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(failing_diag.failure_stage, Some(DispatchFailureStage::Send));
    assert_eq!(failing_diag.outcome, DispatchOutcomeReason::SendFailed);
    assert_eq!(failing_diag.send_error.as_deref(), Some("Mock send failed"));

    let miss_balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::Hashing,
        ..Default::default()
    }));
    miss_balancer.add_target(Target::new("default"));
    dispatcher.add_balancer("test", miss_balancer);
    let mut miss = DispatchMessage::from_outbound(&outbound_message());
    miss.reply_target = None;
    miss.chat_id = None;
    miss.session_key = None;
    miss.sender_id = None;
    let miss_result = dispatcher.dispatch(&miss);
    assert!(!miss_result.success);

    let miss_diag = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(miss_diag.failure_stage, Some(DispatchFailureStage::Send));
    assert_eq!(miss_diag.outcome, DispatchOutcomeReason::SendFailed);
    assert_eq!(
        miss_diag.balancer_miss_reason,
        Some(BalancerMissReason::HashingKeyMissing)
    );
    assert_eq!(miss_diag.send_error.as_deref(), Some("Mock send failed"));

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.channel_not_found, 1);
    assert_eq!(stats.errors, 2);
    assert_eq!(stats.dispatched, 0);
    assert_eq!(stats.balancer_misses, 1);
    assert_eq!(
        stats.balancer_miss_reasons[&BalancerMissReason::HashingKeyMissing],
        1
    );
    assert_eq!(
        stats.channel_lookup_failures[&ChannelLookupReason::ChannelMissing],
        1
    );
    assert!(dispatcher.status().last_error_at.is_some());
}

#[test]
fn dispatcher_outbound_loop_consumes_bus_and_dispatches_messages() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Arc::new(Dispatcher::new(registry.clone(), None));
    let channel = Arc::new(TestChannel::new("test"));
    registry.register(channel.clone(), None);

    let handle = start_dispatcher_outbound_loop(bus.clone(), dispatcher.clone());
    bus.publish_outbound(outbound_message()).unwrap();

    for _ in 0..20 {
        if !channel.sent_messages().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    bus.close();
    handle.join().unwrap();

    let sent = channel.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, "hello dispatcher");

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.dispatched, 1);
}

#[test]
fn dispatcher_records_selected_target_failure_when_account_is_missing() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry, None);
    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        ..Default::default()
    }));
    balancer.add_target(make_target(
        Some("prod".into()),
        TargetOptions {
            metadata: Some(json!({"route": "prod"})),
            ..Default::default()
        },
    ));
    dispatcher.add_balancer("test", balancer);

    let msg = DispatchMessage::from_outbound(&outbound_message());
    let result = dispatcher.dispatch(&msg);
    assert!(!result.success);

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.channel_not_found, 1);
    assert_eq!(stats.balancer_selected, 1);
    assert_eq!(stats.target_stats["prod"].selected, 1);
    assert_eq!(stats.target_stats["prod"].failures, 1);
    assert!(stats.target_stats["prod"].last_failure_at.is_some());

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(
        diagnostic.failure_stage,
        Some(DispatchFailureStage::ChannelLookup)
    );
    assert_eq!(diagnostic.selected_target_id.as_deref(), Some("prod"));
}

#[test]
fn dispatcher_send_failure_opens_breaker_and_switches_to_healthy_target() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);
    let failing_prod = Arc::new(TestChannel::failing("test"));
    let healthy_backup = Arc::new(TestChannel::new("test"));
    registry.register(failing_prod, Some("prod"));
    registry.register(healthy_backup.clone(), Some("backup"));

    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: Some(CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_ms: 1_000,
            }),
            health: Some(HealthPolicy {
                heartbeat_ttl_ms: 1_000,
                check_on_select: true,
            }),
        }),
        ..Default::default()
    }));
    balancer.add_target(Target::new("prod"));
    balancer.add_target(Target::new("backup"));
    balancer.record_target_heartbeat("prod", true);
    balancer.record_target_heartbeat("backup", true);
    dispatcher.add_balancer("test", balancer.clone());

    let first = DispatchMessage::from_outbound(&outbound_message());
    let first_result = dispatcher.dispatch(&first);
    assert!(!first_result.success);
    assert_eq!(
        balancer.target_health("prod").unwrap().circuit_state,
        CircuitState::Open
    );
    let first_diag = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(first_diag.failure_stage, Some(DispatchFailureStage::Send));
    assert_eq!(first_diag.selected_target_id.as_deref(), Some("prod"));

    let mut second = DispatchMessage::from_outbound(&outbound_message());
    second.session_key = Some("session-2".into());
    let second_result = dispatcher.dispatch(&second);
    assert!(second_result.success);
    assert_eq!(healthy_backup.sent_messages().len(), 1);

    let status = dispatcher.status();
    assert_eq!(status.routes["test"].open_circuit_count, 1);
    assert_eq!(status.routes["test"].healthy_target_count, 2);
}

#[test]
fn dispatcher_channel_not_found_does_not_open_breaker() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry, None);
    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: Some(CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_ms: 1_000,
            }),
            health: Some(HealthPolicy {
                heartbeat_ttl_ms: 1_000,
                check_on_select: true,
            }),
        }),
        ..Default::default()
    }));
    balancer.add_target(Target::new("prod"));
    balancer.record_target_heartbeat("prod", true);
    dispatcher.add_balancer("test", balancer.clone());

    let result = dispatcher.dispatch(&DispatchMessage::from_outbound(&outbound_message()));
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("Channel not found"));
    assert_eq!(
        balancer.target_health("prod").unwrap().circuit_state,
        CircuitState::Closed
    );

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(
        diagnostic.failure_stage,
        Some(DispatchFailureStage::ChannelLookup)
    );
}

#[test]
fn dispatcher_returns_balancer_miss_when_all_targets_are_unhealthy() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry, None);
    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: None,
            health: Some(HealthPolicy {
                heartbeat_ttl_ms: 1_000,
                check_on_select: true,
            }),
        }),
        ..Default::default()
    }));
    balancer.add_target(Target::new("prod"));
    balancer.record_target_heartbeat("prod", false);
    dispatcher.add_balancer("test", balancer.clone());

    let result = dispatcher.dispatch(&DispatchMessage::from_outbound(&outbound_message()));
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("Channel not found"));

    let stats = dispatcher.stats().snapshot();
    assert_eq!(stats.balancer_misses, 1);
    assert_eq!(stats.balancer_selected, 0);
    assert_eq!(
        stats.balancer_miss_reasons[&BalancerMissReason::NoHealthyTargets],
        1
    );

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(
        diagnostic.balancer_miss_reason,
        Some(BalancerMissReason::NoHealthyTargets)
    );
    assert_eq!(
        balancer
            .diagnostics_snapshot()
            .last_selection
            .unwrap()
            .miss_reason,
        Some(BalancerMissReason::NoHealthyTargets)
    );

    let status = dispatcher.status();
    assert_eq!(
        status.routes["test"].last_miss_reason,
        Some(BalancerMissReason::NoHealthyTargets)
    );
}

#[test]
fn dispatcher_channel_lookup_uses_default_fallback_reason() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);
    let default_channel = Arc::new(TestChannel::new("test"));
    registry.register(default_channel.clone(), None);

    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        ..Default::default()
    }));
    balancer.add_target(Target::new("prod"));
    dispatcher.add_balancer("test", balancer);

    let result = dispatcher.dispatch(&DispatchMessage::from_outbound(&outbound_message()));
    assert!(result.success);

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(
        diagnostic.channel_lookup_reason,
        Some(ChannelLookupReason::DefaultFallback)
    );
}
