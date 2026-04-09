use anima_runtime::agent::TaskExecutor;
use anima_runtime::agent::{make_task, MakeTask, WorkerPool};
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct StreamingApiCallExecutor {
    sse_sequences: Vec<Vec<String>>,
    sync_response: Value,
    streaming_call_count: AtomicUsize,
    sync_call_count: AtomicUsize,
}

impl StreamingApiCallExecutor {
    fn new(sse_sequences: Vec<Vec<String>>, sync_response: Value) -> Self {
        Self {
            sse_sequences,
            sync_response,
            streaming_call_count: AtomicUsize::new(0),
            sync_call_count: AtomicUsize::new(0),
        }
    }

    fn streaming_calls(&self) -> usize {
        self.streaming_call_count.load(Ordering::SeqCst)
    }

    fn sync_calls(&self) -> usize {
        self.sync_call_count.load(Ordering::SeqCst)
    }
}

impl TaskExecutor for StreamingApiCallExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        self.sync_call_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.sync_response.clone())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "worker-stream-session"}))
    }

    fn send_prompt_streaming(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Box<dyn Iterator<Item = Result<String, String>>>, String> {
        let idx = self.streaming_call_count.fetch_add(1, Ordering::SeqCst);
        let lines = self
            .sse_sequences
            .get(idx)
            .cloned()
            .ok_or_else(|| "no more streaming mock sequences".to_string())?;
        Ok(Box::new(lines.into_iter().map(Ok)))
    }
}

fn sse(data: &str) -> String {
    format!("data: {data}")
}

fn make_pool(
    executor: Arc<dyn TaskExecutor>,
    events: Arc<Mutex<Vec<(String, Value)>>>,
) -> WorkerPool {
    let client = SdkClient::new("http://127.0.0.1:9711");
    WorkerPool::new(client, executor, Some(1), None, Some(200)).with_runtime_event_publisher(
        Arc::new(move |_trace_id, event, payload| {
            events.lock().unwrap().push((event.to_string(), payload));
        }),
    )
}

fn collect_event_names(events: &[(String, Value)]) -> Vec<String> {
    events.iter().map(|(event, _)| event.clone()).collect()
}

#[test]
fn worker_streams_orchestration_api_call_with_runtime_events() {
    let executor = Arc::new(StreamingApiCallExecutor::new(
        vec![vec![
            sse(r#"{"type":"message_start","message":{"id":"msg_ws_1"}}"#),
            sse(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"worker"}}"#,
            ),
            sse(r#"{"type":"content_block_stop","index":0}"#),
            sse(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#),
            sse(r#"{"type":"message_stop"}"#),
        ]],
        json!({"content": [{"type": "text", "text": "blocking fallback"}]}),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let pool = make_pool(executor.clone(), events.clone());
    pool.start();

    let task = make_task(MakeTask {
        trace_id: Some("trace-streaming-worker".into()),
        task_type: "api-call".into(),
        payload: Some(json!({
            "content": "stream this orchestration task",
            "opencode-session-id": "sess-stream-1",
            "parent_job_id": "job-main-1",
            "plan_id": "plan-1",
            "subtask_id": "subtask-1",
            "subtask_name": "implement-frontend",
            "original_task_type": "frontend"
        })),
        metadata: Some(json!({
            "orchestration_subtask": true,
            "streaming_observable": true,
        })),
        ..Default::default()
    });

    let result = pool.submit_task(task).recv().unwrap();
    assert_eq!(result.status, "success");
    assert_eq!(executor.streaming_calls(), 1);
    assert_eq!(executor.sync_calls(), 0);
    assert_eq!(result.result.as_ref().unwrap()["id"], "msg_ws_1");
    assert_eq!(
        result.result.as_ref().unwrap()["content"][0]["text"],
        "Hello worker"
    );

    let events = events.lock().unwrap().clone();
    let names = collect_event_names(&events);
    assert!(names.contains(&"worker_api_call_started".to_string()));
    assert!(names.contains(&"sdk_send_prompt_started".to_string()));
    assert!(names.contains(&"worker_api_call_streaming_started".to_string()));
    assert!(names.contains(&"sdk_stream_message_started".to_string()));
    assert!(names.contains(&"sdk_stream_content_block_delta".to_string()));
    assert!(names.contains(&"sdk_stream_message_stopped".to_string()));
    assert!(names.contains(&"sdk_send_prompt_finished".to_string()));
    assert!(names.contains(&"worker_api_call_streaming_finished".to_string()));
    assert!(names.contains(&"worker_api_call_finished".to_string()));
    assert!(names.contains(&"worker_task_cleanup_finished".to_string()));

    let delta_payloads: Vec<_> = events
        .iter()
        .filter(|(event, _)| event == "sdk_stream_content_block_delta")
        .map(|(_, payload)| payload.clone())
        .collect();
    assert!(delta_payloads
        .iter()
        .any(|payload| payload["delta_kind"] == "text_delta"));
    assert!(delta_payloads
        .iter()
        .any(|payload| payload["text_delta"] == "Hello "));
}

#[test]
fn worker_keeps_non_orchestration_api_call_on_blocking_path() {
    let executor = Arc::new(StreamingApiCallExecutor::new(
        vec![vec![sse(
            r#"{"type":"message_start","message":{"id":"unused"}}"#,
        )]],
        json!({"content": [{"type": "text", "text": "blocking response"}]}),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let pool = make_pool(executor.clone(), events.clone());
    pool.start();

    let task = make_task(MakeTask {
        trace_id: Some("trace-blocking-worker".into()),
        task_type: "api-call".into(),
        payload: Some(json!({
            "content": "normal blocking api call",
            "opencode-session-id": "sess-blocking-1"
        })),
        metadata: Some(json!({})),
        ..Default::default()
    });

    let result = pool.submit_task(task).recv().unwrap();
    assert_eq!(result.status, "success");
    assert_eq!(executor.streaming_calls(), 0);
    assert_eq!(executor.sync_calls(), 1);

    let names = collect_event_names(&events.lock().unwrap());
    assert!(names.contains(&"worker_api_call_started".to_string()));
    assert!(names.contains(&"sdk_send_prompt_started".to_string()));
    assert!(names.contains(&"sdk_send_prompt_finished".to_string()));
    assert!(names.contains(&"worker_api_call_finished".to_string()));
    assert!(!names.iter().any(|name| name.starts_with("sdk_stream_")));
    assert!(!names.contains(&"worker_api_call_streaming_started".to_string()));
}

#[test]
fn worker_reports_streaming_failures_and_tool_input_deltas() {
    let executor = Arc::new(StreamingApiCallExecutor::new(
        vec![vec![
            sse(r#"{"type":"message_start","message":{"id":"msg_ws_2"}}"#),
            sse(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"bash","input":{}}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\""}}"#,
            ),
            sse(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":\"ls\"}"}}"#,
            ),
            sse(r#"{"type":"error","error":{"type":"overloaded","message":"server busy"}}"#),
        ]],
        json!({"content": [{"type": "text", "text": "blocking response"}]}),
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let pool = make_pool(executor.clone(), events.clone());
    pool.start();

    let task = make_task(MakeTask {
        trace_id: Some("trace-streaming-failure".into()),
        task_type: "api-call".into(),
        payload: Some(json!({
            "content": "stream this failing orchestration task",
            "opencode-session-id": "sess-stream-fail",
            "parent_job_id": "job-main-2",
            "plan_id": "plan-2",
            "subtask_id": "subtask-2",
            "subtask_name": "implement-backend",
            "original_task_type": "backend"
        })),
        metadata: Some(json!({
            "orchestration_subtask": true,
            "streaming_observable": true,
        })),
        ..Default::default()
    });

    let result = pool.submit_task(task).recv().unwrap();
    assert_eq!(result.status, "failure");
    assert!(result
        .error
        .as_deref()
        .unwrap_or("")
        .contains("server busy"));
    assert_eq!(executor.streaming_calls(), 1);
    assert_eq!(executor.sync_calls(), 0);

    let events = events.lock().unwrap().clone();
    let names = collect_event_names(&events);
    assert!(names.contains(&"sdk_stream_content_block_started".to_string()));
    assert!(names.contains(&"sdk_stream_content_block_delta".to_string()));
    assert!(names.contains(&"sdk_send_prompt_failed".to_string()));
    assert!(names.contains(&"worker_api_call_streaming_failed".to_string()));
    assert!(names.contains(&"worker_api_call_failed".to_string()));

    let tool_delta = events
        .iter()
        .find(|(event, payload)| {
            event == "sdk_stream_content_block_delta" && payload["delta_kind"] == "input_json_delta"
        })
        .map(|(_, payload)| payload.clone())
        .expect("expected tool input delta event");
    assert_eq!(tool_delta["partial_json"], "{\"command\"");
}
