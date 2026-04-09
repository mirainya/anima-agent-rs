use anima_runtime::bus::{make_outbound, MakeOutbound};
use anima_runtime::channel::{ChannelLookupReason, ChannelRegistry, TestChannel};
use anima_runtime::dispatcher::{
    Balancer, BalancerMissReason, BalancerOptions, BalancerStrategy, DispatchFailureStage,
    DispatchMessage, DispatchOutcomeReason, Dispatcher, Target,
};
use std::sync::Arc;

fn outbound(channel: &str) -> anima_runtime::bus::OutboundMessage {
    make_outbound(MakeOutbound {
        channel: channel.into(),
        account_id: Some("default".into()),
        chat_id: Some("chat-1".into()),
        content: "payload".into(),
        reply_target: Some("reply-user".into()),
        sender_id: Some("sender-user".into()),
        stage: Some("final".into()),
        ..Default::default()
    })
}

#[test]
fn dispatch_message_selection_key_prefers_session_key_then_chat_then_sender() {
    let mut from_session = DispatchMessage::from_outbound(&outbound("cli"));
    from_session.session_key = Some("session-key".into());
    from_session.chat_id = Some("chat-key".into());
    from_session.sender_id = Some("sender-key".into());
    assert_eq!(from_session.selection_key(), Some("session-key"));

    let mut from_chat = DispatchMessage::from_outbound(&outbound("cli"));
    from_chat.session_key = None;
    from_chat.chat_id = Some("chat-key".into());
    from_chat.sender_id = Some("sender-key".into());
    assert_eq!(from_chat.selection_key(), Some("chat-key"));

    let mut from_sender = DispatchMessage::from_outbound(&outbound("cli"));
    from_sender.session_key = None;
    from_sender.chat_id = None;
    from_sender.sender_id = Some("sender-key".into());
    assert_eq!(from_sender.selection_key(), Some("sender-key"));
}

#[test]
fn dispatcher_exact_lookup_success_records_running_status_and_timestamps() {
    let registry = Arc::new(ChannelRegistry::new());
    registry.register(Arc::new(TestChannel::new("cli")), Some("default"));
    let dispatcher = Dispatcher::new(registry, None);

    let result = dispatcher.dispatch(&DispatchMessage::from_outbound(&outbound("cli")));
    assert!(result.success);

    let status = dispatcher.status();
    assert_eq!(
        status.state,
        anima_runtime::dispatcher::DispatcherState::Running
    );
    assert!(status.last_dispatch_at.is_some());
    assert_eq!(status.last_error_at, None);
    assert_eq!(status.queue_depth, 0);

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(diagnostic.outcome, DispatchOutcomeReason::SelectedAndSent);
    assert_eq!(diagnostic.failure_stage, None);
    assert_eq!(
        diagnostic.channel_lookup_reason,
        Some(ChannelLookupReason::ExactMatch)
    );
}

#[test]
fn dispatcher_channel_lookup_failure_attributes_error_to_lookup_stage() {
    let registry = Arc::new(ChannelRegistry::new());
    let dispatcher = Dispatcher::new(registry, None);

    let result = dispatcher.dispatch(&DispatchMessage::from_outbound(&outbound("missing")));
    assert!(!result.success);

    let status = dispatcher.status();
    assert!(status.last_error_at.is_some());

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(
        diagnostic.failure_stage,
        Some(DispatchFailureStage::ChannelLookup)
    );
    assert_eq!(diagnostic.outcome, DispatchOutcomeReason::ChannelNotFound);
    assert_eq!(
        diagnostic.channel_lookup_reason,
        Some(ChannelLookupReason::ChannelMissing)
    );
}

#[test]
fn dispatcher_balancer_miss_keeps_selected_target_empty_and_reports_reason() {
    let registry = Arc::new(ChannelRegistry::new());
    registry.register(Arc::new(TestChannel::new("cli")), Some("default"));
    let dispatcher = Dispatcher::new(registry, None);

    let balancer = Arc::new(Balancer::new(BalancerOptions {
        strategy: BalancerStrategy::Hashing,
        ..Default::default()
    }));
    balancer.add_target(Target::new("default"));
    dispatcher.add_balancer("cli", balancer);

    let mut msg = DispatchMessage::from_outbound(&outbound("cli"));
    msg.session_key = None;
    msg.chat_id = None;
    msg.sender_id = None;
    msg.reply_target = Some("reply-user".into());

    let result = dispatcher.dispatch(&msg);
    assert!(result.success);

    let diagnostic = dispatcher.last_dispatch_diagnostic().unwrap();
    assert_eq!(diagnostic.selected_target_id, None);
    assert_eq!(
        diagnostic.balancer_miss_reason,
        Some(BalancerMissReason::HashingKeyMissing)
    );
    assert_eq!(diagnostic.outcome, DispatchOutcomeReason::SelectedAndSent);
    assert_eq!(
        diagnostic.channel_lookup_reason,
        Some(ChannelLookupReason::ExactMatch)
    );
}
