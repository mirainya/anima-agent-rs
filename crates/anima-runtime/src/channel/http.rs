use crate::channel::adapter::{err, ok, Channel, SendOptions, SendResult};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelCapabilities {
    pub outbound: bool,
    pub inbound: bool,
    pub media: bool,
    pub health_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelHealthSnapshot {
    pub status: String,
    pub running: bool,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpChannelConfig {
    pub name: String,
    pub endpoint: String,
    pub timeout_ms: u64,
    pub auth_header: Option<String>,
}

impl Default for HttpChannelConfig {
    fn default() -> Self {
        Self {
            name: "http".into(),
            endpoint: "http://127.0.0.1:3000/messages".into(),
            timeout_ms: 5_000,
            auth_header: None,
        }
    }
}

pub trait ChannelAdapter: Channel {
    fn capabilities(&self) -> ChannelCapabilities;
    fn health_snapshot(&self) -> ChannelHealthSnapshot;
}

pub struct HttpChannel {
    config: HttpChannelConfig,
    client: Client,
    running: AtomicBool,
    last_health: Mutex<Option<ChannelHealthSnapshot>>,
}

impl HttpChannel {
    pub fn new(config: HttpChannelConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .expect("http client should build");
        Self {
            config,
            client,
            running: AtomicBool::new(false),
            last_health: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &HttpChannelConfig {
        &self.config
    }

    pub fn send_payload(&self, payload: &Value) -> SendResult {
        if !self.running.load(Ordering::SeqCst) {
            return err("HTTP channel is not running", None);
        }

        let mut request = self.client.post(&self.config.endpoint).json(payload);
        if let Some(auth_header) = &self.config.auth_header {
            request = request.header("authorization", auth_header);
        }

        match request.send() {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    let data = response.json::<Value>().ok();
                    let snapshot = ChannelHealthSnapshot {
                        status: "healthy".into(),
                        running: true,
                        details: json!({"status_code": status.as_u16()}),
                    };
                    *self.last_health.lock().unwrap() = Some(snapshot);
                    ok(data)
                } else {
                    let snapshot = ChannelHealthSnapshot {
                        status: "unhealthy".into(),
                        running: true,
                        details: json!({"status_code": status.as_u16()}),
                    };
                    *self.last_health.lock().unwrap() = Some(snapshot.clone());
                    err(format!("HTTP send failed with status {}", status.as_u16()), Some(snapshot.details))
                }
            }
            Err(error) => {
                let snapshot = ChannelHealthSnapshot {
                    status: "unhealthy".into(),
                    running: true,
                    details: json!({"error": error.to_string()}),
                };
                *self.last_health.lock().unwrap() = Some(snapshot.clone());
                err(error.to_string(), Some(snapshot.details))
            }
        }
    }
}

impl Channel for HttpChannel {
    fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn send_message(&self, target: &str, message: &str, opts: SendOptions) -> SendResult {
        self.send_payload(&json!({
            "target": target,
            "message": message,
            "media": opts.media,
            "stage": opts.stage,
        }))
    }

    fn channel_name(&self) -> &str {
        &self.config.name
    }

    fn health_check(&self) -> bool {
        self.running.load(Ordering::SeqCst)
            && self
                .last_health
                .lock()
                .unwrap()
                .as_ref()
                .map(|snapshot| snapshot.status == "healthy")
                .unwrap_or(true)
    }
}

impl ChannelAdapter for HttpChannel {
    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            outbound: true,
            inbound: false,
            media: true,
            health_check: true,
        }
    }

    fn health_snapshot(&self) -> ChannelHealthSnapshot {
        self.last_health
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(ChannelHealthSnapshot {
                status: if self.running.load(Ordering::SeqCst) {
                    "unknown"
                } else {
                    "stopped"
                }
                .into(),
                running: self.running.load(Ordering::SeqCst),
                details: json!({}),
            })
    }
}
