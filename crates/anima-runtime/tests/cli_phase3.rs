use anima_runtime::bus::Bus;
use anima_runtime::channel::Channel;
use anima_runtime::cli::{is_quit_command, CliChannel, DEFAULT_PROMPT};
use std::sync::Arc;

#[test]
fn create_cli_channel_with_defaults() {
    let cli = CliChannel::new(None, None, None);
    assert_eq!(cli.channel_name(), "cli");
    assert!(!cli.is_running());
    assert!(cli.bus().is_none());
    assert_eq!(cli.prompt(), DEFAULT_PROMPT);
}

#[test]
fn create_cli_channel_with_bus_and_prompt() {
    let bus = Arc::new(Bus::create());
    let cli = CliChannel::new(None, Some(bus.clone()), Some("custom> "));
    assert!(cli.bus().is_some());
    assert_eq!(cli.prompt(), "custom> ");
}

#[test]
fn cli_lifecycle_and_health_check() {
    let cli = CliChannel::new(None, None, None);
    assert!(!cli.health_check());
    cli.start();
    assert!(cli.is_running());
    assert!(cli.health_check());
    cli.stop();
    assert!(!cli.health_check());
}

#[test]
fn cli_send_message_and_chunk_like_behavior() {
    let cli = CliChannel::new(None, None, None);
    cli.start();
    let result = cli.send_message("target", "Hello!", Default::default());
    assert!(result.success);

    let result = cli.send_message(
        "target",
        "chunk text",
        anima_runtime::channel::SendOptions {
            media: vec![],
            stage: Some("chunk".into()),
        },
    );
    assert!(result.success);

    let result = cli.send_message(
        "target",
        "final text",
        anima_runtime::channel::SendOptions {
            media: vec![],
            stage: Some("final".into()),
        },
    );
    assert!(result.success);

    assert_eq!(
        cli.outputs(),
        vec![
            "Hello!".to_string(),
            DEFAULT_PROMPT.to_string(),
            "chunk text".to_string(),
            "final text".to_string(),
            DEFAULT_PROMPT.to_string(),
        ]
    );
}

#[test]
fn cli_empty_line_and_clear_without_session_match_baseline() {
    let cli = CliChannel::new(None, None, None);

    let prompt = cli.handle_line("   ").unwrap();
    assert_eq!(prompt, DEFAULT_PROMPT);

    cli.clear_history();
    assert!(cli.outputs().is_empty());

    let status = cli.handle_line("status").unwrap();
    assert_eq!(status, "No active session");

    let history = cli.handle_line("history").unwrap();
    assert_eq!(history, "No active session");
}

#[test]
fn quit_commands_match_baseline() {
    assert!(is_quit_command("exit"));
    assert!(is_quit_command("quit"));
    assert!(is_quit_command(":q"));
    assert!(is_quit_command("/quit"));
    assert!(is_quit_command("/exit"));
    assert!(is_quit_command("bye"));
    assert!(is_quit_command("  exit  "));
    assert!(!is_quit_command("hello"));
    assert!(!is_quit_command("exitnow"));
    assert!(!is_quit_command(""));
}

#[test]
fn cli_special_commands_match_baseline() {
    let cli = CliChannel::new(None, None, None);
    cli.start();

    let help = cli.handle_line("help").unwrap();
    assert!(help.contains("Available commands"));

    let status = cli.handle_line("status").unwrap();
    assert!(status.contains("Session Status:"));

    let history_before = cli.handle_line("history").unwrap();
    assert!(history_before.contains("No conversation history"));

    let cleared = cli.handle_line("clear").unwrap();
    assert_eq!(cleared, "Conversation history cleared.");

    let goodbye = cli.handle_line("exit").unwrap();
    assert_eq!(goodbye, "Goodbye!");
    assert!(!cli.is_running());
}

#[test]
fn cli_regular_message_publishes_to_bus_and_updates_history() {
    let bus = Arc::new(Bus::create());
    let cli = CliChannel::new(None, Some(bus.clone()), None);
    cli.start();
    let result = cli.handle_line("What is Clojure?");
    assert!(result.is_none());

    let session = cli.default_session().unwrap();
    let history_text = cli.handle_line("history").unwrap();
    assert!(history_text.contains("What is Clojure?"));

    let inbound = bus.consume_inbound().unwrap();
    assert_eq!(inbound.channel, "cli");
    assert_eq!(inbound.sender_id, "user");
    assert_eq!(inbound.chat_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(
        inbound.session_key.as_deref(),
        Some(session.routing_key.as_str())
    );
    assert_eq!(inbound.content, "What is Clojure?");
}
