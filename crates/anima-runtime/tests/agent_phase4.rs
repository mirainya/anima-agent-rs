use anima_runtime::agent::{Agent, QuestionAnswerInput, TaskExecutor, WorkerAgent, WorkerPool};
use anima_runtime::agent_orchestrator::{AgentOrchestrator, OrchestratorConfig};
use anima_runtime::agent_specialist_pool::SpecialistPool;
use anima_runtime::bus::{make_inbound, Bus, MakeInbound};
use anima_runtime::channel::{start_outbound_dispatch, ChannelRegistry, DispatchStats, TestChannel};
use anima_runtime::Channel;
use anima_sdk::facade::Client as SdkClient;
use serde_json::{json, Value};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Default)]
struct SlowExecutor;

#[derive(Debug, Default)]
struct MockExecutor;

#[derive(Debug, Default)]
struct QuestionFlowExecutor;

#[derive(Debug, Default)]
struct FollowupExecutor;

#[derive(Debug, Default)]
struct OrchestrationQuestionExecutor;

#[derive(Debug, Default)]
struct OrchestrationFollowupExecutor;

#[derive(Debug, Default)]
struct RepeatingUnsatisfiedExecutor;

#[derive(Debug, Default)]
struct RepeatedQuestionExecutor;

impl TaskExecutor for MockExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        Ok(json!({
            "content": format!("reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-1"}))
    }
}

#[derive(Debug, Default)]
struct FailingExecutor;

#[derive(Debug, Default)]
struct QuestionLookingErrorExecutor;

impl TaskExecutor for FailingExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("upstream exploded".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-err"}))
    }
}

impl TaskExecutor for QuestionLookingErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err(r#"{"question":{"id":"fake-question","prompt":"请确认是否继续","options":["继续","取消"]}}"#.into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-question-looking-error"}))
    }
}

impl TaskExecutor for QuestionFlowExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text == "Need continuation" {
            Ok(json!({
                "question": {
                    "id": "question-1",
                    "kind": "input",
                    "prompt": "请选择继续方式",
                    "options": ["继续执行", "取消"]
                }
            }))
        } else {
            Ok(json!({
                "content": format!("reply[{session_id}]: continued with {text}")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-question"}))
    }
}

impl TaskExecutor for FollowupExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text == "Need better completion" {
            Ok(json!({
                "content": "I need more information before I can conclude this task."
            }))
        } else {
            Ok(json!({
                "content": format!("reply[{session_id}]: final completed answer")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-followup"}))
    }
}

impl TaskExecutor for OrchestrationQuestionExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text.contains("[orchestration/") {
            Ok(json!({
                "question": {
                    "id": "orch-question-1",
                    "kind": "input",
                    "prompt": "需要确认 orchestration 是否继续",
                    "options": ["继续 orchestration", "停止"]
                }
            }))
        } else {
            Ok(json!({
                "content": format!("reply[{session_id}]: orchestration continued with {text}")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-orch-question"}))
    }
}

impl TaskExecutor for OrchestrationFollowupExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text.contains("[orchestration/") {
            Ok(json!({
                "content": "I need more information before I can conclude this task."
            }))
        } else {
            Ok(json!({
                "content": format!("reply[{session_id}]: orchestration followup completed")
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-orch-followup"}))
    }
}

impl TaskExecutor for RepeatingUnsatisfiedExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({
            "content": "Need more information before proceeding."
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-repeat"}))
    }
}

impl TaskExecutor for RepeatedQuestionExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        let text = content.as_str().unwrap_or("");
        if text == "Need repeated question" {
            Ok(json!({
                "question": {
                    "id": "question-1",
                    "kind": "input",
                    "prompt": "第一次需要你的确认",
                    "options": ["继续", "停止"]
                }
            }))
        } else {
            Ok(json!({
                "question": {
                    "id": "question-2",
                    "kind": "input",
                    "prompt": "还需要再确认一次",
                    "options": ["继续执行", "取消"]
                }
            }))
        }
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-repeated-question"}))
    }
}

#[derive(Debug, Default)]
struct SessionCreateErrorExecutor;

impl TaskExecutor for SessionCreateErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({"content": "unused"}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Err("create session upstream exploded".into())
    }
}

#[derive(Debug, Default)]
struct MissingSessionIdExecutor;

impl TaskExecutor for MissingSessionIdExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({"content": "unused"}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"ok": true}))
    }
}

#[derive(Debug, Default)]
struct SessionTransportErrorExecutor;

impl TaskExecutor for SessionTransportErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Ok(json!({"content": "unused"}))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Err("HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)".into())
    }
}

#[derive(Debug, Default)]
struct UpstreamStreamErrorExecutor;

impl TaskExecutor for UpstreamStreamErrorExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("empty_stream: upstream stream closed before first payload".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-stream-fail"}))
    }
}

#[derive(Debug, Default)]
struct UpstreamTimeoutExecutor;

impl TaskExecutor for UpstreamTimeoutExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        _session_id: &str,
        _content: Value,
    ) -> Result<Value, String> {
        Err("HTTP 408 Request Timeout: stream disconnected before completion: stream closed before response.completed".into())
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        Ok(json!({"id": "mock-session-timeout"}))
    }
}

impl TaskExecutor for SlowExecutor {
    fn send_prompt(
        &self,
        _client: &SdkClient,
        session_id: &str,
        content: Value,
    ) -> Result<Value, String> {
        thread::sleep(Duration::from_millis(200));
        Ok(json!({
            "content": format!("slow-reply[{session_id}]: {}", content.as_str().unwrap_or(""))
        }))
    }

    fn create_session(&self, _client: &SdkClient) -> Result<Value, String> {
        thread::sleep(Duration::from_millis(50));
        Ok(json!({"id": "slow-session-1"}))
    }
}

#[test]
fn worker_executes_api_call_task() {
    let worker = Arc::new(WorkerAgent::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        None,
    ));
    worker.start();

    let rx = worker.submit_task(anima_runtime::agent::make_task(
        anima_runtime::agent::MakeTask {
            task_type: "api-call".into(),
            payload: Some(json!({
                "opencode-session-id": "session-123",
                "content": "Hello"
            })),
            ..Default::default()
        },
    ), None);

    let result = rx.recv().unwrap();
    assert_eq!(result.status, "success");
    assert_eq!(
        result.result.unwrap()["content"],
        "reply[session-123]: Hello"
    );
}

#[test]
fn worker_pool_submits_task_to_available_worker() {
    let pool = WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(50),
    );
    pool.start();

    let rx = pool.submit_task(anima_runtime::agent::make_task(
        anima_runtime::agent::MakeTask {
            task_type: "session-create".into(),
            payload: Some(json!({})),
            ..Default::default()
        },
    ));

    let result = rx.recv().unwrap();
    assert_eq!(result.status, "success");
    assert_eq!(
        result.result.unwrap()["opencode-session-id"],
        "mock-session-1"
    );
}

#[test]
fn agent_processes_messages_and_publishes_outbound() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "Hello, world!".into(),
        ..Default::default()
    }));

    let mut outbound = None;
    for _ in 0..20 {
        if let Some(msg) = bus
            .outbound_receiver()
            .recv_timeout(Duration::from_millis(50))
            .ok()
        {
            outbound = Some(msg);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    let outbound = outbound.expect("expected outbound message");
    assert_eq!(outbound.channel, "test");
    assert_eq!(outbound.reply_target.as_deref(), Some("user-1"));
    assert_eq!(outbound.content, "reply[mock-session-1]: Hello, world!");

    agent.stop();
}

#[test]
fn agent_reuses_created_session_for_same_chat() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "First".into(),
        ..Default::default()
    }));
    let _ = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "Second".into(),
        ..Default::default()
    }));
    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert_eq!(outbound.content, "reply[mock-session-1]: Second");
    assert!(agent.status().running);
    assert_eq!(agent.status().core.sessions_count, 1);
    assert_eq!(agent.status().core.context_status, "running");
    assert!(agent.status().core.metrics.counters["messages_received"] >= 2);
    assert!(agent.status().core.metrics.counters["messages_processed"] >= 2);

    agent.stop();
}

#[test]
fn channel_to_agent_to_dispatch_to_channel_round_trip_succeeds() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let channel = Arc::new(TestChannel::new("test"));
    channel.start();
    registry.register(channel.clone(), None);

    let dispatch_handle = start_outbound_dispatch(bus.clone(), registry.clone(), stats.clone());

    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-1".into()),
        chat_id: Some("chat-1".into()),
        content: "Hello, world!".into(),
        ..Default::default()
    }));

    let mut sent = Vec::new();
    for _ in 0..20 {
        sent = channel.sent_messages();
        if !sent.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, "user-1");
    assert_eq!(sent[0].message, "reply[mock-session-1]: Hello, world!");
    assert_eq!(sent[0].opts.stage.as_deref(), Some("final"));

    let status = agent.status();
    assert!(status.running);
    assert_eq!(status.core.sessions_count, 1);
    assert_eq!(status.core.context_status, "running");

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.channel_not_found, 0);

    agent.stop();
    bus.close();
    dispatch_handle.join().unwrap();
}

#[test]
fn agent_publishes_error_response_when_worker_execution_fails() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(FailingExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-err".into()),
        chat_id: Some("chat-err".into()),
        content: "Boom".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert_eq!(outbound.channel, "test");
    assert_eq!(outbound.reply_target.as_deref(), Some("user-err"));
    assert_eq!(outbound.stage, "final");
    assert!(outbound.content.starts_with("Error [task_execution_failed]: "));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("message_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "task_execution_failed");
                assert_eq!(payload["error_stage"], "plan_execute");
                assert!(payload["error"].as_str().unwrap_or("").contains("upstream exploded"));
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected message_failed runtime event");

    let status = agent.status();
    assert!(status.running);
    assert!(status.core.metrics.counters["messages_failed"] >= 1);
    assert!(status.core.metrics.counters["tasks_failed"] >= 1);
    assert_eq!(status.core.failures.counts_by_error_code["task_execution_failed"], 1);
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "task_execution_failed");
    assert_eq!(last_failure.error_stage, "plan_execute");
    assert_eq!(last_failure.channel, "test");
    assert_eq!(last_failure.chat_id.as_deref(), Some("chat-err"));
    assert!(last_failure.internal_message.contains("upstream exploded"));

    agent.stop();
}

#[test]
fn agent_classifies_upstream_stream_error_during_plan_execution() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(UpstreamStreamErrorExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-stream-err".into()),
        chat_id: Some("chat-stream-err".into()),
        content: "Explain HTTP".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [upstream_stream_failed]: "));
    assert!(outbound.content.contains("流式响应异常中断"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("message_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "upstream_stream_failed");
                assert_eq!(payload["error_stage"], "plan_execute");
                assert_eq!(payload["error"], "empty_stream: upstream stream closed before first payload");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected message_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "upstream_stream_failed");
    assert_eq!(last_failure.error_stage, "plan_execute");
    assert_eq!(last_failure.internal_message, "empty_stream: upstream stream closed before first payload");

    agent.stop();
}

#[test]
fn agent_classifies_upstream_timeout_during_plan_execution() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(UpstreamTimeoutExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-timeout-err".into()),
        chat_id: Some("chat-timeout-err".into()),
        content: "Explain HTTP".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [upstream_timeout]: "));
    assert!(outbound.content.contains("响应超时"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("message_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "upstream_timeout");
                assert_eq!(payload["error_stage"], "plan_execute");
                assert!(payload["error"].as_str().unwrap_or("").contains("Request Timeout"));
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected message_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "upstream_timeout");
    assert_eq!(last_failure.error_stage, "plan_execute");
    assert!(last_failure.internal_message.contains("Request Timeout"));

    agent.stop();
}

#[test]
fn agent_submits_question_answer_and_continues_same_session() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(QuestionFlowExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-question".into()),
        chat_id: Some("chat-question".into()),
        content: "Need continuation".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    std::thread::sleep(Duration::from_millis(150));
    let pending = agent.pending_question_for(&job_id).expect("expected pending question");
    assert_eq!(pending.question_id, "question-1");
    assert_eq!(pending.opencode_session_id, "mock-session-question");
    assert_eq!(pending.raw_question["prompt"], "请选择继续方式");
    assert_eq!(pending.raw_question["options"][0], "继续执行");

    let waiting_status = agent.status();
    let waiting_timeline = waiting_status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let waiting_events = waiting_timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(waiting_events.contains(&"upstream_response_observed"));
    assert!(waiting_events.contains(&"requirement_unsatisfied"));
    assert!(waiting_events.contains(&"user_input_required"));
    assert!(waiting_events.contains(&"question_asked"));
    assert!(!waiting_events.contains(&"question_answer_submitted"));
    assert!(!waiting_events.contains(&"question_resolved"));
    assert!(!waiting_events.contains(&"message_completed"));
    let unsatisfied_idx = waiting_events
        .iter()
        .position(|event| *event == "requirement_unsatisfied")
        .unwrap();
    let user_input_required_idx = waiting_events
        .iter()
        .position(|event| *event == "user_input_required")
        .unwrap();
    let asked_idx = waiting_events
        .iter()
        .position(|event| *event == "question_asked")
        .unwrap();
    assert!(unsatisfied_idx < user_input_required_idx);
    assert!(user_input_required_idx < asked_idx);
    let waiting_summary = waiting_status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected waiting execution summary");
    assert_eq!(waiting_summary.status, "waiting_user_input");

    let continued = agent
        .submit_question_answer(
            &job_id,
            QuestionAnswerInput {
                question_id: "question-1".into(),
                source: "user".into(),
                answer_type: "text".into(),
                answer: "继续执行".into(),
            },
        )
        .expect("continuation should succeed");
    assert_eq!(continued.question_id, "question-1");
    assert!(agent.pending_question_for(&job_id).is_none());

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.contains("continued with 继续执行"));

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(events.contains(&"upstream_response_observed"));
    assert!(events.contains(&"question_asked"));
    assert!(events.contains(&"question_answer_submitted"));
    assert!(events.contains(&"question_resolved"));
    assert!(events.contains(&"message_completed"));
    let observed_idx = events.iter().position(|event| *event == "upstream_response_observed").unwrap();
    let asked_idx = events.iter().position(|event| *event == "question_asked").unwrap();
    assert!(observed_idx < asked_idx);
    let asked_payload = timeline
        .iter()
        .find(|event| event.event == "question_asked")
        .map(|event| event.payload.clone())
        .expect("expected question_asked payload");
    assert_eq!(asked_payload["raw_question"]["prompt"], "请选择继续方式");
    assert_eq!(asked_payload["raw_question"]["options"][1], "取消");
    let final_summary = status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected final execution summary");
    assert_eq!(final_summary.status, "success");

    agent.stop();
}

#[test]
fn agent_continuation_can_return_to_waiting_user_input() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(RepeatedQuestionExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-repeat-question".into()),
        chat_id: Some("chat-repeat-question".into()),
        content: "Need repeated question".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    thread::sleep(Duration::from_millis(150));
    let first_pending = agent.pending_question_for(&job_id).expect("expected first pending question");
    assert_eq!(first_pending.question_id, "question-1");

    agent
        .submit_question_answer(
            &job_id,
            QuestionAnswerInput {
                question_id: "question-1".into(),
                source: "user".into(),
                answer_type: "text".into(),
                answer: "继续".into(),
            },
        )
        .expect("continuation should accept first answer");

    thread::sleep(Duration::from_millis(150));
    let pending = agent.pending_question_for(&job_id).expect("expected second pending question");
    assert_eq!(pending.question_id, "question-2");
    assert_eq!(pending.prompt, "还需要再确认一次");

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(events.contains(&"question_answer_submitted"));
    assert!(events.contains(&"question_resolved"));
    assert!(events.contains(&"user_input_required"));
    assert!(events.contains(&"question_asked"));
    assert!(!events.contains(&"message_completed"));

    let answer_submitted_idx = events
        .iter()
        .rposition(|event| *event == "question_answer_submitted")
        .unwrap();
    let resolved_idx = events
        .iter()
        .rposition(|event| *event == "question_resolved")
        .unwrap();
    let user_input_required_idx = events
        .iter()
        .rposition(|event| *event == "user_input_required")
        .unwrap();
    let asked_idx = events
        .iter()
        .rposition(|event| *event == "question_asked")
        .unwrap();
    assert!(answer_submitted_idx < resolved_idx);
    assert!(resolved_idx < user_input_required_idx);
    assert!(user_input_required_idx < asked_idx);

    let resolved_payload = timeline
        .iter()
        .rev()
        .find(|event| event.event == "question_resolved")
        .map(|event| event.payload.clone())
        .expect("expected question_resolved payload");
    assert_eq!(resolved_payload["question_id"], "question-1");
    assert_eq!(resolved_payload["answer_summary"], "继续");
    assert_eq!(resolved_payload["resolution_source"], "user");
    assert_eq!(resolved_payload["opencode_session_id"], "mock-session-repeated-question");

    let waiting_summary = status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected waiting summary after continuation");
    assert_eq!(waiting_summary.status, "waiting_user_input");

    agent.stop();
}

#[test]
fn agent_schedules_followup_before_completion_when_result_is_unsatisfied() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(FollowupExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-followup".into()),
        chat_id: Some("chat-followup".into()),
        content: "Need better completion".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.contains("final completed answer"));

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline.iter().map(|event| event.event.as_str()).collect::<Vec<_>>();
    assert!(events.contains(&"requirement_unsatisfied"));
    assert!(events.contains(&"requirement_followup_scheduled"));
    assert!(events.contains(&"requirement_satisfied"));
    assert!(events.contains(&"message_completed"));
    let unsatisfied_idx = events
        .iter()
        .position(|event| *event == "requirement_unsatisfied")
        .unwrap();
    let followup_scheduled_idx = events
        .iter()
        .position(|event| *event == "requirement_followup_scheduled")
        .unwrap();
    let satisfied_idx = events.iter().position(|event| *event == "requirement_satisfied").unwrap();
    let completed_idx = events.iter().position(|event| *event == "message_completed").unwrap();
    assert!(unsatisfied_idx < followup_scheduled_idx);
    assert!(followup_scheduled_idx < satisfied_idx);
    assert!(satisfied_idx < completed_idx);

    let summary = status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected execution summary");
    assert_eq!(summary.status, "success");

    agent.stop();
}

#[test]
fn agent_exhausts_followup_when_result_repeats_without_progress() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(RepeatingUnsatisfiedExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-repeat".into()),
        chat_id: Some("chat-repeat".into()),
        content: "Repeat until exhausted".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.contains("requirement_followup_exhausted"));

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline.iter().map(|event| event.event.as_str()).collect::<Vec<_>>();
    assert!(events.contains(&"requirement_followup_exhausted"));
    assert!(events.contains(&"message_failed"));
    assert!(!events.contains(&"message_completed"));

    let exhausted_payload = timeline
        .iter()
        .find(|event| event.event == "requirement_followup_exhausted")
        .map(|event| event.payload.clone())
        .expect("expected requirement_followup_exhausted payload");
    assert_eq!(exhausted_payload["attempted_rounds"], 2);
    assert_eq!(exhausted_payload["max_rounds"], 3);
    assert_eq!(exhausted_payload["reason"], "自动 follow-up 得到了重复结果，尚未收敛");
    assert_eq!(
        exhausted_payload["missing_requirements"][0],
        "避免重复前一轮输出，继续给出真正推进结果"
    );
    assert!(exhausted_payload["result_fingerprint"]
        .as_str()
        .unwrap_or("")
        .contains("Needmoreinformationbeforeproceeding."));

    let summary = status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected execution summary");
    assert_eq!(summary.status, "followup_exhausted");

    agent.stop();
}

#[test]
fn agent_orchestration_p2_surfaces_question_and_continues_same_session() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(OrchestrationQuestionExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("user-orch-question".into()),
        chat_id: Some("chat-orch-question".into()),
        content: "create REST API endpoint".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    thread::sleep(Duration::from_millis(200));
    let pending = agent
        .pending_question_for(&job_id)
        .expect("expected orchestration pending question");
    assert_eq!(pending.question_id, "orch-question-1");
    assert_eq!(pending.opencode_session_id, "mock-session-orch-question");
    assert_eq!(pending.raw_question["prompt"], "需要确认 orchestration 是否继续");

    let waiting_status = agent.status();
    let waiting_timeline = waiting_status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let waiting_events = waiting_timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(waiting_events.contains(&"orchestration_selected"));
    assert!(waiting_events.contains(&"orchestration_plan_created"));
    assert!(waiting_events.contains(&"orchestration_subtask_started"));
    assert!(waiting_events.contains(&"orchestration_subtask_completed"));
    assert!(waiting_events.contains(&"question_asked"));
    assert!(!waiting_events.contains(&"orchestration_fallback"));
    assert!(!waiting_events.contains(&"message_completed"));

    let started_payload = waiting_timeline
        .iter()
        .find(|event| event.event == "orchestration_subtask_started")
        .map(|event| event.payload.clone())
        .expect("expected orchestration_subtask_started payload");
    assert_eq!(started_payload["original_task_type"], "design");
    assert_eq!(started_payload["lowered_task_type"], "api-call");
    assert_eq!(started_payload["execution_mode"], "serial");
    assert_eq!(started_payload["result_kind"], "upstream");

    let continued = agent
        .submit_question_answer(
            &job_id,
            QuestionAnswerInput {
                question_id: "orch-question-1".into(),
                source: "user".into(),
                answer_type: "text".into(),
                answer: "继续 orchestration".into(),
            },
        )
        .expect("orchestration continuation should succeed");
    assert_eq!(continued.question_id, "orch-question-1");
    assert!(agent.pending_question_for(&job_id).is_none());

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.contains("orchestration continued with 继续 orchestration"));

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(events.contains(&"question_answer_submitted"));
    assert!(events.contains(&"question_resolved"));
    assert!(events.contains(&"message_completed"));
    let waiting_summary = waiting_status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected waiting orchestration summary");
    assert_eq!(waiting_summary.plan_type, "orchestration-v1");
    assert_eq!(waiting_summary.status, "waiting_user_input");

    let final_summary = status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected final orchestration summary");
    assert_eq!(final_summary.status, "success");

    agent.stop();
}

#[test]
fn agent_orchestration_p2_keeps_followup_contract_after_upstream_result() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(OrchestrationFollowupExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "web".into(),
        sender_id: Some("user-orch-followup".into()),
        chat_id: Some("chat-orch-followup".into()),
        content: "create REST API endpoint".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.contains("orchestration followup completed"));

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(events.contains(&"orchestration_selected"));
    assert!(events.contains(&"orchestration_plan_created"));
    assert!(events.contains(&"orchestration_subtask_started"));
    assert!(events.contains(&"orchestration_subtask_completed"));
    assert!(events.contains(&"requirement_unsatisfied"));
    assert!(events.contains(&"requirement_followup_scheduled"));
    assert!(events.contains(&"requirement_satisfied"));
    assert!(events.contains(&"message_completed"));
    assert!(!events.contains(&"orchestration_fallback"));

    let followup_idx = events
        .iter()
        .position(|event| *event == "requirement_followup_scheduled")
        .unwrap();
    let completed_idx = events
        .iter()
        .position(|event| *event == "message_completed")
        .unwrap();
    assert!(followup_idx < completed_idx);

    let summary = status
        .core
        .recent_execution_summaries
        .iter()
        .rev()
        .find(|item| item.message_id == job_id)
        .expect("expected orchestration followup summary");
    assert_eq!(summary.plan_type, "orchestration-v1");
    assert_eq!(summary.status, "success");

    agent.stop();
}

#[test]
fn agent_failure_does_not_create_pending_question() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(FailingExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-fail".into()),
        chat_id: Some("chat-fail".into()),
        content: "please fail".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    thread::sleep(Duration::from_millis(150));
    assert!(agent.pending_question_for(&job_id).is_none());

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline.iter().map(|event| event.event.as_str()).collect::<Vec<_>>();
    assert!(events.contains(&"message_failed"));
    assert!(!events.contains(&"question_asked"));

    agent.stop();
}

#[test]
fn agent_question_like_error_text_does_not_create_pending_question() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(QuestionLookingErrorExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-question-looking-error".into()),
        chat_id: Some("chat-question-looking-error".into()),
        content: "please fail like question".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    thread::sleep(Duration::from_millis(150));
    assert!(agent.pending_question_for(&job_id).is_none());

    let status = agent.status();
    let timeline = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == job_id)
        .collect::<Vec<_>>();
    let events = timeline.iter().map(|event| event.event.as_str()).collect::<Vec<_>>();
    assert!(events.contains(&"message_failed"));
    assert!(!events.contains(&"question_asked"));

    agent.stop();
}

#[test]
fn agent_emits_runtime_events_and_recent_sessions_in_status() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-obs".into()),
        chat_id: Some("chat-obs".into()),
        content: "Observe me".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(outbound.content, "reply[mock-session-1]: Observe me");

    let mut events = Vec::new();
    for _ in 0..20 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if let Some(event) = msg.payload.get("event").and_then(|v| v.as_str()) {
                events.push(event.to_string());
            }
            if events.iter().any(|e| e == "message_completed") {
                break;
            }
        }
    }

    assert!(events.iter().any(|e| e == "message_received"));
    assert!(events.iter().any(|e| e == "session_ready"));
    assert!(events.iter().any(|e| e == "plan_built"));
    assert!(events.iter().any(|e| e == "worker_task_assigned"));
    assert!(events.iter().any(|e| e == "api_call_started"));
    assert!(events.iter().any(|e| e == "upstream_response_observed"));
    assert!(events.iter().any(|e| e == "message_completed"));

    let status = agent.status();
    assert_eq!(status.core.recent_sessions.len(), 1);
    assert_eq!(status.core.recent_sessions[0].chat_id, "chat-obs");
    assert_eq!(status.core.recent_sessions[0].channel, "test");
    assert_eq!(status.core.recent_sessions[0].session_id.as_deref(), Some("mock-session-1"));
    assert_eq!(status.core.metrics.gauges["sessions_active"], 1);

    let timeline = &status.core.runtime_timeline;
    let timeline_events = timeline
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert!(timeline_events.contains(&"message_received"));
    assert!(timeline_events.contains(&"session_ready"));
    assert!(timeline_events.contains(&"plan_built"));
    assert!(timeline_events.contains(&"worker_task_assigned"));
    assert!(timeline_events.contains(&"api_call_started"));
    assert!(timeline_events.contains(&"upstream_response_observed"));
    assert!(timeline_events.contains(&"message_completed"));

    let message_received_pos = timeline_events.iter().position(|event| *event == "message_received").unwrap();
    let session_ready_pos = timeline_events.iter().position(|event| *event == "session_ready").unwrap();
    let plan_built_pos = timeline_events.iter().position(|event| *event == "plan_built").unwrap();
    let worker_task_assigned_pos = timeline
        .iter()
        .position(|event| {
            event.event == "worker_task_assigned"
                && event.payload.get("task_type").and_then(|value| value.as_str()) == Some("api-call")
        })
        .unwrap();
    let api_call_started_pos = timeline
        .iter()
        .position(|event| {
            event.event == "api_call_started"
                && event.payload.get("task_type").and_then(|value| value.as_str()) == Some("api-call")
        })
        .unwrap();
    let upstream_response_observed_pos = timeline_events.iter().position(|event| *event == "upstream_response_observed").unwrap();
    let message_completed_pos = timeline_events.iter().position(|event| *event == "message_completed").unwrap();
    assert!(message_received_pos < session_ready_pos);
    assert!(session_ready_pos < plan_built_pos);
    assert!(plan_built_pos < worker_task_assigned_pos);
    assert!(worker_task_assigned_pos < api_call_started_pos);
    assert!(api_call_started_pos < upstream_response_observed_pos);
    assert!(upstream_response_observed_pos < message_completed_pos);

    assert_eq!(status.core.recent_execution_summaries.len(), 1);
    let summary = &status.core.recent_execution_summaries[0];
    assert_eq!(summary.plan_type, "single");
    assert_eq!(summary.status, "success");
    assert!(!summary.cache_hit);
    assert!(summary.stages.total_ms >= summary.stages.execute_ms);
    assert!(summary.stages.total_ms >= summary.stages.context_ms);
    assert!(summary.stages.total_ms >= summary.stages.session_ms);
    assert!(summary.stages.total_ms >= summary.stages.classify_ms);

    agent.stop();
}

#[test]
fn agent_preserves_executor_session_create_error_across_failure_surfaces() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(SessionCreateErrorExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-session-create-err".into()),
        chat_id: Some("chat-session-create-err".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(outbound.channel, "test");
    assert_eq!(outbound.reply_target.as_deref(), Some("user-session-create-err"));
    assert_eq!(outbound.stage, "final");
    assert!(outbound.content.starts_with("Error [session_create_failed]: "));
    assert!(outbound.content.contains("无法创建上游会话"));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "session_create_failed");
                assert_eq!(payload["error_stage"], "session_create");
                assert_eq!(payload["error_message"], "create session upstream exploded");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "session_create_failed");
    assert_eq!(last_failure.error_stage, "session_create");
    assert_eq!(last_failure.channel, "test");
    assert_eq!(last_failure.chat_id.as_deref(), Some("chat-session-create-err"));
    assert_eq!(last_failure.internal_message, "create session upstream exploded");
    assert!(!last_failure.internal_message.contains("Unknown runtime error"));

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(timeline_event.payload["error_message"], "create session upstream exploded");
    assert!(!timeline_event.payload["error_message"]
        .as_str()
        .unwrap_or("")
        .contains("Unknown runtime error"));

    agent.stop();
}

#[test]
fn agent_preserves_missing_session_id_error_across_failure_surfaces() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MissingSessionIdExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-missing-session-id".into()),
        chat_id: Some("chat-missing-session-id".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [session_create_failed]: "));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "session_create_failed");
                assert_eq!(payload["error_stage"], "session_create");
                assert_eq!(payload["error_message"], "Failed to create session: no ID returned");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "session_create_failed");
    assert_eq!(last_failure.error_stage, "session_create");
    assert_eq!(last_failure.internal_message, "Failed to create session: no ID returned");
    assert!(!last_failure.internal_message.contains("Unknown runtime error"));

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(timeline_event.payload["error_message"], "Failed to create session: no ID returned");

    agent.stop();
}

#[test]
fn agent_classifies_session_transport_error_as_session_create_failure() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(SessionTransportErrorExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-session-transport-error".into()),
        chat_id: Some("chat-session-transport-error".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [session_create_failed]: "));
    assert!(outbound.content.contains("无法创建上游会话"));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "session_create_failed");
                assert_eq!(payload["error_stage"], "session_create");
                assert_eq!(
                    payload["error_message"],
                    "HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)"
                );
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "session_create_failed");
    assert_eq!(last_failure.error_stage, "session_create");
    assert_eq!(
        last_failure.internal_message,
        "HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)"
    );

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(
        timeline_event.payload["error_message"],
        "HTTP transport error: error sending request for url (http://127.0.0.1:9711/session)"
    );

    agent.stop();
}

#[test]
fn agent_preserves_worker_pool_unavailable_error_during_session_create() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );

    agent.core_agent().process_inbound_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-worker-unavailable".into()),
        chat_id: Some("chat-worker-unavailable".into()),
        content: "Hello".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(outbound.content.starts_with("Error [worker_unavailable]: "));
    assert!(!outbound.content.contains("Unknown runtime error"));

    let mut saw_failed_event = false;
    for _ in 0..12 {
        if let Ok(msg) = bus.internal_receiver().recv_timeout(Duration::from_millis(200)) {
            if msg.payload.get("event").and_then(|v| v.as_str()) == Some("session_create_failed") {
                let payload = msg.payload.get("payload").cloned().unwrap_or_default();
                assert_eq!(payload["error_code"], "worker_unavailable");
                assert_eq!(payload["error_stage"], "worker_pool");
                assert_eq!(payload["error_message"], "Worker pool is not running");
                saw_failed_event = true;
                break;
            }
        }
    }
    assert!(saw_failed_event, "expected session_create_failed runtime event");

    let status = agent.status();
    let last_failure = status.core.failures.last_failure.as_ref().expect("expected last failure snapshot");
    assert_eq!(last_failure.error_code, "worker_unavailable");
    assert_eq!(last_failure.error_stage, "worker_pool");
    assert_eq!(last_failure.internal_message, "Worker pool is not running");
    assert!(!last_failure.internal_message.contains("Unknown runtime error"));

    let timeline_event = status
        .core
        .runtime_timeline
        .iter()
        .find(|event| event.event == "session_create_failed")
        .expect("expected session_create_failed timeline event");
    assert_eq!(timeline_event.payload["error_message"], "Worker pool is not running");

    agent.stop();
}

#[test]
fn agent_exposes_busy_worker_current_task_during_long_execution() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(SlowExecutor)),
    );
    agent.start();

    let inbound = make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-slow".into()),
        chat_id: Some("chat-slow".into()),
        content: "summarize a very long task".into(),
        ..Default::default()
    });
    let job_id = inbound.id.clone();
    agent.process_message(inbound);

    let mut saw_busy = false;
    for _ in 0..20 {
        let status = agent.status();
        if let Some(worker) = status.core.worker_pool.workers.iter().find(|worker| worker.status == "busy") {
            let current_task = worker.current_task.as_ref().expect("busy worker should expose current task");
            assert_eq!(current_task.trace_id, job_id);
            assert!(current_task.started_ms > 0);
            saw_busy = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(saw_busy, "expected a busy worker during slow execution");

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert!(outbound.content.contains("slow-reply[slow-session-1]"));

    agent.stop();
}

#[test]
fn runtime_timeline_uses_subtask_metadata_as_job_identity() {
    let worker_pool = Arc::new(WorkerPool::new(
        SdkClient::new("http://127.0.0.1:9711"),
        Arc::new(MockExecutor),
        Some(2),
        None,
        Some(100),
    ));
    worker_pool.start();
    let specialist_pool = Arc::new(SpecialistPool::new(worker_pool.clone()));
    let orchestrator = AgentOrchestrator::new(worker_pool.clone(), specialist_pool, OrchestratorConfig::default());
    let plan = orchestrator.decompose_task("build a web app", "main-job-1", "main-job-1");
    let subtask = plan.subtasks.values().next().unwrap();

    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("subtask-user".into()),
        chat_id: Some("chat-subtask".into()),
        content: "Run subtask".into(),
        metadata: Some(json!({
            "parent_job_id": subtask.parent_job_id,
            "subtask_id": subtask.id,
            "plan_id": subtask.parent_id,
            "specialist_type": subtask.specialist_type,
        })),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(outbound.content, "reply[mock-session-1]: Run subtask");

    let status = agent.status();
    let subtask_events = status
        .core
        .runtime_timeline
        .iter()
        .filter(|event| event.message_id == subtask.id)
        .collect::<Vec<_>>();
    assert!(!subtask_events.is_empty());
    assert!(subtask_events.iter().any(|event| event.event == "message_received"));
    assert!(subtask_events.iter().any(|event| event.event == "plan_built"));
    assert!(subtask_events.iter().any(|event| event.event == "message_completed"));
    for event in subtask_events {
        assert!(!event.trace_id.is_empty());
        assert_eq!(event.payload["parent_job_id"], "main-job-1");
        assert_eq!(event.payload["subtask_id"], subtask.id);
        assert_eq!(event.payload["plan_id"], subtask.parent_id);
    }

    agent.stop();
    worker_pool.stop();
}

#[test]
fn integration_routes_to_named_channel_when_multiple_channels_are_registered() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let primary = Arc::new(TestChannel::new("test"));
    let secondary = Arc::new(TestChannel::new("test"));
    primary.start();
    secondary.start();
    registry.register(primary.clone(), Some("primary"));
    registry.register(secondary.clone(), Some("secondary"));

    let dispatch_handle = start_outbound_dispatch(bus.clone(), registry.clone(), stats.clone());

    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-multi".into()),
        chat_id: Some("chat-multi".into()),
        content: "Route me".into(),
        metadata: Some(json!({"account-id": "secondary"})),
        ..Default::default()
    }));

    for _ in 0..20 {
        if secondary.sent_messages().len() == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let primary_sent = primary.sent_messages();
    let secondary_sent = secondary.sent_messages();

    assert!(primary_sent.is_empty());
    assert_eq!(secondary_sent.len(), 1);
    assert_eq!(secondary_sent[0].target, "user-multi");
    assert_eq!(secondary_sent[0].message, "reply[mock-session-1]: Route me");
    assert_eq!(secondary_sent[0].opts.stage.as_deref(), Some("final"));

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.channel_not_found, 0);

    agent.stop();
    bus.close();
    dispatch_handle.join().unwrap();
}

#[test]
fn integration_mixed_dispatch_outcomes_do_not_block_successful_messages() {
    let bus = Arc::new(Bus::create());
    let registry = Arc::new(ChannelRegistry::new());
    let stats = Arc::new(DispatchStats::new());

    let ok_channel = Arc::new(TestChannel::new("test"));
    let failing_channel = Arc::new(TestChannel::failing("test"));
    ok_channel.start();
    failing_channel.start();
    registry.register(ok_channel.clone(), Some("ok-account"));
    registry.register(failing_channel.clone(), Some("bad-account"));

    let dispatch_handle = start_outbound_dispatch(bus.clone(), registry.clone(), stats.clone());

    let agent = Agent::create(
        bus.clone(),
        Some(SdkClient::new("http://127.0.0.1:9711")),
        None,
        Some(Arc::new(MockExecutor)),
    );
    agent.start();

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-ok".into()),
        chat_id: Some("chat-ok".into()),
        content: "deliver".into(),
        metadata: Some(json!({"account-id": "ok-account"})),
        ..Default::default()
    }));

    agent.process_message(make_inbound(MakeInbound {
        channel: "test".into(),
        sender_id: Some("user-bad".into()),
        chat_id: Some("chat-bad".into()),
        content: "fail-route".into(),
        metadata: Some(json!({"account-id": "bad-account"})),
        ..Default::default()
    }));

    agent.process_message(make_inbound(MakeInbound {
        channel: "missing".into(),
        sender_id: Some("user-missing".into()),
        chat_id: Some("chat-missing".into()),
        content: "missing-route".into(),
        metadata: Some(json!({"account-id": "missing-account"})),
        ..Default::default()
    }));

    for _ in 0..20 {
        if ok_channel.sent_messages().len() == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let ok_sent = ok_channel.sent_messages();
    assert_eq!(ok_sent.len(), 1);
    assert_eq!(ok_sent[0].target, "user-ok");
    assert_eq!(ok_sent[0].message, "reply[mock-session-1]: deliver");
    assert_eq!(ok_sent[0].opts.stage.as_deref(), Some("final"));

    let snapshot = stats.snapshot();
    assert_eq!(snapshot.dispatched, 1);
    assert_eq!(snapshot.errors, 1);
    assert_eq!(snapshot.channel_not_found, 1);

    agent.stop();
    bus.close();
    dispatch_handle.join().unwrap();
}
