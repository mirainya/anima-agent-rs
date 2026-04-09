use anima_runtime::channel::{Channel, ChannelAdapter, RabbitMqChannel, RabbitMqChannelConfig};

#[test]
fn rabbitmq_channel_exposes_capabilities_and_default_health() {
    let channel = RabbitMqChannel::new(RabbitMqChannelConfig::default());
    let capabilities = channel.capabilities();
    let health = channel.health_snapshot();

    assert!(capabilities.outbound);
    assert!(capabilities.inbound);
    assert!(!capabilities.media);
    assert!(capabilities.health_check);
    assert_eq!(health.status, "stopped");
    assert!(!health.running);
}

#[test]
fn rabbitmq_channel_rejects_send_when_not_running() {
    let channel = RabbitMqChannel::new(RabbitMqChannelConfig::default());
    let result = channel.send_message("target", "hello", Default::default());

    assert!(!result.success);
    assert_eq!(
        result.error.as_deref(),
        Some("RabbitMQ channel is not running")
    );
}

#[test]
fn rabbitmq_channel_running_skeleton_returns_unimplemented_error() {
    let channel = RabbitMqChannel::new(RabbitMqChannelConfig::default());
    channel.start();
    let result = channel.send_message("target", "hello", Default::default());
    let health = channel.health_snapshot();

    assert!(!result.success);
    assert!(result
        .error
        .as_deref()
        .unwrap()
        .contains("RabbitMQ adapter skeleton"));
    assert_eq!(health.status, "unimplemented");
    assert!(health.running);
}
