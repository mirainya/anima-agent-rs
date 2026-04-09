use anima_runtime::channel::session::*;
use serde_json::json;

// ── Create & Get ────────────────────────────────────────────────────

#[test]
fn session_store_create_and_get() {
    let store = SessionStore::new();
    let session = store.create_session("cli", SessionCreateOptions::default());
    assert!(!session.id.is_empty());
    assert_eq!(session.channel, "cli");

    let retrieved = store.get_session(&session.id).unwrap();
    assert_eq!(retrieved.id, session.id);
}

#[test]
fn session_store_get_returns_none_for_missing() {
    let store = SessionStore::new();
    assert!(store.get_session("nonexistent").is_none());
}

#[test]
fn session_store_create_with_custom_id() {
    let store = SessionStore::new();
    let session = store.create_session(
        "cli",
        SessionCreateOptions {
            id: Some("custom-id".into()),
            ..Default::default()
        },
    );
    assert_eq!(session.id, "custom-id");
    assert!(store.get_session("custom-id").is_some());
}

// ── Routing key lookup ──────────────────────────────────────────────

#[test]
fn session_store_lookup_by_routing_key() {
    let store = SessionStore::new();
    let session = store.create_session(
        "cli",
        SessionCreateOptions {
            routing_key: Some("opencode.session.abc123".into()),
            ..Default::default()
        },
    );
    let found = store
        .get_session_by_routing_key("opencode.session.abc123")
        .unwrap();
    assert_eq!(found.id, session.id);
}

#[test]
fn session_store_routing_key_returns_none_for_missing() {
    let store = SessionStore::new();
    assert!(store.get_session_by_routing_key("nonexistent").is_none());
}

// ── Update ──────────────────────────────────────────────────────────

#[test]
fn session_store_update_context() {
    let store = SessionStore::new();
    let session = store.create_session("cli", SessionCreateOptions::default());
    store.update_session_context(&session.id, json!({"key": "value"}));
    let updated = store.get_session(&session.id).unwrap();
    assert_eq!(updated.context["key"], "value");
}

#[test]
fn session_store_set_context_replaces() {
    let store = SessionStore::new();
    let session = store.create_session(
        "cli",
        SessionCreateOptions {
            context: Some(json!({"old": true})),
            ..Default::default()
        },
    );
    store.set_session_context(&session.id, json!({"new": true}));
    let updated = store.get_session(&session.id).unwrap();
    assert!(updated.context.get("old").is_none());
    assert_eq!(updated.context["new"], true);
}

// ── Delete ──────────────────────────────────────────────────────────

#[test]
fn session_store_close_session() {
    let store = SessionStore::new();
    let session = store.create_session("cli", SessionCreateOptions::default());
    assert!(store.close_session(&session.id).is_some());
    assert!(store.get_session(&session.id).is_none());
    assert!(store.close_session(&session.id).is_none()); // already closed
}

// ── List & Stats ────────────────────────────────────────────────────

#[test]
fn session_store_list_sessions() {
    let store = SessionStore::new();
    store.create_session("cli", SessionCreateOptions::default());
    store.create_session("rabbitmq", SessionCreateOptions::default());
    store.create_session("cli", SessionCreateOptions::default());

    let sessions = store.get_all_sessions();
    assert_eq!(sessions.len(), 3);
}

#[test]
fn session_store_stats() {
    let store = SessionStore::new();
    store.create_session("cli", SessionCreateOptions::default());
    store.create_session("cli", SessionCreateOptions::default());
    store.create_session("rabbitmq", SessionCreateOptions::default());

    let stats = store.get_stats();
    assert_eq!(stats.total, 3);
    assert_eq!(stats.by_channel["cli"], 2);
    assert_eq!(stats.by_channel["rabbitmq"], 1);
}

// ── Find or Create ──────────────────────────────────────────────────

#[test]
fn session_store_find_or_create_creates_new() {
    let store = SessionStore::new();
    let session = store.find_or_create_session("cli", FindSessionOptions::default());
    assert_eq!(session.channel, "cli");
    assert_eq!(store.get_all_sessions().len(), 1);
}

#[test]
fn session_store_find_or_create_finds_existing_by_id() {
    let store = SessionStore::new();
    let original = store.create_session("cli", SessionCreateOptions::default());
    let found = store.find_or_create_session(
        "cli",
        FindSessionOptions {
            session_id: Some(original.id.clone()),
            ..Default::default()
        },
    );
    assert_eq!(found.id, original.id);
    assert_eq!(store.get_all_sessions().len(), 1); // no new session created
}
