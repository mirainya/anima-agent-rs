use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFacadeDispatcherStatus {
    pub state: String,
    pub queue_depth: usize,
    pub route_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFacadeRouteReport {
    pub channel: String,
    pub pending_queue_depth: usize,
    pub target_count: usize,
    pub healthy_target_count: usize,
    pub open_circuit_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFacadeStatus {
    pub dispatcher: RuntimeFacadeDispatcherStatus,
    pub route_reports: Vec<RuntimeFacadeRouteReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBootstrapOptions {
    pub url: String,
    pub prompt: String,
    pub cli_enabled: bool,
}

impl Default for RuntimeBootstrapOptions {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:9711".into(),
            prompt: "anima> ".into(),
            cli_enabled: true,
        }
    }
}

pub struct RuntimeBootstrapOptionsBuilder {
    options: RuntimeBootstrapOptions,
}

impl Default for RuntimeBootstrapOptionsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBootstrapOptionsBuilder {
    pub fn new() -> Self {
        Self {
            options: RuntimeBootstrapOptions::default(),
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.options.url = url.into();
        self
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.options.prompt = prompt.into();
        self
    }

    pub fn with_cli_enabled(mut self, enabled: bool) -> Self {
        self.options.cli_enabled = enabled;
        self
    }

    pub fn build(self) -> RuntimeBootstrapOptions {
        self.options
    }
}
