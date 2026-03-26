use anima_sdk::runtime::{
    RuntimeFacadeDispatcherStatus, RuntimeFacadeRouteReport, RuntimeFacadeStatus,
};

#[test]
fn runtime_facade_status_models_are_constructible_and_stable() {
    let status = RuntimeFacadeStatus {
        dispatcher: RuntimeFacadeDispatcherStatus {
            state: "Running".into(),
            queue_depth: 2,
            route_count: 1,
        },
        route_reports: vec![RuntimeFacadeRouteReport {
            channel: "cli".into(),
            pending_queue_depth: 2,
            target_count: 1,
            healthy_target_count: 1,
            open_circuit_count: 0,
        }],
    };

    assert_eq!(status.dispatcher.state, "Running");
    assert_eq!(status.dispatcher.queue_depth, 2);
    assert_eq!(status.route_reports.len(), 1);
    assert_eq!(status.route_reports[0].channel, "cli");
}
