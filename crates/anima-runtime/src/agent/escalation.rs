use serde_json::{json, Value};

use crate::bus::InboundMessage;
use crate::tasks::SubtaskBlockedReason;

use super::core::{CoreAgent, ExecutionContext};

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

        let prompt = format!(
            "你是主调度 Agent。一个子任务执行时遇到阻塞，需要你判断能否从已有信息中推断出答案。\n\n\
             用户原始请求: {}\n\n\
             阻塞原因: {}\n\n\
             如果你能从用户请求中推断出合理答案，请直接给出简短答案。\n\
             如果信息不足无法判断，请只回复: CANNOT_RESOLVE",
            inbound_msg.content, reason_desc
        );

        let content = json!([{"type": "text", "text": prompt}]);
        let result = self
            .executor
            .send_prompt(&self._client, &exec_ctx.opencode_session_id, content)
            .ok()?;

        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .or_else(|| result.get("text").and_then(Value::as_str))
            .unwrap_or("")
            .trim();

        if text.is_empty() || text.contains("CANNOT_RESOLVE") {
            None
        } else {
            Some(text.to_string())
        }
    }
}
