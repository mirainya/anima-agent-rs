use std::fmt;

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderErrorKind {
    Authentication,
    RateLimit,
    InvalidRequest,
    Timeout,
    StreamFailed,
    Network,
    Internal,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ProviderErrorKind::Internal, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ProviderErrorKind::Timeout, message)
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ProviderError {}

impl From<&str> for ProviderError {
    fn from(message: &str) -> Self {
        Self::internal(message)
    }
}

impl From<String> for ProviderError {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}
