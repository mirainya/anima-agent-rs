use anima_runtime::cache::lru::*;
use anima_runtime::channel::adapter::*;
use anima_runtime::context::utils::*;

// ── Task A: Cache key helpers ──────────────────────────────────────

#[test]
fn make_task_cache_key_has_correct_format() {
    let key = make_task_cache_key("classify", "some payload");
    assert!(key.starts_with("task:classify:"));
    // Same input → same key
    assert_eq!(key, make_task_cache_key("classify", "some payload"));
    // Different input → different key
    assert_ne!(key, make_task_cache_key("classify", "other payload"));
}

#[test]
fn make_session_cache_key_has_correct_format() {
    let key = make_session_cache_key("sess-123", "history");
    assert!(key.starts_with("session:sess-123:"));
    assert_eq!(key, make_session_cache_key("sess-123", "history"));
    assert_ne!(key, make_session_cache_key("sess-123", "config"));
}

#[test]
fn make_api_cache_key_still_works() {
    let key = make_api_cache_key("sess-1", "hello");
    assert!(key.starts_with("api:sess-1:"));
}

// ── Task B: Context utilities ──────────────────────────────────────

#[test]
fn key_prefix_extracts_first_segment() {
    assert_eq!(key_prefix("session:abc:data"), Some("session"));
    assert_eq!(key_prefix("task:123"), Some("task"));
    assert_eq!(key_prefix("noprefix"), None);
    assert_eq!(key_prefix(""), None);
}

#[test]
fn key_parts_splits_by_colon() {
    assert_eq!(key_parts("a:b:c"), vec!["a", "b", "c"]);
    assert_eq!(key_parts("single"), vec!["single"]);
    assert_eq!(key_parts(""), vec![""]);
}

#[test]
fn matches_pattern_supports_wildcards() {
    assert!(matches_pattern("session:abc:data", "session:*"));
    assert!(matches_pattern("session:abc:data", "session:*:data"));
    assert!(matches_pattern("task:123", "task:???"));
    assert!(!matches_pattern("task:1234", "task:???"));
    assert!(matches_pattern("anything", "*"));
    assert!(!matches_pattern("abc", "ab"));
}

#[test]
fn context_type_maps_prefixes_correctly() {
    assert_eq!(context_type("session:abc"), ContextType::Session);
    assert_eq!(context_type("task:123"), ContextType::Task);
    assert_eq!(context_type("agent:x"), ContextType::Agent);
    assert_eq!(context_type("cache:key"), ContextType::Cache);
    assert_eq!(context_type("unknown:key"), ContextType::Unknown);
    assert_eq!(context_type("noprefix"), ContextType::Unknown);
}

// ── Task C: EvictionPolicy ─────────────────────────────────────────

#[test]
fn lru_eviction_policy_tracks_access_order() {
    let mut policy = LruEvictionPolicy::new();
    policy.on_set("a");
    policy.on_set("b");
    policy.on_set("c");
    assert_eq!(policy.select_for_eviction(), Some("a".to_string()));

    policy.on_access("a");
    assert_eq!(policy.select_for_eviction(), Some("b".to_string()));

    policy.on_delete("b");
    assert_eq!(policy.select_for_eviction(), Some("c".to_string()));
}

#[test]
fn lru_eviction_policy_empty_returns_none() {
    let policy = LruEvictionPolicy::new();
    assert_eq!(policy.select_for_eviction(), None);
}

// ── Task D: StreamingChannel ───────────────────────────────────────

#[test]
fn test_channel_implements_streaming_channel() {
    let ch = TestChannel::new("test");
    ch.start();

    let result = ch.send_chunk("user1", "chunk data");
    assert!(result.success);

    let result = ch.start_typing("user1");
    assert!(result.success);

    let result = ch.stop_typing("user1");
    assert!(result.success);

    // send_chunk should delegate to send_message
    let msgs = ch.sent_messages();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].message, "chunk data");
}

#[test]
fn failing_test_channel_streaming_also_fails() {
    let ch = TestChannel::failing("fail-ch");
    let result = ch.send_chunk("user1", "data");
    assert!(!result.success);
}
