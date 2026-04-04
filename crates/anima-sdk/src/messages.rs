use anima_types::{AnimaError, Result};
use serde_json::{json, Map, Value};

use crate::{client as http, facade::Client, utils};

pub fn list_messages(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(
        client,
        &format!("/session/{session_id}/message"),
        params,
    )?)
}

pub fn get_message(
    client: &Client,
    session_id: &str,
    message_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(
        client,
        &format!("/session/{session_id}/message/{message_id}"),
        params,
    )?)
}

pub fn normalize_message(message: Value) -> Result<Value> {
    match message {
        Value::String(text) => Ok(json!({"parts": [{"type": "text", "text": text}]})),
        Value::Object(map) if map.contains_key("text") && !map.contains_key("parts") => Ok(
            json!({"parts": [{"type": "text", "text": map.get("text").cloned().unwrap_or(Value::Null)}]}),
        ),
        Value::Object(map) if map.contains_key("parts") => Ok(Value::Object(map)),
        _ => Err(AnimaError::InvalidMessageFormat),
    }
}


pub fn execute_command(
    client: &Client,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(params, &["arguments", "command"])?;
    let mut body = Map::new();
    body.insert(
        "arguments".into(),
        params.get("arguments").cloned().unwrap(),
    );
    body.insert("command".into(), params.get("command").cloned().unwrap());
    if let Some(agent) = params.get("agent") {
        body.insert("agent".into(), agent.clone());
    }
    if let Some(model) = params.get("model") {
        body.insert("model".into(), model.clone());
    }
    if let Some(message_id) = params.get("message-id") {
        body.insert("messageID".into(), message_id.clone());
    }
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/command"),
        &Value::Object(body),
        None,
    )?)
}

pub fn run_shell_command(
    client: &Client,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(params, &["agent", "command"])?;
    let body = json!({
        "agent": params.get("agent").cloned().unwrap(),
        "command": params.get("command").cloned().unwrap(),
    });
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/shell"),
        &body,
        None,
    )?)
}

pub fn revert_message(
    client: &Client,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(params, &["message-id"])?;
    let mut body = Map::new();
    body.insert(
        "messageID".into(),
        params.get("message-id").cloned().unwrap(),
    );
    if let Some(part_id) = params.get("part-id") {
        body.insert("partID".into(), part_id.clone());
    }
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/revert"),
        &Value::Object(body),
        None,
    )?)
}

pub fn unrevert_messages(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let body = params
        .cloned()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/unrevert"),
        &body,
        None,
    )?)
}

pub fn respond_to_permission(
    client: &Client,
    session_id: &str,
    permission_id: &str,
    response: &str,
) -> Result<Value> {
    if !matches!(response, "once" | "always" | "reject") {
        return Err(AnimaError::InvalidPermissionResponse);
    }
    let body = json!({"response": response});
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/permissions/{permission_id}"),
        &body,
        None,
    )?)
}

pub fn send_prompt_with_agent(
    client: &Client,
    session_id: &str,
    message: Value,
    agent: Option<&str>,
) -> Result<Value> {
    let mut normalized = match normalize_message(message)? {
        Value::Object(map) => map,
        _ => unreachable!(),
    };
    if let Some(agent) = agent {
        normalized.insert("agent".into(), Value::String(agent.to_string()));
    }

    let mut params = Map::new();
    if let Some(directory) = client.directory.as_deref() {
        params.insert("directory".into(), Value::String(directory.to_string()));
    }
    params.insert("workspace".into(), Value::String("global".to_string()));
    let params_ref = if params.is_empty() { None } else { Some(&params) };

    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/message"),
        &Value::Object(normalized),
        params_ref,
    )?)
}

pub fn send_prompt(
    client: &Client,
    session_id: &str,
    message: Value,
    agent: Option<&str>,
) -> Result<Value> {
    send_prompt_with_agent(client, session_id, message, agent)
}

pub fn subscribe_event_stream(
    client: &Client,
    directory: Option<&str>,
    workspace: Option<&str>,
) -> Result<reqwest::blocking::Response> {
    let mut params = Map::new();
    if let Some(directory) = directory.or(client.directory.as_deref()) {
        params.insert("directory".into(), Value::String(directory.to_string()));
    }
    if let Some(workspace) = workspace {
        params.insert("workspace".into(), Value::String(workspace.to_string()));
    }

    let params_ref = if params.is_empty() { None } else { Some(&params) };
    http::get_request_streaming(client, "/event", params_ref)
}
