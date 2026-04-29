use reqwest::blocking::Client as HttpClient;
use std::time::Duration;

use anima_types::config::SdkConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOptions {
    pub request_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub retry_backoff_cap_ms: u64,
}

impl Default for ClientOptions {
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

impl ClientOptions {
    /// 从环境变量加载配置，未设置时保留默认值
    ///
    /// 支持的变量：
    /// - `ANIMA_SDK_REQUEST_TIMEOUT_MS`
    /// - `ANIMA_SDK_CONNECT_TIMEOUT_MS`
    /// - `ANIMA_SDK_MAX_RETRIES`
    /// - `ANIMA_SDK_RETRY_BACKOFF_MS`
    /// - `ANIMA_SDK_RETRY_BACKOFF_CAP_MS`
    pub fn from_env() -> Self {
        let mut options = Self::default();
        if let Ok(value) = std::env::var("ANIMA_SDK_REQUEST_TIMEOUT_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                options.request_timeout_ms = parsed;
            }
        }
        if let Ok(value) = std::env::var("ANIMA_SDK_CONNECT_TIMEOUT_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                options.connect_timeout_ms = parsed;
            }
        }
        if let Ok(value) = std::env::var("ANIMA_SDK_MAX_RETRIES") {
            if let Ok(parsed) = value.parse::<u32>() {
                options.max_retries = parsed;
            }
        }
        if let Ok(value) = std::env::var("ANIMA_SDK_RETRY_BACKOFF_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                options.retry_backoff_ms = parsed;
            }
        }
        if let Ok(value) = std::env::var("ANIMA_SDK_RETRY_BACKOFF_CAP_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                options.retry_backoff_cap_ms = parsed;
            }
        }
        options
    }

    /// 计算第 `attempt` 次重试（1-based）前的等待时长（毫秒）
    ///
    /// 指数退避：`retry_backoff_ms * 2^(attempt-1)`，以 `retry_backoff_cap_ms` 为上限
    pub fn backoff_delay_ms(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let exp = attempt.saturating_sub(1).min(20);
        let raw = self.retry_backoff_ms.saturating_mul(1u64 << exp);
        raw.min(self.retry_backoff_cap_ms)
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    pub base_url: String,
    pub directory: Option<String>,
    pub http_client: HttpClient,
    pub options: ClientOptions,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_options(base_url, ClientOptions::default())
    }

    pub fn with_options(base_url: impl Into<String>, options: ClientOptions) -> Self {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_millis(options.request_timeout_ms))
            .connect_timeout(Duration::from_millis(options.connect_timeout_ms))
            .build()
            .unwrap_or_else(|_| HttpClient::new());
        Self {
            base_url: base_url.into(),
            directory: None,
            http_client,
            options,
        }
    }

    pub fn with_directory(mut self, directory: impl Into<String>) -> Self {
        self.directory = Some(directory.into());
        self
    }
}

impl From<&SdkConfig> for ClientOptions {
    fn from(c: &SdkConfig) -> Self {
        Self {
            request_timeout_ms: c.request_timeout_ms,
            connect_timeout_ms: c.connect_timeout_ms,
            max_retries: c.max_retries,
            retry_backoff_ms: c.retry_backoff_ms,
            retry_backoff_cap_ms: c.retry_backoff_cap_ms,
        }
    }
}
