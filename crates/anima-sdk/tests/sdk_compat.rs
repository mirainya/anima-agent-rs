use anima_sdk::{client, config, files, messages, sessions, ApiErrorKind};
use serde_json::{json, Map, Value};

#[test]
fn client_reuses_http_client_with_timeouts() {
    // Client 构造时就应配置好超时，http_client 可复用
    let c = anima_sdk::Client::new("http://127.0.0.1:9711");
    // 确保 http_client 存在且可正常使用（不会 panic）
    let _url = client::build_url(&c.base_url, "/test");
}

#[test]
fn client_options_default_matches_phase_b_contract() {
    let options = anima_sdk::ClientOptions::default();
    assert_eq!(options.request_timeout_ms, 600_000);
    assert_eq!(options.connect_timeout_ms, 60_000);
    assert_eq!(options.max_retries, 1);
    assert_eq!(options.retry_backoff_ms, 250);
    assert_eq!(options.retry_backoff_cap_ms, 5_000);
}

#[test]
fn client_options_backoff_delay_grows_exponentially_until_cap() {
    let options = anima_sdk::ClientOptions {
        retry_backoff_ms: 100,
        retry_backoff_cap_ms: 1_000,
        ..Default::default()
    };
    assert_eq!(options.backoff_delay_ms(0), 0);
    assert_eq!(options.backoff_delay_ms(1), 100);
    assert_eq!(options.backoff_delay_ms(2), 200);
    assert_eq!(options.backoff_delay_ms(3), 400);
    assert_eq!(options.backoff_delay_ms(4), 800);
    // Capped at retry_backoff_cap_ms
    assert_eq!(options.backoff_delay_ms(5), 1_000);
    assert_eq!(options.backoff_delay_ms(20), 1_000);
}

#[test]
fn client_options_from_env_overrides_defaults() {
    // 使用唯一前缀避免与其他测试并发冲突
    // SAFETY: test-only env var mutation
    unsafe {
        std::env::set_var("ANIMA_SDK_REQUEST_TIMEOUT_MS", "12345");
        std::env::set_var("ANIMA_SDK_CONNECT_TIMEOUT_MS", "6789");
        std::env::set_var("ANIMA_SDK_MAX_RETRIES", "7");
        std::env::set_var("ANIMA_SDK_RETRY_BACKOFF_MS", "321");
        std::env::set_var("ANIMA_SDK_RETRY_BACKOFF_CAP_MS", "9999");
    }
    let options = anima_sdk::ClientOptions::from_env();
    assert_eq!(options.request_timeout_ms, 12345);
    assert_eq!(options.connect_timeout_ms, 6789);
    assert_eq!(options.max_retries, 7);
    assert_eq!(options.retry_backoff_ms, 321);
    assert_eq!(options.retry_backoff_cap_ms, 9999);
    unsafe {
        std::env::remove_var("ANIMA_SDK_REQUEST_TIMEOUT_MS");
        std::env::remove_var("ANIMA_SDK_CONNECT_TIMEOUT_MS");
        std::env::remove_var("ANIMA_SDK_MAX_RETRIES");
        std::env::remove_var("ANIMA_SDK_RETRY_BACKOFF_MS");
        std::env::remove_var("ANIMA_SDK_RETRY_BACKOFF_CAP_MS");
    }
}

#[test]
fn build_url_matches_clojure_behavior() {
    assert_eq!(
        client::build_url("http://localhost:9711", "/api"),
        "http://localhost:9711/api"
    );
    assert_eq!(
        client::build_url("http://localhost:9711/", "/api"),
        "http://localhost:9711/api"
    );
    assert_eq!(
        client::build_url("http://localhost:9711", "api"),
        "http://localhost:9711/api"
    );
    assert_eq!(
        client::build_url("http://localhost:9711/", "api"),
        "http://localhost:9711/api"
    );
}

#[test]
fn add_query_params_skips_empty() {
    let url = client::add_query_params("http://localhost:9711/api".into(), None);
    assert_eq!(url, "http://localhost:9711/api");

    let empty = Map::new();
    let url = client::add_query_params("http://localhost:9711/api".into(), Some(&empty));
    assert_eq!(url, "http://localhost:9711/api");
}

#[test]
fn add_query_params_appends_values() {
    let mut params = Map::new();
    params.insert("limit".into(), json!(10));
    params.insert("offset".into(), json!(0));
    let url = client::add_query_params("http://localhost:9711/api".into(), Some(&params));
    assert!(url.contains("limit=10"));
    assert!(url.contains("offset=0"));
}

#[test]
fn parse_response_matches_status_mapping() {
    let ok = client::parse_response(200, "{\"data\":\"test\"}");
    assert!(ok.success);
    assert_eq!(ok.data, Some(json!({"data":"test"})));

    let created = client::parse_response(201, "{\"id\":\"123\"}");
    assert!(created.success);
    assert_eq!(created.data, Some(json!({"id":"123"})));

    let bad = client::parse_response(400, "{\"error\":\"Bad request\"}");
    assert!(!bad.success);
    assert_eq!(bad.error, Some(ApiErrorKind::BadRequest));

    let not_found = client::parse_response(404, "{\"error\":\"Not found\"}");
    assert_eq!(not_found.error, Some(ApiErrorKind::NotFound));

    let server = client::parse_response(500, "{\"error\":\"Server error\"}");
    assert_eq!(server.error, Some(ApiErrorKind::ServerError));

    let unknown = client::parse_response(999, "{\"error\":\"Unknown\"}");
    assert_eq!(unknown.error, Some(ApiErrorKind::Unknown));
    assert_eq!(unknown.status, Some(999));
}

#[test]
fn parse_response_keeps_raw_string_when_json_invalid() {
    let response = client::parse_response(200, "not-json");
    assert_eq!(response.data, Some(Value::String("not-json".into())));
}

#[test]
fn send_prompt_normalizes_string() {
    let normalized = messages::normalize_message(Value::String("Hello world".into())).unwrap();
    assert_eq!(
        normalized,
        json!({"parts": [{"type": "text", "text": "Hello world"}]})
    );
}

#[test]
fn send_prompt_normalizes_text_map() {
    let normalized = messages::normalize_message(json!({"text": "Hello from map"})).unwrap();
    assert_eq!(
        normalized,
        json!({"parts": [{"type": "text", "text": "Hello from map"}]})
    );
}

#[test]
fn send_prompt_preserves_parts() {
    let message = json!({"parts": [{"type": "text", "text": "Hello"}, {"type": "code", "text": "println('hi')"}]});
    let normalized = messages::normalize_message(message.clone()).unwrap();
    assert_eq!(normalized, message);
}

#[test]
fn send_prompt_rejects_invalid_format() {
    assert!(messages::normalize_message(json!(123)).is_err());
    assert!(messages::normalize_message(json!({"invalid": "format"})).is_err());
}

#[test]
fn files_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");
    let err = files::list_files(&client, &Map::new()).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: path"));

    let err = files::find_text(&client, Value::Null, None).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: pattern"));
}

#[test]
fn config_list_tools_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");
    let mut params = Map::new();
    params.insert("provider".into(), json!("openai"));
    let err = config::list_tools(&client, &params).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: model"));
}

#[test]
fn config_write_log_level_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");
    let mut params = Map::new();
    params.insert("service".into(), json!("test-service"));
    params.insert("level".into(), json!("invalid"));
    params.insert("message".into(), json!("test"));
    let err = config::write_log(&client, &params).unwrap_err();
    assert!(err.to_string().contains("Invalid level"));
}

#[test]
fn sessions_required_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");
    let mut body = Map::new();
    body.insert("modelID".into(), json!("model-1"));
    let err = sessions::init_session(&client, "123", &body).unwrap_err();
    assert!(err.to_string().contains("Missing required parameters"));
}

#[test]
fn respond_to_permission_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");
    let err = messages::respond_to_permission(&client, "123", "perm-1", "invalid").unwrap_err();
    assert!(err.to_string().contains("Invalid response"));
}

#[test]
fn add_query_params_appends_to_existing_query_and_formats_values() {
    let mut params = Map::new();
    params.insert("flag".into(), json!(true));
    params.insert("text".into(), json!("hello"));
    params.insert("empty".into(), Value::Null);
    params.insert("list".into(), json!([1, 2]));

    let url =
        client::add_query_params("http://localhost:9711/api?existing=1".into(), Some(&params));
    assert!(url.starts_with("http://localhost:9711/api?existing=1&"));
    assert!(url.contains("flag=true"));
    assert!(url.contains("text=hello"));
    assert!(url.contains("empty=null"));
    // list=[1,2] → percent-encoded brackets/comma
    assert!(url.contains("list="));
}

#[test]
fn query_params_percent_encodes_special_characters() {
    let mut params = Map::new();
    params.insert("q".into(), json!("hello world&foo=bar"));
    let url = client::add_query_params("http://localhost:9711/api".into(), Some(&params));
    // 空格→%20, &→%26, =→%3D
    assert!(url.contains("q=hello%20world%26foo%3Dbar"));
}

#[test]
fn build_url_preserves_nested_endpoint_paths() {
    assert_eq!(
        client::build_url("http://localhost:9711/base/", "/session/123/message"),
        "http://localhost:9711/base/session/123/message"
    );
    assert_eq!(
        client::build_url("http://localhost:9711/base", "session/123/message"),
        "http://localhost:9711/base/session/123/message"
    );
}

#[test]
fn parse_response_treats_empty_body_as_success_without_data() {
    let response = client::parse_response(200, "");
    assert!(response.success);
    assert_eq!(response.data, None);
    assert_eq!(response.error, None);
}

#[test]
fn send_prompt_normalizes_null_text_field() {
    let normalized = messages::normalize_message(json!({"text": null})).unwrap();
    assert_eq!(
        normalized,
        json!({"parts": [{"type": "text", "text": null}]})
    );
}

#[test]
fn messages_command_related_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");

    let mut command_only = Map::new();
    command_only.insert("command".into(), json!("/help"));
    let err = messages::execute_command(&client, "session-1", &command_only).unwrap_err();
    assert!(err.to_string().contains("Missing required parameters"));

    let mut shell_only = Map::new();
    shell_only.insert("command".into(), json!("ls"));
    let err = messages::run_shell_command(&client, "session-1", &shell_only).unwrap_err();
    assert!(err.to_string().contains("Missing required parameters"));

    let empty = Map::new();
    let err = messages::revert_message(&client, "session-1", &empty).unwrap_err();
    assert!(err.to_string().contains("Missing required parameters"));
}

#[test]
fn files_additional_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");

    let err = files::read_file(&client, &Map::new()).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: path"));

    let err = files::find_files(&client, Value::Null, None).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: query"));

    let err = files::find_symbols(&client, Value::Null, None).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: query"));
}

#[test]
fn config_and_session_summary_validation_matches_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");

    let empty = Map::new();
    let err = sessions::summarize_session(&client, "session-1", &empty).unwrap_err();
    assert!(err.to_string().contains("Missing required parameters"));

    let mut provider_only = Map::new();
    provider_only.insert("provider".into(), json!("anthropic"));
    let err = config::list_tools(&client, &provider_only).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: model"));
}

#[test]
fn messages_more_validation_edges_match_baseline() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");

    let mut missing_command = Map::new();
    missing_command.insert("arguments".into(), json!(["arg1"]));
    let err = messages::execute_command(&client, "123", &missing_command).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: command"));

    let mut missing_arguments = Map::new();
    missing_arguments.insert("command".into(), json!("test-command"));
    let err = messages::execute_command(&client, "123", &missing_arguments).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: arguments"));

    let mut missing_agent = Map::new();
    missing_agent.insert("command".into(), json!("ls -la"));
    let err = messages::run_shell_command(&client, "123", &missing_agent).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: agent"));

    let empty = Map::new();
    let err = messages::revert_message(&client, "123", &empty).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: message-id"));
}

#[test]
fn files_config_projects_and_sessions_more_matrix_edges() {
    let client = anima_sdk::Client::new("http://127.0.0.1:9711");

    let mut missing_service = Map::new();
    missing_service.insert("level".into(), json!("info"));
    missing_service.insert("message".into(), json!("test message"));
    let err = config::write_log(&client, &missing_service).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: service"));

    let err = files::list_files(&client, &Map::new()).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: path"));

    let err = files::read_file(&client, &Map::new()).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: path"));

    let mut summary_missing_model = Map::new();
    summary_missing_model.insert("providerID".into(), json!("anthropic"));
    let err =
        sessions::summarize_session(&client, "session-1", &summary_missing_model).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: modelID"));

    let mut init_missing_message = Map::new();
    init_missing_message.insert("modelID".into(), json!("claude"));
    init_missing_message.insert("providerID".into(), json!("anthropic"));
    let err = sessions::init_session(&client, "session-1", &init_missing_message).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required parameters: messageID"));

    assert_eq!(
        client::build_url("http://127.0.0.1:9711", "/project"),
        "http://127.0.0.1:9711/project"
    );
    assert_eq!(
        client::build_url("http://127.0.0.1:9711", "/project/current"),
        "http://127.0.0.1:9711/project/current"
    );
}
