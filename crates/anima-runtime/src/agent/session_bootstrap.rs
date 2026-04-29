use serde_json::{json, Value};

use crate::bus::InboundMessage;

use super::core::CoreAgent;
use super::runtime_error::{
    classify_runtime_error, RuntimeError, RuntimeErrorKind, RuntimeErrorStage,
};
use super::types::{make_task, MakeTask, TaskResult};

impl CoreAgent {
    pub(crate) fn existing_opencode_session_id(&self, key: &str) -> Option<String> {
        self.memory
            .lock()
            .get(key)
            .and_then(|ctx| ctx.session_id.clone())
    }

    pub(crate) fn request_new_opencode_session(
        &self,
        inbound_msg: &InboundMessage,
        key: &str,
    ) -> Result<TaskResult, RuntimeError> {
        let task = make_task(MakeTask {
            trace_id: Some(inbound_msg.id.clone()),
            task_type: "session-create".into(),
            payload: Some(json!({})),
            ..Default::default()
        });
        self.emitter.publish(
            "worker_task_assigned",
            inbound_msg,
            json!({
                "memory_key": key,
                "plan_type": "session-create",
                "task_id": task.id,
                "task_type": task.task_type,
                "task_summary": "为当前 job 创建上游会话",
                "task_preview": "创建新的上游会话",
            }),
        );
        self.worker_pool.submit_task(task).recv().map_err(|error| {
            let internal_message = format!("Failed to receive session-create result: {error}");
            let error_info = classify_runtime_error(
                Some(internal_message.as_str()),
                Some(RuntimeErrorStage::SessionCreate.as_str()),
            );
            RuntimeError::new(
                match error_info.code {
                    "worker_unavailable" => RuntimeErrorKind::WorkerUnavailable,
                    "worker_capacity_exhausted" => RuntimeErrorKind::WorkerCapacityExhausted,
                    _ => RuntimeErrorKind::SessionCreateFailed,
                },
                match error_info.stage {
                    "worker_pool" => RuntimeErrorStage::WorkerPool,
                    _ => RuntimeErrorStage::SessionCreate,
                },
                internal_message,
            )
        })
    }

    pub(crate) fn extract_created_opencode_session_id(
        &self,
        result: &TaskResult,
    ) -> Result<String, RuntimeError> {
        if result.status != "success" {
            let error_text = result.error.clone().unwrap_or_else(|| {
                format!("Failed to create session: task status={}", result.status)
            });
            let error_info = classify_runtime_error(
                Some(error_text.as_str()),
                Some(RuntimeErrorStage::SessionCreate.as_str()),
            );
            return Err(RuntimeError::new(
                match error_info.code {
                    "worker_unavailable" => RuntimeErrorKind::WorkerUnavailable,
                    "worker_capacity_exhausted" => RuntimeErrorKind::WorkerCapacityExhausted,
                    _ => RuntimeErrorKind::SessionCreateFailed,
                },
                match error_info.stage {
                    "worker_pool" => RuntimeErrorStage::WorkerPool,
                    _ => RuntimeErrorStage::SessionCreate,
                },
                error_text,
            ));
        }

        if let Some(error_text) = result.error.clone() {
            let error_info = classify_runtime_error(
                Some(error_text.as_str()),
                Some(RuntimeErrorStage::SessionCreate.as_str()),
            );
            return Err(RuntimeError::new(
                match error_info.code {
                    "worker_unavailable" => RuntimeErrorKind::WorkerUnavailable,
                    "worker_capacity_exhausted" => RuntimeErrorKind::WorkerCapacityExhausted,
                    _ => RuntimeErrorKind::SessionCreateFailed,
                },
                match error_info.stage {
                    "worker_pool" => RuntimeErrorStage::WorkerPool,
                    _ => RuntimeErrorStage::SessionCreate,
                },
                error_text,
            ));
        }

        result
            .result
            .as_ref()
            .and_then(|value| value.get("opencode-session-id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| {
                RuntimeError::new(
                    RuntimeErrorKind::SessionCreateFailed,
                    RuntimeErrorStage::SessionCreate,
                    "Failed to create session: no ID returned",
                )
            })
    }

    pub(crate) fn store_opencode_session_id(&self, key: &str, session_id: &str) {
        self.memory.lock().entry(key.to_string()).and_modify(|ctx| {
            ctx.session_id = Some(session_id.to_string());
        });
    }

    /// 获取已有的 SDK 会话 ID，若不存在则通过 WorkerPool 创建新会话
    pub(crate) fn get_or_create_opencode_session(
        &self,
        inbound_msg: &InboundMessage,
        key: &str,
    ) -> Result<String, RuntimeError> {
        if let Some(existing_id) = self.existing_opencode_session_id(key) {
            return Ok(existing_id);
        }

        let result = self.request_new_opencode_session(inbound_msg, key)?;
        let session_id = self.extract_created_opencode_session_id(&result)?;
        self.store_opencode_session_id(key, &session_id);
        Ok(session_id)
    }
}
