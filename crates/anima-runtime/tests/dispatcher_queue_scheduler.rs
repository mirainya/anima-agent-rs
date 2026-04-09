use anima_runtime::bus::{make_outbound, Bus, MakeOutbound};
use anima_runtime::channel::{Channel, ChannelRegistry, TestChannel};
use anima_runtime::dispatcher::{
    apply_dispatcher_control, start_dispatch_scheduler, start_dispatcher_outbound_loop,
    DispatchEnvelope, DispatchMessage, DispatchQueue, Dispatcher, DispatcherControlCommand,
    DispatcherState,
};
use std::sync::Arc;
use std::time::Duration;

fn make_message(channel: &str, content: &str, priority: u8) -> DispatchMessage {
    let mut msg = DispatchMessage::from_outbound(&make_outbound(MakeOutbound {
        channel: channel.into(),
        account_id: Some("default".into()),
        chat_id: Some(format!("chat-{priority}")),
        content: content.into(),
        reply_target: Some(format!("target-{priority}")),
        stage: Some("final".into()),
        ..Default::default()
    }));
    msg.priority = priority;
    msg
}

#[test]
fn dispatch_queue_dequeues_higher_priority_first() {
    let queue = DispatchQueue::new();
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "cli", "low", 9,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "cli", "high", 1,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "cli", "mid", 5,
    )));

    let first = queue.dequeue().unwrap();
    let second = queue.dequeue().unwrap();
    let third = queue.dequeue().unwrap();

    assert_eq!(first.message.content, "high");
    assert_eq!(second.message.content, "mid");
    assert_eq!(third.message.content, "low");
    assert!(queue.is_empty());
}

#[test]
fn dispatch_queue_rotates_fairly_across_routes_with_same_priority() {
    let queue = DispatchQueue::new();
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "alpha", "a1", 1,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "beta", "b1", 1,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "alpha", "a2", 1,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "beta", "b2", 1,
    )));

    let first = queue.dequeue().unwrap();
    let second = queue.dequeue().unwrap();
    let third = queue.dequeue().unwrap();
    let fourth = queue.dequeue().unwrap();

    assert_eq!(first.message.channel, "alpha");
    assert_eq!(second.message.channel, "beta");
    assert_eq!(third.message.channel, "alpha");
    assert_eq!(fourth.message.channel, "beta");
}

#[test]
fn dispatch_queue_keeps_priority_precedence_across_routes() {
    let queue = DispatchQueue::new();
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "alpha", "low-a", 9,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "beta", "high-b", 1,
    )));
    queue.enqueue(DispatchEnvelope::from_message(make_message(
        "alpha", "high-a", 1,
    )));

    let first = queue.dequeue().unwrap();
    let second = queue.dequeue().unwrap();
    let third = queue.dequeue().unwrap();

    assert!(matches!(
        first.message.content.as_str(),
        "high-b" | "high-a"
    ));
    assert!(matches!(
        second.message.content.as_str(),
        "high-b" | "high-a"
    ));
    assert_ne!(first.message.content, second.message.content);
    assert_eq!(third.message.content, "low-a");
}

#[test]
fn dispatch_queue_snapshot_tracks_lengths_by_priority() {
    let queue = DispatchQueue::new();
    queue.enqueue(DispatchEnvelope::from_message(make_message("cli", "a", 5)));
    queue.enqueue(DispatchEnvelope::from_message(make_message("cli", "b", 5)));
    queue.enqueue(DispatchEnvelope::from_message(make_message("web", "c", 1)));

    let snapshot = queue.snapshot();
    assert_eq!(snapshot.total_len, 3);
    assert_eq!(snapshot.by_priority.get(&1), Some(&1));
    assert_eq!(snapshot.by_priority.get(&5), Some(&2));
    assert_eq!(snapshot.by_route.get("cli"), Some(&2));
    assert_eq!(snapshot.by_route.get("web"), Some(&1));
}

#[test]
fn dispatcher_enqueue_and_drain_one_use_queue_path() {
    let registry = Arc::new(ChannelRegistry::new());
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), Some("default"));
    let dispatcher = Dispatcher::new(registry, None);
    dispatcher.start();

    dispatcher.enqueue(make_message("cli", "queued", 3));
    assert_eq!(dispatcher.queue_snapshot().total_len, 1);
    assert_eq!(dispatcher.status().queue_depth, 1);
    assert_eq!(dispatcher.route_queue_depth("cli"), 1);

    let result = dispatcher.drain_one().unwrap();
    assert!(result.success);
    assert_eq!(dispatcher.queue_snapshot().total_len, 0);
    assert_eq!(dispatcher.status().queue_depth, 0);
    assert_eq!(dispatcher.route_queue_depth("cli"), 0);
    assert_eq!(channel.sent_messages().len(), 1);
    assert_eq!(channel.sent_messages()[0].message, "queued");
}

#[test]
fn dispatcher_outbound_loop_routes_messages_through_queue() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Arc::new(Dispatcher::new(registry.clone(), None));
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), Some("default"));

    let handle = start_dispatcher_outbound_loop(bus.clone(), dispatcher.clone());
    bus.publish_outbound(make_outbound(MakeOutbound {
        channel: "cli".into(),
        account_id: Some("default".into()),
        content: "through-bus".into(),
        reply_target: Some("bus-target".into()),
        stage: Some("final".into()),
        ..Default::default()
    }))
    .unwrap();

    for _ in 0..20 {
        if !channel.sent_messages().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    bus.close();
    handle.join().unwrap();

    assert_eq!(dispatcher.queue_snapshot().total_len, 0);
    assert_eq!(channel.sent_messages().len(), 1);
    assert_eq!(channel.sent_messages()[0].message, "through-bus");
}

#[test]
fn start_dispatch_scheduler_returns_running_loop_for_non_empty_queue() {
    let registry = Arc::new(ChannelRegistry::new());
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), Some("default"));
    let dispatcher = Arc::new(Dispatcher::new(registry, None));
    dispatcher.enqueue(make_message("cli", "scheduled", 2));

    let handle = start_dispatch_scheduler(dispatcher.clone());

    for _ in 0..20 {
        if dispatcher.queue_snapshot().total_len == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    dispatcher.stop();
    handle.join().unwrap();

    assert_eq!(dispatcher.queue_snapshot().total_len, 0);
    assert_eq!(channel.sent_messages().len(), 1);
    assert_eq!(channel.sent_messages()[0].message, "scheduled");
}

#[test]
fn dispatcher_lifecycle_pause_resume_and_stop_control_queue_drain() {
    let registry = Arc::new(ChannelRegistry::new());
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), Some("default"));
    let dispatcher = Arc::new(Dispatcher::new(registry, None));

    dispatcher.enqueue(make_message("cli", "paused", 1));
    dispatcher.pause();
    assert_eq!(dispatcher.status().state, DispatcherState::Paused);
    assert!(dispatcher.drain_one().is_none());
    assert_eq!(dispatcher.queue_snapshot().total_len, 1);

    dispatcher.resume();
    let resumed = dispatcher.drain_one();
    assert!(resumed.is_some());
    assert_eq!(dispatcher.queue_snapshot().total_len, 0);
    assert_eq!(channel.sent_messages().len(), 1);

    dispatcher.stop();
    assert_eq!(dispatcher.status().state, DispatcherState::Stopped);
}

#[test]
fn dispatcher_control_api_applies_commands_and_reports_state() {
    let registry = Arc::new(ChannelRegistry::new());
    let channel = Arc::new(TestChannel::new("cli"));
    channel.start();
    registry.register(channel.clone(), Some("default"));
    let dispatcher = Dispatcher::new(registry, None);

    dispatcher.enqueue(make_message("cli", "drain-control", 2));
    let paused = apply_dispatcher_control(&dispatcher, DispatcherControlCommand::Pause);
    assert_eq!(paused.state, DispatcherState::Paused);
    assert_eq!(paused.queue_depth, 1);

    let resumed = apply_dispatcher_control(&dispatcher, DispatcherControlCommand::Resume);
    assert_eq!(resumed.state, DispatcherState::Running);

    let drained = apply_dispatcher_control(&dispatcher, DispatcherControlCommand::Drain);
    assert_eq!(drained.state, DispatcherState::Stopped);
    assert_eq!(drained.queue_depth, 0);
    assert_eq!(channel.sent_messages().len(), 1);

    let started = apply_dispatcher_control(&dispatcher, DispatcherControlCommand::Start);
    assert_eq!(started.state, DispatcherState::Running);

    let stopped = apply_dispatcher_control(&dispatcher, DispatcherControlCommand::Stop);
    assert_eq!(stopped.state, DispatcherState::Stopped);
}

#[test]
fn dispatcher_retry_backoff_requeues_failed_message_until_due() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);
    dispatcher.start();

    let failing = Arc::new(TestChannel::failing("cli"));
    registry.register(failing, Some("default"));

    let envelope =
        DispatchEnvelope::from_message(make_message("cli", "retry-me", 1)).with_retry_policy(1, 50);
    dispatcher.enqueue_envelope(envelope);

    let first = dispatcher.drain_one().unwrap();
    assert!(!first.success);
    assert_eq!(dispatcher.queue_snapshot().total_len, 1);

    assert!(dispatcher.drain_one().is_none());

    std::thread::sleep(Duration::from_millis(60));
    let retry = dispatcher.drain_one().unwrap();
    assert!(!retry.success);
    assert_eq!(dispatcher.queue_snapshot().total_len, 0);
}

#[test]
fn dispatcher_retry_policy_only_retries_allowed_failure_stages() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry.clone(), None);
    dispatcher.start();

    let envelope = DispatchEnvelope::from_message(make_message("missing", "no-retry", 1))
        .with_retry_policy(2, 10);
    dispatcher.enqueue_envelope(envelope);

    let first = dispatcher.drain_one().unwrap();
    assert!(!first.success);
    assert_eq!(dispatcher.queue_snapshot().total_len, 0);

    let send_retry_registry = Arc::new(ChannelRegistry::new());
    let send_retry_dispatcher = Dispatcher::new(send_retry_registry.clone(), None);
    send_retry_dispatcher.start();
    let failing = Arc::new(TestChannel::failing("cli"));
    send_retry_registry.register(failing, Some("default"));

    let send_retry = DispatchEnvelope::from_message(make_message("cli", "retry-send", 1))
        .with_retry_policy(1, 10);
    send_retry_dispatcher.enqueue_envelope(send_retry);
    let failed_send = send_retry_dispatcher.drain_one().unwrap();
    assert!(!failed_send.success);
    assert_eq!(send_retry_dispatcher.queue_snapshot().total_len, 1);
}

#[test]
fn dispatcher_route_reports_expose_route_level_queue_and_balancer_state() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry, None);
    let balancer = Arc::new(anima_runtime::dispatcher::Balancer::new(
        anima_runtime::dispatcher::BalancerOptions::default(),
    ));
    balancer.add_target(anima_runtime::dispatcher::Target::new("default"));
    dispatcher.add_balancer("cli", balancer);
    dispatcher.enqueue(make_message("cli", "route-report", 2));

    let report = dispatcher.route_report("cli").unwrap();
    assert_eq!(report.channel, "cli");
    assert_eq!(report.pending_queue_depth, 1);
    assert_eq!(report.target_count, 1);

    let reports = dispatcher.route_reports();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].channel, "cli");
}
