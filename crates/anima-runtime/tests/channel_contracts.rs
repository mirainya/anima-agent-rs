use anima_runtime::bus::{make_outbound, MakeOutbound};
use anima_runtime::channel::{
    dispatch_outbound_message, ChannelLookupReason, ChannelRegistry, DispatchStats,
    FindSessionOptions, SessionCreateOptions, SessionStore, TestChannel,
};
use anima_runtime::Channel;
use serde_json::json;
use std::sync::Arc;

#[test]
fn session_store_set_context_overwrites_but_update_context_merges() {
    let store = SessionStore::new();
    let session = store.create_session(
        "cli",
        SessionCreateOptions {
            context: Some(json!({"a": 1, "b": 2})),
            ..Default::default()
        },
    );

    let merged = store
        .update_session_context(&session.id, json!({"b": 3, "c": 4}))
        .unwrap();
    assert_eq!(merged.context, json!({"a": 1, "b": 3, "c": 4}));

    let replaced = store
        .set_session_context(&session.id, json!({"only": true}))
        .unwrap();
    assert_eq!(replaced.context, json!({"only": true}));
}

#[test]
fn session_store_find_or_create_prefers_explicit_session_id_over_routing_key() {
    let store = SessionStore::new();
    let session_by_id = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("session-id".into()),
            routing_key: Some("anima.session.other".into()),
            ..Default::default()
        },
    );
    let _session_by_routing = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("different-id".into()),
            routing_key: Some("anima.session.route-key".into()),
            ..Default::default()
        },
    );

    let found = store.find_or_create_session(
        "cli",
        FindSessionOptions {
            session_id: Some("session-id".into()),
            routing_key: Some("anima.session.route-key".into()),
            ..Default::default()
        },
    );

    assert_eq!(found.id, session_by_id.id);
}

#[test]
fn registry_lookup_snapshot_reports_channel_missing() {
    let registry = ChannelRegistry::new();
    let (_, snapshot) = registry.find_channel_with_lookup("missing", Some("default"));

    assert_eq!(snapshot.reason, ChannelLookupReason::ChannelMissing);
    assert_eq!(snapshot.requested_channel, "missing");
    assert_eq!(snapshot.requested_account_id.as_deref(), Some("default"));
    assert_eq!(snapshot.matched_account_id, None);
}

#[test]
fn dispatch_outbound_uses_sender_id_when_reply_target_is_absent() {
    let registry = ChannelRegistry::new();
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), None);
    let stats = DispatchStats::new();

    let outbound = make_outbound(MakeOutbound {
        channel: "cli".into(),
        content: "hello".into(),
        sender_id: Some("sender-fallback".into()),
        stage: Some("final".into()),
        ..Default::default()
    });

    let result = dispatch_outbound_message(&outbound, &registry, &stats);
    assert!(result.success);
    let sent = channel.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, "sender-fallback");
}

#[test]
fn dispatch_stats_reset_clears_all_counters() {
    let registry = ChannelRegistry::new();
    let stats = DispatchStats::new();
    let outbound = make_outbound(MakeOutbound {
        channel: "missing".into(),
        content: "hello".into(),
        ..Default::default()
    });

    let _ = dispatch_outbound_message(&outbound, &registry, &stats);
    assert_eq!(stats.snapshot().channel_not_found, 1);

    stats.reset();
    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 0);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.channel_not_found, 0);
}
