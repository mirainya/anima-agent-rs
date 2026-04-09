use anima_runtime::dispatcher::{
    constant_hashing_index, make_target, round_robin_index, Balancer, BalancerMetrics,
    BalancerMissReason, BalancerOptions, BalancerRuntimeConfig, BalancerStrategy,
    CircuitBreakerConfig, CircuitState, HealthPolicy, LoadUpdate, SelectionReason, Target,
    TargetExcludeReason, TargetOptions, TargetStatus,
};
use serde_json::json;

#[test]
fn router_round_robin_rotates_indices() {
    let mut last = None;
    let mut observed = Vec::new();

    for _ in 0..6 {
        let next = round_robin_index(last, 3).unwrap();
        observed.push(next);
        last = Some(next);
    }

    assert_eq!(observed, vec![0, 1, 2, 0, 1, 2]);
}

#[test]
fn router_constant_hashing_is_stable_for_same_key() {
    let first = constant_hashing_index("session-123", 5).unwrap();
    let second = constant_hashing_index("session-123", 5).unwrap();
    assert_eq!(first, second);
}

#[test]
fn router_handles_empty_and_singleton_inputs() {
    assert_eq!(round_robin_index(None, 0), None);
    assert_eq!(constant_hashing_index("session-123", 0), None);
    assert_eq!(round_robin_index(None, 1), Some(0));
    assert_eq!(constant_hashing_index("session-123", 1), Some(0));
}

#[test]
fn balancer_manages_targets_in_insertion_order() {
    let balancer = Balancer::default();
    let alpha = Target::new("alpha");
    let beta = make_target(
        Some("beta".into()),
        TargetOptions {
            weight: Some(2),
            capacity: Some(20),
            metadata: Some(json!({"zone": "b"})),
            ..Default::default()
        },
    );

    balancer.add_target(alpha.clone());
    balancer.add_target(beta.clone());

    assert_eq!(balancer.get_target("alpha"), Some(alpha));
    assert_eq!(balancer.get_target("beta"), Some(beta.clone()));
    assert_eq!(
        balancer
            .list_targets()
            .into_iter()
            .map(|target| target.id)
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );

    let removed = balancer.remove_target("alpha").unwrap();
    assert_eq!(removed.id, "alpha");
    assert!(balancer.get_target("alpha").is_none());
    assert_eq!(balancer.list_targets().len(), 1);
}

#[test]
fn balancer_tracks_load_and_status_transitions() {
    let balancer = Balancer::default();
    balancer.add_target(Target::new("alpha"));

    let initial = balancer.get_load("alpha").unwrap();
    assert_eq!(initial.active, 0);
    assert_eq!(initial.total, 0);
    assert_eq!(initial.errors, 0);
    assert_eq!(initial.last_error_at, None);

    let after_inc = balancer.update_load("alpha", LoadUpdate::Inc).unwrap();
    assert_eq!(after_inc.active, 1);
    assert_eq!(after_inc.total, 1);

    let after_dec = balancer.update_load("alpha", LoadUpdate::Dec).unwrap();
    assert_eq!(after_dec.active, 0);
    assert_eq!(after_dec.total, 1);

    balancer.update_load("alpha", LoadUpdate::Inc).unwrap();
    let after_error = balancer.update_load("alpha", LoadUpdate::Error).unwrap();
    assert_eq!(after_error.active, 0);
    assert_eq!(after_error.total, 2);
    assert_eq!(after_error.errors, 1);
    assert!(after_error.last_error_at.is_some());

    let after_reset = balancer.update_load("alpha", LoadUpdate::Reset).unwrap();
    assert_eq!(after_reset.active, 0);
    assert_eq!(balancer.get_active_count("alpha"), Some(0));

    assert_eq!(
        balancer.set_target_status("alpha", TargetStatus::Busy),
        Some(TargetStatus::Busy)
    );
    assert!(balancer.list_available_targets().is_empty());
    assert_eq!(
        balancer.mark_target_offline("alpha"),
        Some(TargetStatus::Offline)
    );
    assert_eq!(
        balancer.mark_target_available("alpha"),
        Some(TargetStatus::Available)
    );
    assert_eq!(balancer.list_available_targets().len(), 1);
}

#[test]
fn balancer_round_robin_selection_is_deterministic() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));
    balancer.add_target(Target::new("gamma"));

    let selected = (0..5)
        .map(|_| balancer.select_target(None).unwrap().id)
        .collect::<Vec<_>>();

    assert_eq!(selected, vec!["alpha", "beta", "gamma", "alpha", "beta"]);

    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert_eq!(last.selected_reason, Some(SelectionReason::RoundRobin));
    assert_eq!(last.candidate_ids, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn balancer_weighted_selection_uses_deterministic_expansion_order() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::Weighted,
        ..Default::default()
    });
    balancer.add_target(make_target(
        Some("alpha".into()),
        TargetOptions {
            weight: Some(2),
            ..Default::default()
        },
    ));
    balancer.add_target(make_target(
        Some("beta".into()),
        TargetOptions {
            weight: Some(1),
            ..Default::default()
        },
    ));

    let selected = (0..6)
        .map(|_| balancer.select_target(None).unwrap().id)
        .collect::<Vec<_>>();

    assert_eq!(
        selected,
        vec!["alpha", "alpha", "beta", "alpha", "alpha", "beta"]
    );
    assert_eq!(
        balancer
            .diagnostics_snapshot()
            .last_selection
            .unwrap()
            .selected_reason,
        Some(SelectionReason::WeightedRoundRobin)
    );
}

#[test]
fn balancer_least_connections_breaks_ties_by_insertion_order() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::LeastConnections,
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));

    assert_eq!(balancer.select_target(None).unwrap().id, "alpha");

    balancer.update_load("alpha", LoadUpdate::Inc).unwrap();
    assert_eq!(balancer.select_target(None).unwrap().id, "beta");
    assert_eq!(
        balancer
            .diagnostics_snapshot()
            .last_selection
            .unwrap()
            .selected_reason,
        Some(SelectionReason::LeastConnections)
    );
}

#[test]
fn balancer_hashing_is_stable_for_same_target_set() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::Hashing,
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));
    balancer.add_target(Target::new("gamma"));

    let first = balancer.select_target(Some("session-7")).unwrap().id;
    let second = balancer.select_target(Some("session-7")).unwrap().id;
    assert_eq!(first, second);
    assert!(matches!(first.as_str(), "alpha" | "beta" | "gamma"));

    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert_eq!(last.selected_reason, Some(SelectionReason::Hashing));
    assert_eq!(last.selection_key.as_deref(), Some("session-7"));
}

#[test]
fn balancer_status_and_metrics_expose_read_only_snapshots() {
    let balancer = Balancer::new(BalancerOptions {
        id: Some("balancer-1".into()),
        strategy: BalancerStrategy::RoundRobin,
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));
    balancer.update_load("alpha", LoadUpdate::Inc).unwrap();
    balancer.update_load("alpha", LoadUpdate::Error).unwrap();
    balancer.mark_target_busy("alpha").unwrap();
    balancer.mark_target_offline("beta").unwrap();
    assert!(balancer.select_target(None).is_none());

    let status = balancer.balancer_status();
    assert_eq!(status.id, "balancer-1");
    assert_eq!(status.strategy, BalancerStrategy::RoundRobin);
    assert_eq!(status.target_count, 2);
    assert_eq!(status.available_count, 0);
    assert_eq!(status.healthy_count, 2);
    assert_eq!(status.open_circuit_count, 0);
    assert_eq!(status.half_open_count, 0);
    assert_eq!(status.targets["alpha"].status, TargetStatus::Busy);
    assert_eq!(status.targets["alpha"].load.errors, 1);
    assert_eq!(status.targets["alpha"].circuit_state, CircuitState::Closed);
    assert_eq!(status.targets["beta"].status, TargetStatus::Offline);
    assert_eq!(status.stats.selections, 0);
    assert_eq!(status.stats.failures, 1);

    let metrics = balancer.balancer_metrics();
    assert_eq!(
        metrics,
        BalancerMetrics {
            total_targets: 2,
            available_targets: 0,
            busy_targets: 1,
            offline_targets: 1,
            healthy_targets: 2,
            unhealthy_targets: 0,
            open_circuits: 0,
            half_open_circuits: 0,
            total_active: 0,
            total_errors: 1,
        }
    );

    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert_eq!(
        last.miss_reason,
        Some(BalancerMissReason::NoAvailableTargets)
    );
    assert_eq!(
        diagnostics.exclude_reason_counts[&TargetExcludeReason::StatusBusy],
        1
    );
    assert_eq!(
        diagnostics.exclude_reason_counts[&TargetExcludeReason::StatusOffline],
        1
    );
}

#[test]
fn balancer_circuit_breaker_transitions_closed_open_half_open_closed() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: Some(CircuitBreakerConfig {
                failure_threshold: 2,
                success_threshold: 2,
                cooldown_ms: 100,
            }),
            health: None,
        }),
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));

    balancer.record_target_failure("alpha");
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::Closed
    );

    balancer.record_target_failure("alpha");
    let opened = balancer.target_health("alpha").unwrap();
    assert_eq!(opened.circuit_state, CircuitState::Open);
    assert!(!balancer.is_target_routable("alpha", opened.circuit_opened_at.unwrap()));

    balancer.refresh_target_health("alpha", opened.circuit_opened_at.unwrap() + 50);
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::Open
    );

    balancer.refresh_target_health("alpha", opened.circuit_opened_at.unwrap() + 100);
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::HalfOpen
    );

    balancer.record_target_success("alpha");
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::HalfOpen
    );

    balancer.record_target_success("alpha");
    let closed = balancer.target_health("alpha").unwrap();
    assert_eq!(closed.circuit_state, CircuitState::Closed);
    assert!(balancer.is_target_routable("alpha", closed.last_checked_at.unwrap()));
}

#[test]
fn balancer_half_open_failure_reopens_circuit() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: Some(CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_ms: 10,
            }),
            health: None,
        }),
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));

    balancer.record_target_failure("alpha");
    let opened = balancer.target_health("alpha").unwrap();
    balancer.refresh_target_health("alpha", opened.circuit_opened_at.unwrap() + 10);
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::HalfOpen
    );

    balancer.record_target_failure("alpha");
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::Open
    );
}

#[test]
fn balancer_heartbeat_and_ttl_control_routability() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: None,
            health: Some(HealthPolicy {
                heartbeat_ttl_ms: 1,
                check_on_select: true,
            }),
        }),
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));

    let heartbeat = balancer.record_target_heartbeat("alpha", true).unwrap();
    assert_eq!(heartbeat.last_heartbeat_at, heartbeat.last_checked_at);
    assert!(balancer.is_target_routable("alpha", heartbeat.last_heartbeat_at.unwrap()));

    balancer.record_target_heartbeat("alpha", false);
    let unhealthy = balancer.target_health("alpha").unwrap();
    assert!(!unhealthy.healthy);
    assert!(!balancer.is_target_routable("alpha", unhealthy.last_checked_at.unwrap()));

    balancer.record_target_heartbeat("alpha", true);
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(balancer.select_target(None).is_none());

    let expired = balancer.target_health("alpha").unwrap();
    assert!(!expired.healthy);

    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert_eq!(last.miss_reason, Some(BalancerMissReason::NoHealthyTargets));
    assert!(last.target_diagnostics["alpha"]
        .excluded_reasons
        .contains(&TargetExcludeReason::HeartbeatExpired));
}

#[test]
fn balancer_prefers_closed_healthy_targets_over_half_open_targets() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        runtime: Some(BalancerRuntimeConfig {
            circuit_breaker: Some(CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_ms: 10,
            }),
            health: Some(HealthPolicy {
                heartbeat_ttl_ms: 1_000,
                check_on_select: true,
            }),
        }),
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));

    let alpha_beat = balancer.record_target_heartbeat("alpha", true).unwrap();
    balancer.record_target_heartbeat("beta", true);
    balancer.record_target_failure("beta");
    let beta_opened = balancer
        .target_health("beta")
        .unwrap()
        .circuit_opened_at
        .unwrap();
    balancer.refresh_target_health("beta", beta_opened + 10);
    assert_eq!(
        balancer.target_health("beta").unwrap().circuit_state,
        CircuitState::HalfOpen
    );

    let selected = balancer.select_target(None).unwrap();
    assert_eq!(selected.id, "alpha");
    assert!(balancer.is_target_routable("alpha", alpha_beat.last_checked_at.unwrap()));

    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert!(last.target_diagnostics["beta"]
        .excluded_reasons
        .contains(&TargetExcludeReason::PreferClosedOverHalfOpen));
}

#[test]
fn balancer_records_hashing_key_miss_diagnostics() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::Hashing,
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));

    assert!(balancer.select_target(None).is_none());
    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert_eq!(
        last.miss_reason,
        Some(BalancerMissReason::HashingKeyMissing)
    );
    assert!(last.target_diagnostics["alpha"]
        .excluded_reasons
        .contains(&TargetExcludeReason::HashingKeyMissing));
    assert_eq!(
        diagnostics.miss_reason_counts[&BalancerMissReason::HashingKeyMissing],
        1
    );
}

#[test]
fn balancer_records_all_circuits_open_diagnostics() {
    let balancer = Balancer::new(BalancerOptions {
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
    });
    balancer.add_target(Target::new("alpha"));
    balancer.record_target_heartbeat("alpha", true);
    balancer.record_target_failure("alpha");

    assert!(balancer.select_target(None).is_none());
    let diagnostics = balancer.diagnostics_snapshot();
    let last = diagnostics.last_selection.unwrap();
    assert_eq!(last.miss_reason, Some(BalancerMissReason::AllCircuitsOpen));
    assert!(last.target_diagnostics["alpha"]
        .excluded_reasons
        .contains(&TargetExcludeReason::CircuitOpen));
}

#[test]
fn balancer_without_runtime_config_preserves_phase7_behavior() {
    let balancer = Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::RoundRobin,
        ..Default::default()
    });
    balancer.add_target(Target::new("alpha"));
    balancer.add_target(Target::new("beta"));

    balancer.record_target_failure("alpha");
    balancer.record_target_heartbeat("alpha", false);

    let selected = (0..4)
        .map(|_| balancer.select_target(None).unwrap().id)
        .collect::<Vec<_>>();
    assert_eq!(selected, vec!["alpha", "beta", "alpha", "beta"]);
    assert_eq!(
        balancer.target_health("alpha").unwrap().circuit_state,
        CircuitState::Closed
    );
    assert!(balancer.target_health("alpha").unwrap().healthy);
}
