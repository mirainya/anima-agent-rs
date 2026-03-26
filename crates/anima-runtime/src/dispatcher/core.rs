use crate::support::now_ms;
use crate::bus::Bus;
use crate::channel::{err, ChannelRegistry, SendOptions, SendResult};
use crate::dispatcher::balancer::{Balancer, BalancerMissReason, LoadUpdate, Target};
use crate::dispatcher::diagnostics::{
    DispatchDiagnosticSnapshot, DispatchFailureStage, DispatchOutcomeReason, DispatcherRuntimeState,
    DispatcherState, DispatcherStats,
};
use crate::dispatcher::message::{DispatchMessage, DispatcherConfig};
use crate::dispatcher::queue::{DispatchEnvelope, DispatchQueue, DispatchQueueSnapshot};
use indexmap::IndexMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub struct Dispatcher {
    config: DispatcherConfig,
    registry: Arc<ChannelRegistry>,
    stats: Arc<DispatcherStats>,
    runtime: DispatcherRuntimeState,
    balancers: Mutex<IndexMap<String, Arc<Balancer>>>,
    queue: Arc<DispatchQueue>,
}

impl Dispatcher {
    pub fn new(registry: Arc<ChannelRegistry>, config: Option<DispatcherConfig>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            registry,
            stats: Arc::new(DispatcherStats::new()),
            runtime: DispatcherRuntimeState::default(),
            balancers: Mutex::new(IndexMap::new()),
            queue: Arc::new(DispatchQueue::new()),
        }
    }

    pub fn config(&self) -> &DispatcherConfig {
        &self.config
    }

    pub fn stats(&self) -> Arc<DispatcherStats> {
        self.stats.clone()
    }

    pub fn add_balancer(&self, channel: impl Into<String>, balancer: Arc<Balancer>) {
        self.balancers.lock().unwrap().insert(channel.into(), balancer);
    }

    pub fn remove_balancer(&self, channel: &str) -> Option<Arc<Balancer>> {
        self.balancers.lock().unwrap().shift_remove(channel)
    }

    pub fn get_balancer(&self, channel: &str) -> Option<Arc<Balancer>> {
        self.balancers.lock().unwrap().get(channel).cloned()
    }

    pub fn last_dispatch_diagnostic(&self) -> Option<DispatchDiagnosticSnapshot> {
        self.runtime.last_dispatch_diagnostic()
    }

    pub fn queue(&self) -> Arc<DispatchQueue> {
        self.queue.clone()
    }

    pub fn queue_snapshot(&self) -> DispatchQueueSnapshot {
        self.queue.snapshot()
    }

    pub fn route_queue_depth(&self, channel: &str) -> usize {
        self.queue_snapshot().by_route.get(channel).copied().unwrap_or(0)
    }

    pub fn start(&self) {
        self.runtime.set_state(DispatcherState::Running);
    }

    pub fn pause(&self) {
        self.runtime.set_state(DispatcherState::Paused);
    }

    pub fn resume(&self) {
        self.runtime.set_state(DispatcherState::Running);
    }

    pub fn stop(&self) {
        self.runtime.set_state(DispatcherState::Stopped);
    }

    pub fn drain(&self) {
        self.runtime.set_state(DispatcherState::Draining);
        while self.drain_one().is_some() {}
        self.runtime.set_state(DispatcherState::Stopped);
    }

    pub fn enqueue(&self, message: DispatchMessage) {
        self.queue.enqueue(DispatchEnvelope::from_message(message));
    }

    pub fn enqueue_envelope(&self, envelope: DispatchEnvelope) {
        self.queue.enqueue(envelope);
    }

    pub fn drain_one(&self) -> Option<SendResult> {
        match self.runtime.current_state() {
            DispatcherState::Paused | DispatcherState::Stopped => return None,
            DispatcherState::Running | DispatcherState::Draining => {}
        }
        let envelope = self.queue.dequeue()?;
        let result = self.dispatch(&envelope.message);
        if !result.success {
            let failure_stage = self
                .last_dispatch_diagnostic()
                .and_then(|diagnostic| diagnostic.failure_stage);
            if envelope.should_retry_for_stage(failure_stage) {
                if let Some(retry) = envelope.next_retry() {
                    self.queue.enqueue(retry);
                }
            }
        }
        Some(result)
    }

    pub fn status(&self) -> crate::dispatcher::diagnostics::DispatcherStatus {
        let snapshot = self.queue.snapshot();
        self.runtime.status(&self.balancers, snapshot.total_len, &snapshot.by_route)
    }

    pub fn route_report(&self, channel: &str) -> Option<crate::dispatcher::diagnostics::DispatcherRouteReport> {
        let status = self.status();
        let route = status.routes.get(channel)?;
        Some(crate::dispatcher::diagnostics::DispatcherRouteReport {
            channel: channel.to_string(),
            pending_queue_depth: route.pending_queue_depth,
            balancer_id: route.balancer_id.clone(),
            strategy: route.strategy.clone(),
            target_count: route.target_count,
            available_target_count: route.available_target_count,
            healthy_target_count: route.healthy_target_count,
            open_circuit_count: route.open_circuit_count,
            half_open_target_count: route.half_open_target_count,
            last_miss_reason: route.last_miss_reason,
        })
    }

    pub fn route_reports(&self) -> Vec<crate::dispatcher::diagnostics::DispatcherRouteReport> {
        let status = self.status();
        status
            .routes
            .iter()
            .map(|(channel, route)| crate::dispatcher::diagnostics::DispatcherRouteReport {
                channel: channel.clone(),
                pending_queue_depth: route.pending_queue_depth,
                balancer_id: route.balancer_id.clone(),
                strategy: route.strategy.clone(),
                target_count: route.target_count,
                available_target_count: route.available_target_count,
                healthy_target_count: route.healthy_target_count,
                open_circuit_count: route.open_circuit_count,
                half_open_target_count: route.half_open_target_count,
                last_miss_reason: route.last_miss_reason,
            })
            .collect()
    }

    pub fn dispatch(&self, message: &DispatchMessage) -> SendResult {
        self.runtime.set_state(DispatcherState::Running);

        let selection = self.select_target(message);
        if selection.selected_target.is_none() && selection.balancer.is_some() {
            self.runtime.record_dispatch_diagnostic(DispatchDiagnosticSnapshot {
                message_id: message.id.clone(),
                channel: message.channel.clone(),
                account_id: message.account_id.clone(),
                selection_key: message.selection_key().map(ToString::to_string),
                selected_target_id: None,
                balancer_miss_reason: selection.balancer_miss_reason,
                channel_lookup_reason: None,
                failure_stage: Some(DispatchFailureStage::BalancerSelection),
                send_error: None,
                outcome: DispatchOutcomeReason::BalancerMiss,
                timestamp_ms: now_ms(),
            });
        }

        let selected_account_id = selection
            .selected_target
            .as_ref()
            .map(|target| target.id.clone())
            .unwrap_or_else(|| message.account_id.clone());

        let (channel, lookup_snapshot) = self
            .registry
            .find_channel_with_lookup(&message.channel, Some(selected_account_id.as_str()));
        let Some(channel) = channel else {
            self.stats.record_channel_not_found(
                selection.selected_target.as_ref().map(|target| target.id.as_str()),
                lookup_snapshot.reason,
            );
            self.runtime.record_error();
            self.runtime.record_dispatch_diagnostic(DispatchDiagnosticSnapshot {
                message_id: message.id.clone(),
                channel: message.channel.clone(),
                account_id: message.account_id.clone(),
                selection_key: message.selection_key().map(ToString::to_string),
                selected_target_id: selection.selected_target.as_ref().map(|target| target.id.clone()),
                balancer_miss_reason: selection.balancer_miss_reason,
                channel_lookup_reason: Some(lookup_snapshot.reason),
                failure_stage: Some(DispatchFailureStage::ChannelLookup),
                send_error: None,
                outcome: DispatchOutcomeReason::ChannelNotFound,
                timestamp_ms: now_ms(),
            });
            self.after_dispatch(
                selection.balancer.as_deref(),
                selection.selected_target.as_ref(),
                false,
                false,
            );
            return err("Channel not found", None);
        };

        let result = channel.send_message(
            &message.target(),
            &message.content,
            SendOptions {
                media: message.media.clone(),
                stage: Some(message.stage.clone()),
            },
        );

        if result.success {
            self.stats.record_dispatch_success(
                selection.selected_target.as_ref().map(|target| target.id.as_str()),
            );
            self.runtime.record_dispatch_success_timestamp();
            self.runtime.record_dispatch_diagnostic(DispatchDiagnosticSnapshot {
                message_id: message.id.clone(),
                channel: message.channel.clone(),
                account_id: message.account_id.clone(),
                selection_key: message.selection_key().map(ToString::to_string),
                selected_target_id: selection.selected_target.as_ref().map(|target| target.id.clone()),
                balancer_miss_reason: selection.balancer_miss_reason,
                channel_lookup_reason: Some(lookup_snapshot.reason),
                failure_stage: None,
                send_error: None,
                outcome: DispatchOutcomeReason::SelectedAndSent,
                timestamp_ms: now_ms(),
            });
        } else {
            self.stats.record_dispatch_error(
                selection.selected_target.as_ref().map(|target| target.id.as_str()),
            );
            self.runtime.record_error();
            self.runtime.record_dispatch_diagnostic(DispatchDiagnosticSnapshot {
                message_id: message.id.clone(),
                channel: message.channel.clone(),
                account_id: message.account_id.clone(),
                selection_key: message.selection_key().map(ToString::to_string),
                selected_target_id: selection.selected_target.as_ref().map(|target| target.id.clone()),
                balancer_miss_reason: selection.balancer_miss_reason,
                channel_lookup_reason: Some(lookup_snapshot.reason),
                failure_stage: Some(DispatchFailureStage::Send),
                send_error: result.error.clone(),
                outcome: DispatchOutcomeReason::SendFailed,
                timestamp_ms: now_ms(),
            });
        }

        self.after_dispatch(
            selection.balancer.as_deref(),
            selection.selected_target.as_ref(),
            result.success,
            true,
        );
        result
    }

    fn select_target(&self, message: &DispatchMessage) -> Selection {
        let Some(balancer) = self.get_balancer(&message.channel) else {
            return Selection {
                balancer: None,
                selected_target: None,
                balancer_miss_reason: None,
            };
        };

        balancer.refresh_targets(now_ms());
        let selected_target = balancer.select_target(message.selection_key());
        let balancer_diagnostic = balancer.diagnostics_snapshot();
        let balancer_miss_reason = balancer_diagnostic
            .last_selection
            .as_ref()
            .and_then(|selection| selection.miss_reason);
        match selected_target.as_ref() {
            Some(target) => {
                self.stats.record_balancer_selected(&target.id);
                self.before_send(&balancer, target);
            }
            None => self.stats.record_balancer_miss(balancer_miss_reason),
        }

        Selection {
            balancer: Some(balancer),
            selected_target,
            balancer_miss_reason,
        }
    }

    fn before_send(&self, balancer: &Balancer, target: &Target) {
        let _ = balancer.update_load(&target.id, LoadUpdate::Inc);
    }

    fn after_dispatch(
        &self,
        balancer: Option<&Balancer>,
        target: Option<&Target>,
        success: bool,
        send_attempted: bool,
    ) {
        let (Some(balancer), Some(target)) = (balancer, target) else {
            return;
        };

        let _ = if success {
            let _ = balancer.record_target_success(&target.id);
            balancer.update_load(&target.id, LoadUpdate::Dec)
        } else {
            if send_attempted {
                let _ = balancer.record_target_failure(&target.id);
            }
            balancer.update_load(&target.id, LoadUpdate::Error)
        };
    }
}

struct Selection {
    balancer: Option<Arc<Balancer>>,
    selected_target: Option<Target>,
    balancer_miss_reason: Option<BalancerMissReason>,
}

pub fn start_dispatch_scheduler(dispatcher: Arc<Dispatcher>) -> thread::JoinHandle<()> {
    dispatcher.start();
    thread::spawn(move || loop {
        match dispatcher.status().state {
            DispatcherState::Stopped => break,
            DispatcherState::Paused => {
                thread::sleep(Duration::from_millis(dispatcher.config.receive_timeout_ms));
                continue;
            }
            DispatcherState::Running | DispatcherState::Draining => {}
        }

        if dispatcher.queue().is_empty() {
            if dispatcher.status().state == DispatcherState::Draining {
                dispatcher.stop();
                break;
            }
            thread::sleep(Duration::from_millis(dispatcher.config.receive_timeout_ms));
            continue;
        }
        let _ = dispatcher.drain_one();
    })
}

pub fn start_dispatcher_outbound_loop(
    bus: Arc<Bus>,
    dispatcher: Arc<Dispatcher>,
) -> thread::JoinHandle<()> {
    dispatcher.start();
    thread::spawn(move || loop {
        if bus.is_closed() && bus.outbound_receiver().is_empty() {
            break;
        }
        match bus
            .outbound_receiver()
            .recv_timeout(Duration::from_millis(dispatcher.config.receive_timeout_ms))
        {
            Ok(msg) => {
                let dispatch_message = DispatchMessage::from_outbound(&msg);
                dispatcher.enqueue(dispatch_message);
                let _ = dispatcher.drain_one();
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

