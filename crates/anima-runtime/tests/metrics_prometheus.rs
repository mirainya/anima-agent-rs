use anima_runtime::metrics::MetricsCollector;

#[test]
fn metrics_collector_renders_prometheus_text_format() {
    let metrics = MetricsCollector::new(Some("anima"));
    metrics.register_agent_metrics();
    metrics.counter_inc("messages_received");
    metrics.counter_add("tasks_completed", 2);
    metrics.update_session_gauge(3);
    metrics.histogram_record("message_latency", 120);

    let output = metrics.render_prometheus();
    assert!(output.contains("# TYPE anima_messages_received counter"));
    assert!(output.contains("anima_messages_received 1"));
    assert!(output.contains("# TYPE anima_sessions_active gauge"));
    assert!(output.contains("anima_sessions_active 3"));
    assert!(output.contains("# TYPE anima_message_latency histogram"));
    assert!(output.contains("anima_message_latency_bucket{le=\"250\"}"));
    assert!(output.contains("anima_message_latency_sum 120"));
    assert!(output.contains("anima_message_latency_count 1"));
}

#[test]
fn summary_record_and_get() {
    let metrics = MetricsCollector::new(Some("anima"));
    metrics.register_summary("response_time", Some(100));
    metrics.summary_record("response_time", 1.5);
    metrics.summary_record("response_time", 2.5);
    metrics.summary_record("response_time", 3.0);

    let summary = metrics.summary_get("response_time").unwrap();
    assert_eq!(summary.count, 3);
    assert!((summary.sum - 7.0).abs() < f64::EPSILON);
    assert_eq!(summary.values.len(), 3);
    assert_eq!(summary.max_size, 100);
}

#[test]
fn summary_get_returns_none_for_missing() {
    let metrics = MetricsCollector::new(None);
    assert!(metrics.summary_get("nonexistent").is_none());
}

#[test]
fn summary_renders_in_prometheus_format() {
    let metrics = MetricsCollector::new(Some("app"));
    metrics.summary_record("latency", 10.0);
    metrics.summary_record("latency", 20.0);

    let output = metrics.render_prometheus();
    assert!(output.contains("# TYPE app_latency summary"));
    assert!(output.contains("app_latency_sum 30"));
    assert!(output.contains("app_latency_count 2"));
}

#[test]
fn gauge_fn_evaluates_on_snapshot() {
    let metrics = MetricsCollector::new(Some("anima"));
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicI64::new(42));
    let counter_clone = counter.clone();
    metrics.register_gauge_fn(
        "dynamic_gauge",
        Box::new(move || counter_clone.load(Ordering::SeqCst)),
    );

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.gauges.get("dynamic_gauge"), Some(&42));

    // Change the value
    counter.store(100, Ordering::SeqCst);
    let snapshot2 = metrics.snapshot();
    assert_eq!(snapshot2.gauges.get("dynamic_gauge"), Some(&100));
}
