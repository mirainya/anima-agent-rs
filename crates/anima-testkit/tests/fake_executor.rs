use anima_runtime::worker::executor::TaskExecutor;
use anima_testkit::{fake_prompt_response, FakeExecutor};
use serde_json::json;

#[test]
fn fake_executor_returns_configured_session_and_prompt_format() {
    let executor = FakeExecutor::new()
        .with_session_id("session-x")
        .with_prompt_prefix("mock-reply");

    let session = executor.create_session().unwrap();
    let prompt = executor.send_prompt("session-x", json!("hello")).unwrap();

    assert_eq!(session["id"], "session-x");
    assert_eq!(prompt["content"], "mock-reply[session-x]: hello");
}

#[test]
fn fake_executor_can_simulate_prompt_and_session_failures() {
    let prompt_fail = FakeExecutor::new().fail_prompts_with("prompt boom");
    assert_eq!(
        prompt_fail
            .send_prompt("session-x", json!("hello"))
            .unwrap_err()
            .message(),
        "prompt boom"
    );

    let session_fail = FakeExecutor::new().fail_session_create_with("session boom");
    assert_eq!(
        session_fail.create_session().unwrap_err().message(),
        "session boom"
    );
}

#[test]
fn fake_prompt_response_helper_matches_default_reply_shape() {
    let value = fake_prompt_response("session-1", "hi");
    assert_eq!(value["content"], "reply[session-1]: hi");
}
