use reqwest::Client as HttpClient;
use std::time::Duration;

use crate::facade::ClientOptions;

#[derive(Debug, Clone)]
pub struct AsyncClient {
    pub base_url: String,
    pub directory: Option<String>,
    pub http_client: HttpClient,
    pub options: ClientOptions,
}

impl AsyncClient {
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
