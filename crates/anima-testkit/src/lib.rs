use anima_runtime::agent::runtime_error::RuntimeError;
use anima_runtime::agent::executor::TaskExecutor;
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct FakeExecutor {
    session_id: String,
    prompt_prefix: String,
    fail_prompts_with: Option<String>,
    fail_session_create_with: Option<String>,
}

impl Default for FakeExecutor {
    fn default() -> Self {
        Self {
            session_id: "fake-session-1".into(),
            prompt_prefix: "reply".into(),
            fail_prompts_with: None,
            fail_session_create_with: None,
        }
    }
}

impl FakeExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }

    pub fn with_prompt_prefix(mut self, prompt_prefix: impl Into<String>) -> Self {
        self.prompt_prefix = prompt_prefix.into();
        self
    }

    pub fn fail_prompts_with(mut self, message: impl Into<String>) -> Self {
        self.fail_prompts_with = Some(message.into());
        self
    }

    pub fn fail_session_create_with(mut self, message: impl Into<String>) -> Self {
        self.fail_session_create_with = Some(message.into());
        self
    }
}

impl TaskExecutor for FakeExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, RuntimeError> {
        if let Some(error) = &self.fail_prompts_with {
            return Err(error.clone().into());
        }
        Ok(json!({
            "content": format!("{}[{}]: {}", self.prompt_prefix, session_id, content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, RuntimeError> {
        if let Some(error) = &self.fail_session_create_with {
            return Err(error.clone().into());
        }
        Ok(json!({"id": self.session_id}))
    }
}

pub fn fake_prompt_response(session_id: &str, text: &str) -> Value {
    json!({"content": format!("reply[{session_id}]: {text}")})
}
