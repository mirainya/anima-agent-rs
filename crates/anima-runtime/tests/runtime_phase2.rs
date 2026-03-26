use anima_runtime::bus::{make_inbound, make_outbound, Bus, MakeInbound, MakeOutbound};
use anima_runtime::channel::{
    create_message, dispatch_outbound_message, extract_session_id, extract_user_id,
    make_channel_routing_key, make_session_routing_key, make_user_routing_key, Channel,
    ChannelRegistry, CreateMessage, DispatchStats, FindSessionOptions, SessionCreateOptions,
    SessionStore, TestChannel, BROADCAST_ROUTING_KEY,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn routing_key_helpers_match_baseline() {
    assert_eq!(
        extract_session_id("anima.session.abc123"),
        Some("abc123".into())
    );
    assert_eq!(extract_session_id("anima.user.alice"), None);
    assert_eq!(extract_user_id("anima.user.alice"), Some("alice".into()));
    assert_eq!(extract_user_id("anima.session.abc123"), None);
    assert_eq!(make_session_routing_key("abc123"), "anima.session.abc123");
    assert_eq!(make_user_routing_key("alice"), "anima.user.alice");
    assert_eq!(make_channel_routing_key("cli"), "anima.channel.cli");
    assert_eq!(BROADCAST_ROUTING_KEY, "anima.broadcast");
}

#[test]
fn create_message_matches_defaults() {
    let msg = create_message(CreateMessage {
        session_id: Some("test-session".into()),
        sender: Some("alice".into()),
        content: "Hello".into(),
        channel: Some("cli".into()),
        ..Default::default()
    });
    assert_eq!(msg.session_id, "test-session");
    assert_eq!(msg.sender, "alice");
    assert_eq!(msg.content, "Hello");
    assert_eq!(msg.channel, "cli");
    assert!(msg.timestamp > 0);
}

#[test]
fn session_store_crud_and_history_match_baseline() {
    let store = SessionStore::new();
    let session = store.create_session("cli", SessionCreateOptions::default());
    assert_eq!(store.session_count(), 1);
    assert!(store.session_exists(&session.id));
    store.update_session_context(&session.id, json!({"mode": "chatting", "user": "alice"}));
    let updated = store.get_session(&session.id).unwrap();
    assert_eq!(updated.context["mode"], "chatting");
    assert_eq!(updated.context["user"], "alice");

    store.add_to_history(&session.id, json!({"role": "user", "content": "Hello"}));
    store.add_to_history(
        &session.id,
        json!({"role": "assistant", "content": "Hi there!"}),
    );
    let history = store.get_history(&session.id);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[1]["content"], "Hi there!");

    let closed = store.close_session(&session.id);
    assert!(closed.is_some());
    assert_eq!(store.session_count(), 0);
}

#[test]
fn find_or_create_session_preserves_lookup_order() {
    let store = SessionStore::new();
    let existing = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("session-123".into()),
            routing_key: Some("anima.session.test".into()),
            ..Default::default()
        },
    );

    let by_id = store.find_or_create_session(
        "cli",
        FindSessionOptions {
            session_id: Some("session-123".into()),
            ..Default::default()
        },
    );
    assert_eq!(by_id.id, existing.id);

    let by_routing = store.find_or_create_session(
        "cli",
        FindSessionOptions {
            routing_key: Some("anima.session.test".into()),
            ..Default::default()
        },
    );
    assert_eq!(by_routing.id, existing.id);

    let extracted = store.find_or_create_session(
        "cli",
        FindSessionOptions {
            routing_key: Some("anima.session.session-123".into()),
            ..Default::default()
        },
    );
    assert_eq!(extracted.id, existing.id);

    let created = store.find_or_create_session("web", FindSessionOptions::default());
    assert_eq!(created.channel, "web");
    assert_eq!(store.session_count(), 2);
}

#[test]
fn session_store_reports_stats_counts_and_touch_updates_activity() {
    let store = SessionStore::new();
    let cli_default = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("cli-default".into()),
            account_id: Some("default".into()),
            ..Default::default()
        },
    );
    let _cli_prod = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("cli-prod".into()),
            account_id: Some("production".into()),
            ..Default::default()
        },
    );
    let web_default = store.create_session(
        "web",
        SessionCreateOptions {
            id: Some("web-default".into()),
            ..Default::default()
        },
    );

    let before_touch = store.get_session(&cli_default.id).unwrap().last_active;
    std::thread::sleep(std::time::Duration::from_millis(2));
    let touched = store.touch_session(&cli_default.id).unwrap();
    assert!(touched.last_active >= before_touch);

    assert_eq!(store.session_count_by_channel("cli", None), 2);
    assert_eq!(store.session_count_by_channel("cli", Some("default")), 1);
    assert_eq!(store.session_count_by_channel("cli", Some("production")), 1);
    assert_eq!(store.session_count_by_channel("web", Some("default")), 1);

    let cli_sessions = store.get_sessions_by_channel("cli", None);
    assert_eq!(cli_sessions.len(), 1);
    assert_eq!(cli_sessions[0].id, cli_default.id);

    let all_sessions = store.get_all_sessions();
    assert_eq!(all_sessions.len(), 3);
    assert!(all_sessions.iter().any(|s| s.id == web_default.id));

    let stats = store.get_stats();
    assert_eq!(stats.total, 3);
    assert_eq!(stats.by_channel["cli"], 2);
    assert_eq!(stats.by_channel["web"], 1);
    assert!(stats.active_last_hour >= 3);
}

#[test]
fn session_store_close_all_sessions_resets_state() {
    let store = SessionStore::new();
    store.create_session("cli", SessionCreateOptions::default());
    store.create_session(
        "cli",
        SessionCreateOptions {
            account_id: Some("production".into()),
            ..Default::default()
        },
    );
    store.create_session("web", SessionCreateOptions::default());

    assert_eq!(store.session_count(), 3);
    assert_eq!(store.session_count_by_channel("cli", None), 2);
    assert_eq!(store.session_count_by_channel("web", None), 1);

    let closed = store.close_all_sessions();
    assert_eq!(closed, 3);
    assert_eq!(store.session_count(), 0);
    assert!(store.get_all_sessions().is_empty());
    assert!(store.get_sessions_by_channel("cli", None).is_empty());
    assert!(store.get_sessions_by_channel("web", None).is_empty());

    let stats = store.get_stats();
    assert_eq!(stats.total, 0);
    assert!(stats.by_channel.is_empty());
    assert_eq!(stats.active_last_hour, 0);
}

#[test]
fn registry_supports_default_and_first_available_fallback() {
    let registry = ChannelRegistry::new();
    let dev = Arc::new(TestChannel::new("cli"));
    let prod = Arc::new(TestChannel::new("cli"));
    registry.register(dev.clone(), Some("development"));
    registry.register(prod.clone(), Some("production"));

    assert_eq!(registry.channel_count(), 2);
    assert!(registry.find_channel("cli", Some("development")).is_some());
    assert!(registry.find_channel("cli", Some("production")).is_some());
    assert!(registry.find_channel("cli", None).is_some());
}

#[test]
fn registry_registers_and_unregisters_channels_by_account() {
    let registry = ChannelRegistry::new();
    let default_channel = Arc::new(TestChannel::new("cli"));
    let dev_channel = Arc::new(TestChannel::new("cli"));
    let prod_channel = Arc::new(TestChannel::new("cli"));

    registry.register(default_channel.clone(), None);
    registry.register(dev_channel.clone(), Some("development"));
    registry.register(prod_channel.clone(), Some("production"));

    assert_eq!(registry.channel_count(), 3);
    assert!(registry.find_channel("cli", None).is_some());
    assert!(registry.find_channel("cli", Some("development")).is_some());
    assert!(registry.find_channel("cli", Some("production")).is_some());

    registry.unregister_with_account("cli", "development");
    assert_eq!(registry.channel_count(), 2);
    assert!(registry.find_channel("cli", Some("development")).is_some());
    assert!(registry.find_channel("cli", Some("production")).is_some());

    registry.unregister_with_account("cli", "production");
    assert_eq!(registry.channel_count(), 1);
    let fallback = registry.find_channel("cli", Some("production")).unwrap();
    assert_eq!(fallback.channel_name(), "cli");

    registry.unregister("cli");
    assert_eq!(registry.channel_count(), 0);
    assert!(registry.find_channel("cli", None).is_none());
}

#[test]
fn registry_reports_health_and_supports_bulk_lifecycle() {
    let registry = ChannelRegistry::new();
    let healthy = Arc::new(TestChannel::new("cli"));
    let failing = Arc::new(TestChannel::failing("worker"));

    registry.register(healthy.clone(), None);
    registry.register(failing.clone(), Some("production"));

    let initial = registry.health_report();
    assert_eq!(initial.total, 2);
    assert_eq!(initial.healthy, 0);
    assert_eq!(initial.unhealthy, 2);
    assert!(!initial.all_healthy);

    registry.start_all();
    assert!(healthy.is_running());
    assert!(failing.is_running());

    let after_start = registry.health_report();
    assert_eq!(after_start.total, 2);
    assert_eq!(after_start.healthy, 1);
    assert_eq!(after_start.unhealthy, 1);
    assert!(!after_start.all_healthy);

    registry.stop_all();
    assert!(!healthy.is_running());
    assert!(!failing.is_running());

    let after_stop = registry.health_report();
    assert_eq!(after_stop.total, 2);
    assert_eq!(after_stop.healthy, 0);
    assert_eq!(after_stop.unhealthy, 2);
    assert!(!after_stop.all_healthy);
}

#[test]
fn registry_lists_channel_names_and_all_channels_without_dup_names() {
    let registry = ChannelRegistry::new();
    let default_cli = Arc::new(TestChannel::new("cli"));
    let production_cli = Arc::new(TestChannel::new("cli"));
    let web = Arc::new(TestChannel::new("web"));

    registry.register(default_cli.clone(), None);
    registry.register(production_cli.clone(), Some("production"));
    registry.register(web.clone(), None);

    let names = registry.channel_names();
    assert_eq!(names, vec!["cli".to_string(), "web".to_string()]);

    let channels = registry.all_channels();
    assert_eq!(channels.len(), 3);
    assert_eq!(channels[0].channel_name(), "cli");
    assert_eq!(channels[1].channel_name(), "cli");
    assert_eq!(channels[2].channel_name(), "web");
}

#[test]
fn session_store_helper_queries_match_account_and_routing_baseline() {
    let store = SessionStore::new();
    let cli_default = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("cli-default".into()),
            routing_key: Some("anima.session.cli-default".into()),
            account_id: Some("default".into()),
            ..Default::default()
        },
    );
    let cli_prod = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("cli-prod".into()),
            routing_key: Some("anima.session.cli-prod".into()),
            account_id: Some("production".into()),
            ..Default::default()
        },
    );

    assert!(store.session_exists(&cli_default.id));
    assert!(!store.session_exists("missing"));

    let by_routing = store
        .get_session_by_routing_key("anima.session.cli-prod")
        .unwrap();
    assert_eq!(by_routing.id, cli_prod.id);

    let default_sessions = store.get_sessions_by_channel("cli", Some("default"));
    assert_eq!(default_sessions.len(), 1);
    assert_eq!(default_sessions[0].id, cli_default.id);

    let prod_sessions = store.get_sessions_by_channel("cli", Some("production"));
    assert_eq!(prod_sessions.len(), 1);
    assert_eq!(prod_sessions[0].id, cli_prod.id);

    assert!(store.get_sessions_by_channel("cli", Some("staging")).is_empty());
}

#[test]
fn dispatch_updates_stats_and_uses_reply_target() {
    let registry = ChannelRegistry::new();
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), None);
    let stats = DispatchStats::new();
    let msg = make_outbound(MakeOutbound {
        channel: "cli".into(),
        content: "Response from AI".into(),
        reply_target: Some("user1".into()),
        stage: Some("final".into()),
        ..Default::default()
    });

    let result = dispatch_outbound_message(&msg, &registry, &stats);
    assert!(result.success);
    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 0);
    let sent = channel.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, "user1");
    assert_eq!(sent[0].message, "Response from AI");
}

#[test]
fn dispatch_handles_missing_channel_and_send_failure() {
    let registry = ChannelRegistry::new();
    let stats = DispatchStats::new();
    let missing = make_outbound(MakeOutbound {
        channel: "missing".into(),
        content: "lost".into(),
        ..Default::default()
    });
    let result = dispatch_outbound_message(&missing, &registry, &stats);
    assert!(!result.success);
    assert_eq!(stats.snapshot().channel_not_found, 1);

    let failing_registry = ChannelRegistry::new();
    let failing = Arc::new(TestChannel::failing("cli"));
    failing_registry.register(failing, None);
    let msg = make_outbound(MakeOutbound {
        channel: "cli".into(),
        content: "fail".into(),
        ..Default::default()
    });
    let stats = DispatchStats::new();
    let result = dispatch_outbound_message(&msg, &failing_registry, &stats);
    assert!(!result.success);
    assert_eq!(stats.snapshot().errors, 1);
}

#[test]
fn bus_inbound_outbound_flow_matches_baseline() {
    let bus = Bus::create();
    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user1".into()),
        content: "Hello AI".into(),
        ..Default::default()
    });
    bus.publish_inbound(inbound).unwrap();
    let received = bus.consume_inbound().unwrap();
    assert_eq!(received.content, "Hello AI");

    let outbound = make_outbound(MakeOutbound {
        channel: received.channel,
        content: "Echo: Hello AI".into(),
        reply_target: Some(received.sender_id),
        stage: Some("final".into()),
        ..Default::default()
    });
    bus.publish_outbound(outbound).unwrap();
    let response = bus.consume_outbound().unwrap();
    assert_eq!(response.content, "Echo: Hello AI");
    assert_eq!(response.reply_target.as_deref(), Some("user1"));
    assert_eq!(response.stage, "final");
}

#[test]
fn outbound_dispatch_loop_tracks_channel_not_found_and_send_errors() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let ok_channel = Arc::new(TestChannel::new("ok"));
    let failing_channel = Arc::new(TestChannel::failing("bad"));
    ok_channel.start();
    failing_channel.start();

    registry.register(ok_channel.clone(), Some("ok-account"));
    registry.register(failing_channel.clone(), Some("bad-account"));

    let dispatch_handle = anima_runtime::channel::start_outbound_dispatch(
        bus.clone(),
        registry.clone(),
        stats.clone(),
    );

    bus.publish_outbound(make_outbound(MakeOutbound {
        channel: "ok".into(),
        account_id: Some("ok-account".into()),
        reply_target: Some("target-ok".into()),
        content: "success".into(),
        stage: Some("final".into()),
        ..Default::default()
    }))
    .unwrap();

    bus.publish_outbound(make_outbound(MakeOutbound {
        channel: "missing".into(),
        account_id: Some("missing-account".into()),
        reply_target: Some("target-missing".into()),
        content: "missing".into(),
        stage: Some("final".into()),
        ..Default::default()
    }))
    .unwrap();

    bus.publish_outbound(make_outbound(MakeOutbound {
        channel: "bad".into(),
        account_id: Some("bad-account".into()),
        reply_target: Some("target-bad".into()),
        content: "fail".into(),
        stage: Some("final".into()),
        ..Default::default()
    }))
    .unwrap();

    for _ in 0..20 {
        if ok_channel.sent_messages().len() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let sent = ok_channel.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, "target-ok");
    assert_eq!(sent[0].message, "success");
    assert_eq!(sent[0].opts.stage.as_deref(), Some("final"));

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.channel_not_found, 1);
    assert_eq!(snapshot.errors, 1);

    let report = registry.health_report();
    assert_eq!(report.total, 2);
    assert_eq!(report.healthy, 1);
    assert_eq!(report.unhealthy, 1);
    assert!(!report.all_healthy);

    bus.close();
    dispatch_handle.join().unwrap();
}
