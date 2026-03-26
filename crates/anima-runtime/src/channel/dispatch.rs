use crate::bus::{Bus, OutboundMessage};
use crate::channel::adapter::{err, SendOptions, SendResult};
use crate::channel::registry::ChannelRegistry;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct DispatchStats {
    pub dispatched: AtomicUsize,
    pub errors: AtomicUsize,
    pub channel_not_found: AtomicUsize,
}

impl Default for DispatchStats {
    fn default() -> Self {
        Self {
            dispatched: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            channel_not_found: AtomicUsize::new(0),
        }
    }
}

impl DispatchStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&self) {
        self.dispatched.store(0, Ordering::SeqCst);
        self.errors.store(0, Ordering::SeqCst);
        self.channel_not_found.store(0, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> DispatchStatsSnapshot {
        DispatchStatsSnapshot {
            dispatched: self.dispatched.load(Ordering::SeqCst),
            errors: self.errors.load(Ordering::SeqCst),
            channel_not_found: self.channel_not_found.load(Ordering::SeqCst),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DispatchStatsSnapshot {
    pub dispatched: usize,
    pub errors: usize,
    pub channel_not_found: usize,
}

pub fn dispatch_outbound_message(
    msg: &OutboundMessage,
    registry: &ChannelRegistry,
    stats: &DispatchStats,
) -> SendResult {
    let Some(channel) = registry.find_channel(&msg.channel, Some(&msg.account_id)) else {
        stats.channel_not_found.fetch_add(1, Ordering::SeqCst);
        return err("Channel not found", None);
    };

    let target = msg
        .reply_target
        .clone()
        .or_else(|| msg.sender_id.clone())
        .unwrap_or_default();
    let result = channel.send_message(
        &target,
        &msg.content,
        SendOptions {
            media: msg.media.clone(),
            stage: Some(msg.stage.clone()),
        },
    );

    if result.success {
        stats.dispatched.fetch_add(1, Ordering::SeqCst);
    } else {
        stats.errors.fetch_add(1, Ordering::SeqCst);
    }
    result
}

pub fn start_outbound_dispatch(
    bus: Arc<Bus>,
    registry: Arc<ChannelRegistry>,
    stats: Arc<DispatchStats>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        if bus.is_closed() && bus.outbound_receiver().is_empty() {
            break;
        }
        match bus
            .outbound_receiver()
            .recv_timeout(Duration::from_millis(25))
        {
            Ok(msg) => {
                let _ = dispatch_outbound_message(&msg, &registry, &stats);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if bus.is_closed() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    })
}
