use serde_json::json;

use crate::bus::InboundMessage;
use crate::messages::types::ContentBlock;
use crate::provider::{ChatMessage, ChatRequest, ChatRole};
use crate::tasks::SubtaskBlockedReason;

use super::context_types::ExecutionContext;
use super::core::CoreAgent;

impl CoreAgent {
    pub(crate) fn try_llm_resolve_blocked(
        &self,
        inbound_msg: &InboundMessage,
        exec_ctx: &ExecutionContext,
        reason: &SubtaskBlockedReason,
    ) -> Option<String> {
        let reason_desc = match reason {
            SubtaskBlockedReason::MissingParameter { name, description } => {
                format!("缺少参数 `{name}`: {description}")
            }
            SubtaskBlockedReason::MissingContext { what_needed } => {
                format!("缺少上下文: {what_needed}")
            }
            SubtaskBlockedReason::NeedsDecision { reason } => {
                format!("需要决策: {reason}")
            }
            SubtaskBlockedReason::MultipleOptions { options, prompt } => {
                format!("多选项: {prompt}\n选项: {}", options.join(", "))
            }
        };

        let prompt = self
            .prompts
            .read()
            .escalation_resolve
            .replace("{request}", &inbound_msg.content)
            .replace("{reason}", &reason_desc);

        let chat_request = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: vec![ContentBlock::Text { text: prompt }],
            }],
            metadata: json!({ "session_id": exec_ctx.opencode_session_id }),
            ..Default::default()
        };
        let response = self.provider.chat(chat_request).ok()?;

        let text = response
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("")
            .trim();

        if text.is_empty() || text.contains("CANNOT_RESOLVE") {
            None
        } else {
            Some(text.to_string())
        }
    }
}
