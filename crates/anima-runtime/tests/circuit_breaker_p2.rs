use anima_runtime::dispatcher::circuit_breaker::*;

// ── State machine basics ────────────────────────────────────────────

#[test]
fn circuit_breaker_starts_closed() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
    assert!(cb.is_closed());
    assert!(cb.allows_request());
}

#[test]
fn circuit_opens_after_failure_threshold() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 3,
        ..Default::default()
    });

    // 3 failures should trip the circuit
    for _ in 0..3 {
        let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    }
    assert!(cb.is_open());
    assert!(!cb.allows_request());
}

#[test]
fn open_circuit_rejects_requests() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        ..Default::default()
    });

    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    assert!(cb.is_open());

    match cb.execute(|| Ok::<_, String>("should not run")) {
        CircuitBreakerResult::Rejected { retry_after_ms } => {
            assert!(retry_after_ms.is_some());
        }
        _ => panic!("expected Rejected"),
    }
}

#[test]
fn success_resets_failure_count() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 3,
        ..Default::default()
    });

    // 2 failures, then a success
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    let _ = cb.execute(|| Ok::<_, String>(()));

    // Should still be closed (success reset the count)
    assert!(cb.is_closed());

    // Need 3 more failures to trip
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    assert!(cb.is_closed());
}

// ── Half-open behavior ──────────────────────────────────────────────

#[test]
fn circuit_transitions_to_half_open_after_timeout() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        timeout_ms: 0, // Immediate timeout for testing
        ..Default::default()
    });

    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    assert!(cb.is_open());

    // Next execute should check timeout and transition to half-open
    std::thread::sleep(std::time::Duration::from_millis(1));
    match cb.execute(|| Ok::<_, String>("recovered")) {
        CircuitBreakerResult::Ok(v) => assert_eq!(v, "recovered"),
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn half_open_closes_after_success_threshold() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        success_threshold: 2,
        timeout_ms: 0,
        half_open_max_calls: 5,
    });

    // Trip the circuit
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    assert!(cb.is_open());

    std::thread::sleep(std::time::Duration::from_millis(1));

    // Two successes should close it
    let _ = cb.execute(|| Ok::<_, String>(()));
    let _ = cb.execute(|| Ok::<_, String>(()));
    assert!(cb.is_closed());
}

#[test]
fn half_open_reopens_on_failure() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        timeout_ms: 0,
        ..Default::default()
    });

    // Trip
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    std::thread::sleep(std::time::Duration::from_millis(1));

    // Fail in half-open → back to open
    let _ = cb.execute(|| Err::<(), _>("still failing".to_string()));
    assert!(cb.is_open());
}

// ── Manual control ──────────────────────────────────────────────────

#[test]
fn manual_trip_opens_circuit() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
    assert!(cb.is_closed());
    cb.trip();
    assert!(cb.is_open());
}

#[test]
fn manual_reset_closes_circuit() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        ..Default::default()
    });
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    assert!(cb.is_open());
    cb.reset();
    assert!(cb.is_closed());
}

// ── Stats tracking ──────────────────────────────────────────────────

#[test]
fn stats_track_successes_and_failures() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 10,
        ..Default::default()
    });

    let _ = cb.execute(|| Ok::<_, String>(()));
    let _ = cb.execute(|| Ok::<_, String>(()));
    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));

    let stats = cb.stats();
    assert_eq!(stats.total_successes, 2);
    assert_eq!(stats.total_failures, 1);
}

#[test]
fn stats_track_rejections() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig {
        failure_threshold: 1,
        ..Default::default()
    });

    let _ = cb.execute(|| Err::<(), _>("fail".to_string()));
    let _ = cb.execute(|| Ok::<_, String>(())); // rejected
    let _ = cb.execute(|| Ok::<_, String>(())); // rejected

    let stats = cb.stats();
    assert_eq!(stats.total_rejections, 2);
}

// ── Status report ───────────────────────────────────────────────────

#[test]
fn status_returns_full_info() {
    let cb = CircuitBreaker::new(CircuitBreakerConfig::default())
        .with_id("test-cb");
    let status = cb.status();
    assert_eq!(status.id, "test-cb");
    assert_eq!(status.state, CircuitState::Closed);
    assert_eq!(status.config.failure_threshold, 5);
}
