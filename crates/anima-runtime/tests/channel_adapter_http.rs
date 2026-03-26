use anima_runtime::channel::{
    Channel, ChannelAdapter, ChannelRegistry, HttpChannel, HttpChannelConfig,
};

#[test]
fn http_channel_exposes_capabilities_and_default_health_snapshot() {
    let channel = HttpChannel::new(HttpChannelConfig::default());

    let capabilities = channel.capabilities();
    assert!(capabilities.outbound);
    assert!(!capabilities.inbound);
    assert!(capabilities.media);
    assert!(capabilities.health_check);

    let health = channel.health_snapshot();
    assert_eq!(health.status, "stopped");
    assert!(!health.running);
}

#[test]
fn http_channel_rejects_send_when_not_running() {
    let channel = HttpChannel::new(HttpChannelConfig::default());
    let result = channel.send_message("target", "hello", Default::default());

    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("HTTP channel is not running"));
}

#[test]
fn registry_entries_include_registered_account_ids() {
    let registry = ChannelRegistry::new();
    let default_http = std::sync::Arc::new(HttpChannel::new(HttpChannelConfig::default()));
    let secondary_http = std::sync::Arc::new(HttpChannel::new(HttpChannelConfig {
        name: "http".into(),
        endpoint: "http://127.0.0.1:9999/messages".into(),
        timeout_ms: 1000,
        auth_header: None,
    }));

    registry.register(default_http, Some("default"));
    registry.register(secondary_http, Some("secondary"));

    let entries = registry.entries();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry.channel == "http" && entry.account_id == "default"));
    assert!(entries.iter().any(|entry| entry.channel == "http" && entry.account_id == "secondary"));
}
