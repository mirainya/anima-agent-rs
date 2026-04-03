use reqwest::blocking::Client as HttpClient;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Client {
    pub base_url: String,
    pub directory: Option<String>,
    pub http_client: HttpClient,
}

/// 默认超时配置
const DEFAULT_SOCKET_TIMEOUT_MS: u64 = 600_000;
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 60_000;

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_millis(DEFAULT_SOCKET_TIMEOUT_MS))
            .connect_timeout(Duration::from_millis(DEFAULT_CONNECT_TIMEOUT_MS))
            .build()
            .unwrap_or_else(|_| HttpClient::new());
        Self {
            base_url: base_url.into(),
            directory: None,
            http_client,
        }
    }

    pub fn with_directory(mut self, directory: impl Into<String>) -> Self {
        self.directory = Some(directory.into());
        self
    }
}
