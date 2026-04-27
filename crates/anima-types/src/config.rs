use serde::Deserialize;
use std::path::Path;

use crate::approval::ApprovalMode;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AnimaConfig {
    pub server: ServerConfig,
    pub cli: CliConfig,
    pub sdk: SdkConfig,
    pub runtime: RuntimeConfig,
    pub worker: WorkerConfig,
    pub approval_mode: ApprovalMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    pub prompt: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SdkConfig {
    pub request_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub retry_backoff_cap_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub state_path: String,
    pub builtin_tools: bool,
    pub store: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WorkerConfig {
    pub pool_size: usize,
}


impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:9711".into(),
        }
    }
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            prompt: "anima> ".into(),
            enabled: true,
        }
    }
}

impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            request_timeout_ms: 600_000,
            connect_timeout_ms: 60_000,
            max_retries: 1,
            retry_backoff_ms: 250,
            retry_backoff_cap_ms: 5_000,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            state_path: ".opencode/runtime/state.json".into(),
            builtin_tools: true,
            store: "json".into(),
        }
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { pool_size: 4 }
    }
}

impl AnimaConfig {
    pub fn load(path: Option<&Path>) -> Self {
        let mut config = match path {
            Some(p) => Self::load_from_file(p),
            None => {
                let default_path = Path::new("anima.toml");
                if default_path.exists() {
                    Self::load_from_file(default_path)
                } else {
                    Self::default()
                }
            }
        };
        config.apply_env_overrides();
        config
    }

    fn load_from_file(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("ANIMA_SDK_REQUEST_TIMEOUT_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.request_timeout_ms = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_CONNECT_TIMEOUT_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.connect_timeout_ms = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_MAX_RETRIES") {
            if let Ok(v) = v.parse() {
                self.sdk.max_retries = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_RETRY_BACKOFF_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.retry_backoff_ms = v;
            }
        }
        if let Ok(v) = std::env::var("ANIMA_SDK_RETRY_BACKOFF_CAP_MS") {
            if let Ok(v) = v.parse() {
                self.sdk.retry_backoff_cap_ms = v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_hardcoded_values() {
        let c = AnimaConfig::default();
        assert_eq!(c.server.url, "http://127.0.0.1:9711");
        assert_eq!(c.cli.prompt, "anima> ");
        assert!(c.cli.enabled);
        assert_eq!(c.sdk.request_timeout_ms, 600_000);
        assert_eq!(c.sdk.connect_timeout_ms, 60_000);
        assert_eq!(c.sdk.max_retries, 1);
        assert_eq!(c.runtime.state_path, ".opencode/runtime/state.json");
        assert!(c.runtime.builtin_tools);
        assert_eq!(c.worker.pool_size, 4);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let c = AnimaConfig::load(Some(Path::new("nonexistent.toml")));
        assert_eq!(c.server.url, "http://127.0.0.1:9711");
    }

    #[test]
    fn parse_partial_toml() {
        let toml_str = r#"
[server]
url = "http://localhost:8080"

[worker]
pool_size = 8
"#;
        let c: AnimaConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(c.server.url, "http://localhost:8080");
        assert_eq!(c.worker.pool_size, 8);
        // unset fields keep defaults
        assert_eq!(c.sdk.request_timeout_ms, 600_000);
        assert_eq!(c.cli.prompt, "anima> ");
    }

    #[test]
    fn env_override_takes_precedence() {
        std::env::set_var("ANIMA_SDK_MAX_RETRIES", "5");
        let mut c = AnimaConfig::default();
        c.apply_env_overrides();
        assert_eq!(c.sdk.max_retries, 5);
        std::env::remove_var("ANIMA_SDK_MAX_RETRIES");
    }
}
