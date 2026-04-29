use crate::channel::adapter::Channel;
use indexmap::IndexMap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type ChannelMap = IndexMap<String, IndexMap<String, Arc<dyn Channel>>>;

pub struct ChannelRegistry {
    channels: Mutex<ChannelMap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChannelLookupReason {
    ExactMatch,
    DefaultFallback,
    FirstAvailableFallback,
    ChannelMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelLookupSnapshot {
    pub requested_channel: String,
    pub requested_account_id: Option<String>,
    pub matched_account_id: Option<String>,
    pub reason: ChannelLookupReason,
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self {
            channels: Mutex::new(IndexMap::new()),
        }
    }
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, channel: Arc<dyn Channel>, account_id: Option<&str>) {
        let mut channels = self.channels.lock();
        let account = account_id.unwrap_or("default").to_string();
        channels
            .entry(channel.channel_name().to_string())
            .or_default()
            .insert(account, channel);
    }

    pub fn unregister(&self, channel_name: &str) {
        self.channels.lock().shift_remove(channel_name);
    }

    pub fn unregister_with_account(&self, channel_name: &str, account_id: &str) {
        let mut channels = self.channels.lock();
        if account_id == "default" {
            channels.shift_remove(channel_name);
        } else if let Some(accounts) = channels.get_mut(channel_name) {
            accounts.shift_remove(account_id);
        }
    }

    pub fn find_channel(
        &self,
        channel_name: &str,
        account_id: Option<&str>,
    ) -> Option<Arc<dyn Channel>> {
        self.find_channel_with_lookup(channel_name, account_id).0
    }

    pub fn find_channel_with_lookup(
        &self,
        channel_name: &str,
        account_id: Option<&str>,
    ) -> (Option<Arc<dyn Channel>>, ChannelLookupSnapshot) {
        let channels = self.channels.lock();
        let requested_account_id = account_id.map(ToString::to_string);
        let Some(accounts) = channels.get(channel_name) else {
            return (
                None,
                ChannelLookupSnapshot {
                    requested_channel: channel_name.to_string(),
                    requested_account_id,
                    matched_account_id: None,
                    reason: ChannelLookupReason::ChannelMissing,
                },
            );
        };

        let requested = account_id.unwrap_or("default");
        if let Some(channel) = accounts.get(requested) {
            return (
                Some(channel.clone()),
                ChannelLookupSnapshot {
                    requested_channel: channel_name.to_string(),
                    requested_account_id,
                    matched_account_id: Some(requested.to_string()),
                    reason: ChannelLookupReason::ExactMatch,
                },
            );
        }
        if let Some(channel) = accounts.get("default") {
            return (
                Some(channel.clone()),
                ChannelLookupSnapshot {
                    requested_channel: channel_name.to_string(),
                    requested_account_id,
                    matched_account_id: Some("default".to_string()),
                    reason: ChannelLookupReason::DefaultFallback,
                },
            );
        }
        if let Some((matched_account_id, channel)) = accounts.first() {
            return (
                Some(channel.clone()),
                ChannelLookupSnapshot {
                    requested_channel: channel_name.to_string(),
                    requested_account_id,
                    matched_account_id: Some(matched_account_id.clone()),
                    reason: ChannelLookupReason::FirstAvailableFallback,
                },
            );
        }

        (
            None,
            ChannelLookupSnapshot {
                requested_channel: channel_name.to_string(),
                requested_account_id,
                matched_account_id: None,
                reason: ChannelLookupReason::ChannelMissing,
            },
        )
    }

    pub fn all_channels(&self) -> Vec<Arc<dyn Channel>> {
        self.channels
            .lock()
            .values()
            .flat_map(|accounts| accounts.values().cloned())
            .collect()
    }

    pub fn channel_names(&self) -> Vec<String> {
        self.channels.lock().keys().cloned().collect()
    }

    pub fn entries(&self) -> Vec<ChannelRegistryEntry> {
        self.channels
            .lock()
            .iter()
            .flat_map(|(channel, accounts)| {
                accounts
                    .keys()
                    .cloned()
                    .map(|account_id| ChannelRegistryEntry {
                        channel: channel.clone(),
                        account_id,
                    })
            })
            .collect()
    }

    pub fn channel_count(&self) -> usize {
        self.channels.lock().values().map(IndexMap::len).sum()
    }

    pub fn start_all(&self) {
        for channel in self.all_channels() {
            channel.start();
        }
    }

    pub fn stop_all(&self) {
        for channel in self.all_channels() {
            channel.stop();
        }
    }

    pub fn health_report(&self) -> HealthReport {
        let channels = self.all_channels();
        let details: Vec<ChannelHealth> = channels
            .iter()
            .map(|channel| ChannelHealth {
                name: channel.channel_name().to_string(),
                status: if channel.health_check() {
                    "healthy".into()
                } else {
                    "unhealthy".into()
                },
            })
            .collect();
        let healthy = details.iter().filter(|c| c.status == "healthy").count();
        let total = details.len();
        HealthReport {
            healthy,
            unhealthy: total.saturating_sub(healthy),
            total,
            all_healthy: total > 0 && healthy == total,
            channels: details,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelHealth {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRegistryEntry {
    pub channel: String,
    pub account_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthReport {
    pub healthy: usize,
    pub unhealthy: usize,
    pub total: usize,
    pub all_healthy: bool,
    pub channels: Vec<ChannelHealth>,
}
