use anima_runtime::support::{make_api_cache_key, ContextManager, LruCache, MetricsCollector};
use serde_json::json;

#[test]
fn context_manager_tracks_session_history_and_snapshots() {
    let manager = ContextManager::new(Some(true));
    manager.set_context("session:s1", json!({"channel": "cli"}));
    manager.add_to_session_history("s1", json!({"role": "user", "content": "hello"}));
    manager.add_to_session_history("s1", json!({"role": "assistant", "content": "hi"}));

    let snapshot = manager.snapshot("session:s1").unwrap();
    manager.set_context("session:s1", json!({"channel": "telegram"}));
    let restored = manager.restore(&snapshot.id).unwrap();

    assert_eq!(restored.key, "session:s1");
    assert_eq!(manager.get_context("session:s1").unwrap()["channel"], "cli");
    assert_eq!(manager.get_session_history("s1").len(), 2);
    assert_eq!(manager.status().status, "running");
}

#[test]
fn lru_cache_supports_get_or_compute_and_stats() {
    let cache = LruCache::new(Some(2), Some(60_000));
    let key = make_api_cache_key("session-1", "hello");

    let first = cache.get_or_compute(&key, || json!({"value": 1}));
    let second = cache.get_or_compute(&key, || json!({"value": 2}));
    let stats = cache.stats();

    assert_eq!(first["value"], 1);
    assert_eq!(second["value"], 1);
    assert_eq!(stats.entry_count, 1);
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
}

#[test]
fn metrics_collector_tracks_counters_gauges_and_histograms() {
    let metrics = MetricsCollector::new(Some("anima"));
    metrics.register_agent_metrics();
    metrics.counter_inc("messages_received");
    metrics.counter_add("tasks_completed", 2);
    metrics.update_session_gauge(3);
    metrics.update_worker_gauges(1, 2);
    metrics.histogram_record("message_latency", 120);

    let snapshot = metrics.snapshot();
    assert_eq!(metrics.prefix(), "anima");
    assert_eq!(snapshot.counters["messages_received"], 1);
    assert_eq!(snapshot.counters["tasks_completed"], 2);
    assert_eq!(snapshot.gauges["sessions_active"], 3);
    assert_eq!(snapshot.gauges["workers_active"], 1);
    assert_eq!(snapshot.gauges["workers_idle"], 2);
    assert_eq!(snapshot.histograms["message_latency"].count, 1);
}
