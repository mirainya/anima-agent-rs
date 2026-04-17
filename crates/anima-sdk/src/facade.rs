use reqwest::blocking::Client as HttpClient;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOptions {
    pub request_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            request_timeout_ms: 600_000,
            connect_timeout_ms: 60_000,
            max_retries: 1,
            retry_backoff_ms: 250,
        }
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
