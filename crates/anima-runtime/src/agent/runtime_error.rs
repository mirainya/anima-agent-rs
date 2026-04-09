use super::core::RuntimeErrorInfo;

pub(crate) fn classify_runtime_error(
    error: Option<&str>,
    fallback_stage: Option<&'static str>,
) -> RuntimeErrorInfo {
    let raw = error.unwrap_or("Unknown runtime error");
    let raw_lower = raw.to_ascii_lowercase();
    let is_session_create_stage = fallback_stage == Some("session_create");
    let is_plan_execute_stage = fallback_stage == Some("plan_execute");
    let looks_like_session_transport_error = raw_lower.contains("http transport error")
        || raw_lower.contains("error sending request")
        || raw_lower.contains("/session)")
        || raw_lower.contains("/session ")
        || raw_lower.ends_with("/session")
        || raw_lower.contains("/session?");
    let looks_like_upstream_stream_error = raw_lower.contains("empty_stream")
        || raw_lower.contains("upstream stream closed before first payload")
        || raw_lower.contains("stream disconnected before completion")
        || raw_lower.contains("stream closed before response.completed");
    let looks_like_upstream_timeout = raw_lower.contains("request timeout")
        || raw_lower.contains("408 request timeout")
        || raw_lower.contains("timed out")
        || raw_lower.contains("timeout");

    if raw.contains("OpenCode session")
        || raw.contains("no ID returned")
        || raw.contains("Failed to create session")
        || raw_lower.contains("create session")
        || raw_lower.contains("session-create")
        || (is_session_create_stage && looks_like_session_transport_error)
    {
        return RuntimeErrorInfo {
            code: "session_create_failed",
            stage: "session_create",
            user_message: "无法创建上游会话，请确认 opencode-server 是否正常运行。".into(),
            internal_message: raw.to_string(),
        };
    }

    if is_plan_execute_stage && looks_like_upstream_timeout {
        return RuntimeErrorInfo {
            code: "upstream_timeout",
            stage: "plan_execute",
            user_message: "上游模型响应超时，请稍后重试。".into(),
            internal_message: raw.to_string(),
        };
    }

    if is_plan_execute_stage && looks_like_upstream_stream_error {
        return RuntimeErrorInfo {
            code: "upstream_stream_failed",
            stage: "plan_execute",
            user_message: "上游模型流式响应异常中断，请稍后重试或检查代理服务状态。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Worker pool is not running") || raw.contains("Worker is not running") {
        return RuntimeErrorInfo {
            code: "worker_unavailable",
            stage: "worker_pool",
            user_message: "当前执行器未就绪，暂时无法处理请求。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Worker is busy") || raw.contains("No available worker") {
        return RuntimeErrorInfo {
            code: "worker_capacity_exhausted",
            stage: "worker_pool",
            user_message: "当前执行队列繁忙，请稍后再试。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Missing required fields")
        || raw.contains("Missing query")
        || raw.contains("Missing transform data")
    {
        return RuntimeErrorInfo {
            code: "invalid_task_payload",
            stage: fallback_stage.unwrap_or("task_execution"),
            user_message: "运行时生成了无效任务，请检查主链路任务构建逻辑。".into(),
            internal_message: raw.to_string(),
        };
    }

    if raw.contains("Unknown task type") {
        return RuntimeErrorInfo {
            code: "unknown_task_type",
            stage: fallback_stage.unwrap_or("task_execution"),
            user_message: "运行时生成了未支持的任务类型。".into(),
            internal_message: raw.to_string(),
        };
    }

    RuntimeErrorInfo {
        code: "task_execution_failed",
        stage: fallback_stage.unwrap_or("task_execution"),
        user_message: "任务执行失败，请查看运行时事件获取详细原因。".into(),
        internal_message: raw.to_string(),
    }
}
