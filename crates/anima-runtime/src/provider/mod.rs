pub mod anthropic;
pub mod error;
pub mod opencode;
pub mod types;

pub use anthropic::AnthropicProvider;
pub use error::{ProviderError, ProviderErrorKind};
pub use opencode::OpenCodeProvider;
pub use types::{ChatMessage, ChatRequest, ChatResponse, ChatRole, StopReason, Usage};

use crate::streaming::types::StreamEvent;
use serde_json::Value;

pub type ChatStreamLine = Result<StreamEvent, ProviderError>;
pub type ChatStream = Box<dyn Iterator<Item = ChatStreamLine>>;

pub trait Provider: Send + Sync {
    fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError>;

    fn chat_stream(&self, _req: ChatRequest) -> Result<ChatStream, ProviderError> {
        Err(ProviderError::internal("streaming not supported"))
    }

    fn create_session(&self) -> Result<Value, ProviderError> {
        Err(ProviderError::internal("create_session not supported"))
    }

    fn label(&self) -> &str {
        ""
    }
}
