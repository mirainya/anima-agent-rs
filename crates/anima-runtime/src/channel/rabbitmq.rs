use crate::channel::adapter::{err, Channel, SendOptions, SendResult};
use crate::channel::http::{ChannelAdapter, ChannelCapabilities, ChannelHealthSnapshot};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RabbitMqChannelConfig {
    pub name: String,
    pub uri: String,
    pub exchange: String,
    pub routing_key: String,
    pub queue: Option<String>,
}

impl Default for RabbitMqChannelConfig {
    fn default() -> Self {
        Self {
            name: "rabbitmq".into(),
            uri: "amqp://guest:guest@127.0.0.1:5672/%2f".into(),
            exchange: "anima.outbound".into(),
            routing_key: "anima.message".into(),
            queue: None,
        }
    }
}

pub struct RabbitMqChannel {
    config: RabbitMqChannelConfig,
    running: AtomicBool,
    last_health: Mutex<Option<ChannelHealthSnapshot>>,
}

impl RabbitMqChannel {
    pub fn new(config: RabbitMqChannelConfig) -> Self {
        Self {
            config,
            running: AtomicBool::new(false),
            last_health: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &RabbitMqChannelConfig {
        &self.config
    }
}

impl Channel for RabbitMqChannel {
    fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
        *self.last_health.lock() = Some(ChannelHealthSnapshot {
            status: "unknown".into(),
            running: true,
            details: json!({
                "uri": self.config.uri,
                "exchange": self.config.exchange,
                "routing_key": self.config.routing_key,
            }),
        });
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn send_message(&self, target: &str, message: &str, opts: SendOptions) -> SendResult {
        if !self.running.load(Ordering::SeqCst) {
            return err("RabbitMQ channel is not running", None);
        }

        let snapshot = ChannelHealthSnapshot {
            status: "unimplemented".into(),
            running: true,
            details: json!({
                "target": target,
                "message": message,
                "media": opts.media,
                "stage": opts.stage,
                "uri": self.config.uri,
                "exchange": self.config.exchange,
                "routing_key": self.config.routing_key,
            }),
        };
        *self.last_health.lock() = Some(snapshot.clone());
        err(
            "RabbitMQ adapter skeleton is not connected to a live broker yet",
            Some(snapshot.details),
        )
    }

    fn channel_name(&self) -> &str {
        &self.config.name
    }

    fn health_check(&self) -> bool {
        self.running.load(Ordering::SeqCst)
            && self
                .last_health
                .lock()
                .as_ref()
                .map(|snapshot| snapshot.status == "healthy")
                .unwrap_or(false)
    }
}

impl ChannelAdapter for RabbitMqChannel {
    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            outbound: true,
            inbound: true,
            media: false,
            health_check: true,
        }
    }

    fn health_snapshot(&self) -> ChannelHealthSnapshot {
        self.last_health
            .lock()
            .clone()
            .unwrap_or(ChannelHealthSnapshot {
                status: if self.running.load(Ordering::SeqCst) {
                    "unknown"
                } else {
                    "stopped"
                }
                .into(),
                running: self.running.load(Ordering::SeqCst),
                details: json!({
                    "exchange": self.config.exchange,
                    "routing_key": self.config.routing_key,
                }),
            })
    }
}
