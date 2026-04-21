//! 方案 A：多轮对话 + 子任务编排 —— 真实后端端到端验证
//!
//! 三幕剧本聚焦 `SdkTaskExecutor` ↔ OpenCode 后端的真实 HTTP 链路，
//! 填补已有测试里 Mock 路径未覆盖的"真实上游"空白。
//!
//! 运行方式（需手动触发，避免 CI 消耗 LLM 配额）：
//!   ```bash
//!   cargo test -p anima-runtime --test e2e_scenario_A -- --ignored --nocapture
//!   ```
//!
//! 前置条件：
//!   - OpenCode 后端运行在 http://127.0.0.1:9711
//!   - 后端已配置可用的 provider / model（默认 provider=opencode）
//!
//! 预期总耗时：~30 秒（≤3 次 LLM 调用）

use anima_runtime::agent::{Agent, SdkTaskExecutor};
use anima_runtime::bus::{make_inbound, Bus, MakeInbound};
use anima_sdk::facade::Client as SdkClient;
use anima_sdk::{messages, sessions};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};

const BACKEND_URL: &str = "http://127.0.0.1:9711";
const ROUND_TIMEOUT: Duration = Duration::from_secs(120);

fn make_client() -> SdkClient {
    SdkClient::new(BACKEND_URL)
}

fn log(act: &str, msg: impl AsRef<str>) {
    eprintln!("[e2e/{}] {}", act, msg.as_ref());
}

/// 幕 1：后端只读 smoke —— 零 LLM 成本
///
/// 验证点：
///   - create_session 能落库并返回 ses_ 前缀 id
///   - list_sessions 能看到新 session
#[test]
#[ignore = "E2E: requires OpenCode backend at 127.0.0.1:9711"]
fn e2e_act1_backend_readonly_smoke() {
    let client = make_client();
    let started = Instant::now();

    let created = sessions::create_session(&client, None).expect("create_session ok");
    let session_id = created["id"]
        .as_str()
        .expect("session has id")
        .to_string();
    assert!(
        session_id.starts_with("ses_"),
        "unexpected session id: {session_id}"
    );
    log("act1", format!("created session_id={session_id}"));

    let listed = sessions::list_sessions(&client, None).expect("list_sessions ok");
    let hit = listed
        .as_array()
        .expect("sessions is array")
        .iter()
        .any(|s| s["id"].as_str() == Some(&session_id));
    assert!(hit, "new session {session_id} should appear in list");

    log(
        "act1",
        format!("readonly smoke passed in {:?}", started.elapsed()),
    );
}

/// 幕 2：单轮真实对话 —— 1 次 LLM 调用
///
/// 验证点：
///   - Agent + SdkTaskExecutor 能完成一次 inbound→outbound 完整链路
///   - outbound 内容非空（LLM 实际产生了回复）
///   - runtime projection 记录了本次 run
#[test]
#[ignore = "E2E: requires OpenCode backend at 127.0.0.1:9711"]
fn e2e_act2_single_turn_real_llm() {
    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(make_client()),
        None,
        Some(Arc::new(SdkTaskExecutor)),
    );
    agent.start();

    let started = Instant::now();
    let chat_id = "e2e-act2-chat";
    agent.process_message(make_inbound(MakeInbound {
        channel: "e2e".into(),
        sender_id: Some("e2e-act2-user".into()),
        chat_id: Some(chat_id.into()),
        content: "reply only the word: ok".into(),
        ..Default::default()
    }));

    let outbound = bus
        .outbound_receiver()
        .recv_timeout(ROUND_TIMEOUT)
        .expect("outbound arrives within 120s");
    log(
        "act2",
        format!(
            "outbound.content (len={}) preview={:?}",
            outbound.content.len(),
            outbound.content.chars().take(80).collect::<String>()
        ),
    );
    assert!(!outbound.content.is_empty(), "outbound content must not be empty");
    assert!(
        !outbound.content.starts_with("Error ["),
        "outbound should not be an error envelope: {}",
        outbound.content
    );

    let projection = agent.runtime_projection_snapshot();
    log(
        "act2",
        format!(
            "runtime_projection: runs={} turns={} tasks={} transcript={}",
            projection.runs.len(),
            projection.turns.len(),
            projection.tasks.len(),
            projection.transcript.len()
        ),
    );
    assert!(
        !projection.runs.is_empty(),
        "runtime_projection should record at least one run"
    );
    assert!(
        projection
            .runs
            .iter()
            .any(|r| r.chat_id.as_deref() == Some(chat_id)),
        "expected run with chat_id={chat_id}"
    );

    agent.stop();
    log(
        "act2",
        format!("single turn passed in {:?}", started.elapsed()),
    );
}

/// 幕 3：双轮真实对话，验证 session 复用 —— 2 次 LLM 调用
///
/// 验证点：
///   - 相同 chat_id 下两轮对话复用同一上游 session（服务端 diff 视角）
///   - 两次都拿到非空 outbound
///   - list_messages 显示 ≥4 条（2 条 user + 2 条 assistant）
#[test]
#[ignore = "E2E: requires OpenCode backend at 127.0.0.1:9711"]
fn e2e_act3_multi_turn_session_reuse() {
    let client = make_client();

    let before_ids = snapshot_session_ids(&client);
    log("act3", format!("before: {} existing sessions", before_ids.len()));

    let bus = Arc::new(Bus::create());
    let agent = Agent::create(
        bus.clone(),
        Some(client.clone()),
        None,
        Some(Arc::new(SdkTaskExecutor)),
    );
    agent.start();

    let started = Instant::now();
    let chat_id = "e2e-act3-chat";
    let prompts = ["reply only the word: hi", "reply only the word: bye"];

    for (idx, prompt) in prompts.iter().enumerate() {
        agent.process_message(make_inbound(MakeInbound {
            channel: "e2e".into(),
            sender_id: Some("e2e-act3-user".into()),
            chat_id: Some(chat_id.into()),
            content: (*prompt).into(),
            ..Default::default()
        }));
        let outbound = bus
            .outbound_receiver()
            .recv_timeout(ROUND_TIMEOUT)
            .unwrap_or_else(|_| panic!("round {idx} timed out after {:?}", ROUND_TIMEOUT));
        log(
            "act3",
            format!(
                "round {idx} content(len={}) preview={:?}",
                outbound.content.len(),
                outbound.content.chars().take(60).collect::<String>()
            ),
        );
        assert!(!outbound.content.is_empty(), "round {idx} content empty");
        assert!(
            !outbound.content.starts_with("Error ["),
            "round {idx} errored: {}",
            outbound.content
        );
    }

    let after_ids = snapshot_session_ids(&client);
    let new_ids: Vec<String> = after_ids
        .iter()
        .filter(|id| !before_ids.contains(*id))
        .cloned()
        .collect();
    log(
        "act3",
        format!("after: {} sessions, newly created: {:?}", after_ids.len(), new_ids),
    );
    assert_eq!(
        new_ids.len(),
        1,
        "same chat_id should create exactly 1 upstream session, got {}: {:?}",
        new_ids.len(),
        new_ids
    );

    let new_session_id = &new_ids[0];
    let listed = messages::list_messages(&client, new_session_id, None)
        .expect("list_messages ok");
    let count = listed.as_array().map(Vec::len).unwrap_or_default();
    let (user_count, assistant_count) = count_roles(&listed);
    log(
        "act3",
        format!(
            "list_messages: total={count} user={user_count} assistant={assistant_count}"
        ),
    );
    assert!(
        count >= 4,
        "expected >=4 messages in {new_session_id}, got {count}"
    );
    assert!(
        user_count >= 2 && assistant_count >= 2,
        "expected >=2 user + >=2 assistant, got user={user_count} assistant={assistant_count}"
    );

    agent.stop();
    log(
        "act3",
        format!("multi turn passed in {:?}", started.elapsed()),
    );
}

/// 扫描当前后端所有 session 的 id 集合。
fn snapshot_session_ids(client: &SdkClient) -> Vec<String> {
    sessions::list_sessions(client, None)
        .expect("list_sessions ok")
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 统计消息列表中的 user/assistant role 条数。
fn count_roles(messages: &Value) -> (usize, usize) {
    let mut user = 0;
    let mut assistant = 0;
    if let Some(arr) = messages.as_array() {
        for m in arr {
            match m
                .pointer("/info/role")
                .and_then(Value::as_str)
                .or_else(|| m.get("role").and_then(Value::as_str))
            {
                Some("user") => user += 1,
                Some("assistant") => assistant += 1,
                _ => {}
            }
        }
    }
    (user, assistant)
}
