use anima_sdk::{facade::Client as SdkClient, messages, sessions};
use serde_json::Value;

pub trait TaskExecutor: Send + Sync {
    fn send_prompt(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String>;
    fn create_session(&self, client: &SdkClient) -> Result<Value, String>;
}

#[derive(Debug, Default)]
pub struct SdkTaskExecutor;

impl TaskExecutor for SdkTaskExecutor {
    fn send_prompt(
        &self,
        client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        messages::send_prompt(client, session_id, content, None).map_err(|err| err.to_string())
    }

    fn create_session(&self, client: &SdkClient) -> Result<Value, String> {
        sessions::create_session(client, None).map_err(|err| err.to_string())
    }
}
