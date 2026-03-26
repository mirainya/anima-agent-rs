use anima_runtime::cache::ttl::*;
use serde_json::json;

// ── Basic get/set ───────────────────────────────────────────────────

#[test]
fn ttl_cache_set_and_get() {
    let cache = TtlCache::new(None, None);
    cache.set("key1", json!("hello"), None);
    assert_eq!(cache.get("key1"), Some(json!("hello")));
}

#[test]
fn ttl_cache_returns_none_for_missing() {
    let cache = TtlCache::new(None, None);
    assert_eq!(cache.get("nonexistent"), None);
}

#[test]
fn ttl_cache_delete() {
    let cache = TtlCache::new(None, None);
    cache.set("key1", json!(42), None);
    assert!(cache.delete("key1"));
    assert_eq!(cache.get("key1"), None);
    assert!(!cache.delete("key1")); // already deleted
}

#[test]
fn ttl_cache_has_key() {
    let cache = TtlCache::new(None, None);
    assert!(!cache.has("key1"));
    cache.set("key1", json!(1), None);
    assert!(cache.has("key1"));
}

// ── TTL expiration ──────────────────────────────────────────────────

#[test]
fn ttl_cache_entry_expires() {
    let cache = TtlCache::new(Some(1), None); // 1ms TTL
    cache.set("key1", json!("ephemeral"), None);
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(cache.get("key1"), None);
}

#[test]
fn ttl_cache_per_entry_ttl_override() {
    let cache = TtlCache::new(Some(100_000), None); // long default
    cache.set("short", json!("gone soon"), Some(1)); // 1ms override
    cache.set("long", json!("stays"), None);
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(cache.get("short"), None);
    assert_eq!(cache.get("long"), Some(json!("stays")));
}

#[test]
fn ttl_cache_remaining_ttl() {
    let cache = TtlCache::new(Some(10_000), None);
    cache.set("key1", json!(1), None);
    let remaining = cache.remaining_ttl("key1").unwrap();
    assert!(remaining > 0 && remaining <= 10_000);
}

// ── Eviction ────────────────────────────────────────────────────────

#[test]
fn ttl_cache_evicts_when_full() {
    let cache = TtlCache::new(None, Some(3)); // max 3 entries
    cache.set("a", json!(1), None);
    cache.set("b", json!(2), None);
    cache.set("c", json!(3), None);
    cache.set("d", json!(4), None); // should evict oldest

    assert_eq!(cache.len(), 3);
    assert!(cache.has("d"));
}

#[test]
fn ttl_cache_evicts_expired_first() {
    let cache = TtlCache::new(None, Some(2));
    cache.set("expired", json!(1), Some(1)); // 1ms TTL
    cache.set("valid", json!(2), Some(100_000));
    std::thread::sleep(std::time::Duration::from_millis(5));
    cache.set("new", json!(3), None); // should evict expired, not valid

    assert!(cache.has("valid"));
    assert!(cache.has("new"));
}

// ── Cleanup ─────────────────────────────────────────────────────────

#[test]
fn ttl_cache_cleanup_expired() {
    let cache = TtlCache::new(Some(1), None);
    cache.set("a", json!(1), None);
    cache.set("b", json!(2), None);
    cache.set("c", json!(3), None);
    std::thread::sleep(std::time::Duration::from_millis(5));

    let removed = cache.cleanup_expired();
    assert_eq!(removed, 3);
    assert_eq!(cache.len(), 0);
}

// ── Get or compute ──────────────────────────────────────────────────

#[test]
fn ttl_cache_get_or_compute() {
    let cache = TtlCache::new(None, None);
    let mut computed = false;
    let value = cache.get_or_compute("key1", || {
        computed = true;
        json!("computed")
    });
    assert!(computed);
    assert_eq!(value, json!("computed"));

    // Second call should use cache
    let mut computed2 = false;
    let value2 = cache.get_or_compute("key1", || {
        computed2 = true;
        json!("recomputed")
    });
    assert!(!computed2);
    assert_eq!(value2, json!("computed"));
}

#[test]
fn ttl_cache_get_or_compute_with_ttl() {
    let cache = TtlCache::new(None, None);
    let value = cache.get_or_compute_with_ttl("key1", 1, || json!("short-lived"));
    assert_eq!(value, json!("short-lived"));

    std::thread::sleep(std::time::Duration::from_millis(5));
    // Should recompute after expiry
    let value2 = cache.get_or_compute_with_ttl("key1", 100_000, || json!("refreshed"));
    assert_eq!(value2, json!("refreshed"));
}

// ── Stats ───────────────────────────────────────────────────────────

#[test]
fn ttl_cache_stats() {
    let cache = TtlCache::new(None, None);
    cache.set("a", json!(1), None);
    cache.set("b", json!(2), None);
    cache.get("a"); // hit
    cache.get("missing"); // miss

    let stats = cache.stats();
    assert_eq!(stats.writes, 2);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.entry_count, 2);
    assert!(stats.hit_rate() > 0.4 && stats.hit_rate() < 0.6);
}

// ── Clear ───────────────────────────────────────────────────────────

#[test]
fn ttl_cache_clear() {
    let cache = TtlCache::new(None, None);
    cache.set("a", json!(1), None);
    cache.set("b", json!(2), None);
    cache.clear();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}
