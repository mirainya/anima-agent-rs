use serde_json::Value;
use thiserror::Error;

pub type JsonMap = serde_json::Map<String, Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiResponse {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<ApiErrorKind>,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiErrorKind {
    BadRequest,
    NotFound,
    ServerError,
    Unknown,
}

#[derive(Debug, Error)]
pub enum AnimaError {
    #[error("Bad request")]
    BadRequest { response: ApiResponse },
    #[error("Resource not found")]
    NotFound { response: ApiResponse },
    #[error("Server error")]
    ServerError { response: ApiResponse },
    #[error("Unknown error")]
    Unknown { response: ApiResponse },
    #[error("Missing required parameters: {0}")]
    MissingRequired(String),
    #[error("Invalid message format. Expected string, {{text: ...}}, or {{parts: [...]}}")]
    InvalidMessageFormat,
    #[error("Invalid level. Must be 'debug', 'info', 'error', or 'warn'")]
    InvalidLogLevel,
    #[error("Invalid response. Must be 'once', 'always', or 'reject'")]
    InvalidPermissionResponse,
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("JSON serialization error: {0}")]
    Json(String),
}

pub type Result<T> = std::result::Result<T, AnimaError>;
