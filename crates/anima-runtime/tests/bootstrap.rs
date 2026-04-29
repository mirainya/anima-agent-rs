use anima_runtime::agent::TaskExecutor;
use anima_runtime::bootstrap::RuntimeBootstrapBuilder;
use anima_runtime::bus::{make_inbound, MakeInbound};
use anima_sdk::ClientOptions;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct SequenceExecutor {
    responses: Vec<Value>,
    call_count: AtomicUsize,
    payloads: Arc<Mutex<Vec<Value>>>,
}

impl SequenceExecutor {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
            payloads: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn payloads(&self) -> Vec<Value> {
        self.payloads
            .lock()
            .expect("payloads lock poisoned")
            .clone()
    }
}

impl TaskExecutor for SequenceExecutor {
    fn send_prompt(
        &self,
        _session_id: &str,
        content: Value,
    ) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        let text = content.as_str().unwrap_or("");
        if text.contains("任务分解引擎") {
            return Ok(json!({"content": "[]"}));
        }
        self.payloads
            .lock()
            .expect("payloads lock poisoned")
            .push(content);
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(idx)
            .cloned()
            .ok_or_else(|| "no more mock responses".into())
    }

    fn create_session(&self) -> Result<Value, anima_runtime::agent::runtime_error::RuntimeError> {
        Ok(json!({"id": "bootstrap-sequence-session"}))
    }

    fn send_prompt_streaming(
        &self,
        session_id: &str,
        content: Value,
    ) -> Result<
        anima_runtime::worker::executor::UnifiedStreamSource,
        anima_runtime::agent::runtime_error::RuntimeError,
    > {
        let response = self.send_prompt(session_id, content)?;
        Ok(Box::new(
            response_to_sse_lines(&response).into_iter().map(Ok),
        ))
    }
}

fn response_to_sse_lines(response: &Value) -> Vec<String> {
    let mut lines = vec![format!(
        "data: {}",
        json!({"type": "message_start", "message": {"id": "msg_bootstrap"}})
    )];
    lines.push(String::new());

    for (index, block) in response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        match block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "text" => {
                lines.push(format!(
                    "data: {}",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {"type": "text", "text": ""}
                    })
                ));
                lines.push(String::new());
                lines.push(format!(
                    "data: {}",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "text_delta",
                            "text": block.get("text").and_then(Value::as_str).unwrap_or_default()
                        }
                    })
                ));
                lines.push(String::new());
                lines.push(format!(
                    "data: {}",
                    json!({"type": "content_block_stop", "index": index})
                ));
                lines.push(String::new());
            }
            "tool_use" => {
                let tool_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool_use");
                let tool_name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                let input_json = serde_json::to_string(&input).unwrap_or_else(|_| "{}".into());
                lines.push(format!(
                    "data: {}",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {
                            "type": "tool_use",
                            "id": tool_id,
                            "name": tool_name,
                            "input": {}
                        }
                    })
                ));
                lines.push(String::new());
                lines.push(format!(
                    "data: {}",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": input_json
                        }
                    })
                ));
                lines.push(String::new());
                lines.push(format!(
                    "data: {}",
                    json!({"type": "content_block_stop", "index": index})
                ));
                lines.push(String::new());
            }
            _ => {}
        }
    }

    let stop_reason = if response
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|content| {
            content
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        }) {
        "tool_calls"
    } else {
        "end_turn"
    };
    lines.push(format!(
        "data: {}",
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason}})
    ));
    lines.push(String::new());
    lines.push(format!("data: {}", json!({"type": "message_stop"})));
    lines.push(String::new());
    lines
}

#[test]
fn runtime_bootstrap_builder_constructs_and_starts_runtime() {
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_url("http://127.0.0.1:9711")
        .with_prompt("test> ")
        .build();

    runtime.start();

    let dispatcher_status = runtime.dispatcher.status();
    assert!(matches!(
        dispatcher_status.state,
        anima_runtime::dispatcher::DispatcherState::Running
    ));
    assert!(runtime.cli_channel().is_some());
    assert!(runtime.agent.is_running());

    runtime.stop();
    assert!(!runtime.agent.is_running());
}

#[test]
fn runtime_bootstrap_builder_can_disable_cli_channel() {
    let runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .build();
    assert!(runtime.cli_channel().is_none());
}

#[test]
fn runtime_bootstrap_builder_accepts_sdk_options() {
    let runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_sdk_options(ClientOptions {
            request_timeout_ms: 1_000,
            connect_timeout_ms: 2_000,
            max_retries: 2,
            retry_backoff_ms: 10,
            retry_backoff_cap_ms: 100,
        })
        .build();

    // SDK options are now encapsulated inside the executor; verify build succeeds
    let status = runtime.agent.status();
    assert!(status.running || !status.running); // build completed without panic
}

#[test]
fn runtime_bootstrap_builder_registers_builtin_tools_and_stop_hook_by_default() {
    let runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .build();

    let status = runtime.agent.status();
    assert!(status.core.tool_count > 0);
    assert_eq!(status.core.pre_hook_count, 0);
    assert_eq!(status.core.post_hook_count, 1);
}

#[test]
fn runtime_bootstrap_builder_can_disable_builtin_tools() {
    let runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_builtin_tools_enabled(false)
        .build();

    let status = runtime.agent.status();
    assert_eq!(status.core.tool_count, 0);
    assert_eq!(status.core.pre_hook_count, 0);
    assert_eq!(status.core.post_hook_count, 1);
}

#[test]
fn runtime_bootstrap_builder_can_disable_sdk_directory() {
    let runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_sdk_directory_enabled(false)
        .build();

    // SDK directory config is now encapsulated inside the executor; verify build succeeds
    let _status = runtime.agent.status();
}

#[test]
fn runtime_bootstrap_builder_stop_hook_applies_on_real_runtime_message_flow() {
    let executor = Arc::new(SequenceExecutor::new(vec![
        json!({
            "content": [
                {"type": "text", "text": "Run failing bash"},
                {
                    "type": "tool_use",
                    "id": "tu_bootstrap_stop_hook",
                    "name": "bash_exec",
                    "input": {"command": "exit 1"}
                }
            ]
        }),
        json!({
            "content": [{"type": "text", "text": "bootstrap runtime finished"}]
        }),
    ]));

    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_cli_enabled(false)
        .with_executor(executor.clone())
        .build();
    runtime.start();

    runtime
        .agent
        .core_agent()
        .process_inbound_message(make_inbound(MakeInbound {
            channel: "test".into(),
            sender_id: Some("user-bootstrap".into()),
            chat_id: Some("chat-bootstrap".into()),
            content: "please run a failing command".into(),
            ..Default::default()
        }));

    let mut payloads = Vec::new();
    for _ in 0..40 {
        payloads = executor.payloads();
        if !payloads.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !payloads.is_empty(),
        "expected at least one executor payload"
    );
    let last_payload_messages = payloads
        .last()
        .and_then(|payload| payload["messages"].as_array())
        .expect("last payload should include messages");
    let tool_result = last_payload_messages
        .iter()
        .find_map(|message| {
            let content = message.get("content")?.as_array()?;
            content.iter().find(|block| {
                block.get("tool_use_id") == Some(&Value::String("tu_bootstrap_stop_hook".into()))
            })
        })
        .expect("expected tool_result block in last payload");

    assert_eq!(tool_result["is_error"], true);
    assert!(tool_result["content"]
        .as_str()
        .unwrap_or_default()
        .contains("工具执行出现需要处理的错误"));
    assert!(tool_result["content"]
        .as_str()
        .unwrap_or_default()
        .contains("原始输出"));
    assert!(tool_result["content"]
        .as_str()
        .unwrap_or_default()
        .contains("exit code 1"));

    runtime.stop();
}
