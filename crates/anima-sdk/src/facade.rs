use reqwest::blocking::Client as HttpClient;

#[derive(Debug, Clone)]
pub struct Client {
    pub base_url: String,
    pub directory: Option<String>,
    pub http_client: HttpClient,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            directory: None,
            http_client: HttpClient::new(),
        }
    }

    pub fn with_directory(mut self, directory: impl Into<String>) -> Self {
        self.directory = Some(directory.into());
        self
    }
}
