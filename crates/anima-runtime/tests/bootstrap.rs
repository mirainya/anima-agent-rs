use anima_runtime::bootstrap::RuntimeBootstrapBuilder;

#[test]
fn runtime_bootstrap_builder_constructs_and_starts_runtime() {
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_url("http://127.0.0.1:9711")
        .with_prompt("test> ")
        .build();

    runtime.start();

    let dispatcher_status = runtime.dispatcher.status();
    assert!(matches!(dispatcher_status.state, anima_runtime::dispatcher::DispatcherState::Running));
    assert!(runtime.cli_channel().is_some());
    assert!(runtime.agent.is_running());

    runtime.stop();
    assert!(!runtime.agent.is_running());
}

#[test]
fn runtime_bootstrap_builder_can_disable_cli_channel() {
    let runtime = RuntimeBootstrapBuilder::new().with_cli_enabled(false).build();
    assert!(runtime.cli_channel().is_none());
}
