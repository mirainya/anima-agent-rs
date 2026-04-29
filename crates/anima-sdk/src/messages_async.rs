use anima_types::{AnimaError, Result};
use serde_json::{json, Map, Value};

use crate::{client_async as http, facade_async::AsyncClient, messages::normalize_message, utils};

pub async fn list_messages(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::get_request(client, &format!("/session/{session_id}/message"), params).await?,
    )
}

pub async fn get_message(
    client: &AsyncClient,
    session_id: &str,
    message_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::get_request(
            client,
            &format!("/session/{session_id}/message/{message_id}"),
            params,
        )
        .await?,
    )
}

pub async fn execute_command(
    client: &AsyncClient,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<Value> {
    let mut body = Map::new();
    body.insert(
        "arguments".into(),
        utils::require_param(params, "arguments")?,
    );
    body.insert("command".into(), utils::require_param(params, "command")?);
    if let Some(agent) = params.get("agent") {
        body.insert("agent".into(), agent.clone());
    }
    if let Some(model) = params.get("model") {
        body.insert("model".into(), model.clone());
    }
    if let Some(message_id) = params.get("message-id") {
        body.insert("messageID".into(), message_id.clone());
    }
    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/command"),
            &Value::Object(body),
            None,
        )
        .await?,
    )
}

pub async fn run_shell_command(
    client: &AsyncClient,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<Value> {
    let body = json!({
        "agent": utils::require_param(params, "agent")?,
        "command": utils::require_param(params, "command")?,
    });
    utils::handle_response(
        http::post_request(client, &format!("/session/{session_id}/shell"), &body, None).await?,
    )
}

pub async fn revert_message(
    client: &AsyncClient,
    session_id: &str,
    params: &Map<String, Value>,
) -> Result<Value> {
    let mut body = Map::new();
    body.insert(
        "messageID".into(),
        utils::require_param(params, "message-id")?,
    );
    if let Some(part_id) = params.get("part-id") {
        body.insert("partID".into(), part_id.clone());
    }
    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/revert"),
            &Value::Object(body),
            None,
        )
        .await?,
    )
}

pub async fn unrevert_messages(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let body = params
        .cloned()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/unrevert"),
            &body,
            None,
        )
        .await?,
    )
}

pub async fn respond_to_permission(
    client: &AsyncClient,
    session_id: &str,
    permission_id: &str,
    response: &str,
) -> Result<Value> {
    if !matches!(response, "once" | "always" | "reject") {
        return Err(AnimaError::InvalidPermissionResponse);
    }
    let body = json!({"response": response});
    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/permissions/{permission_id}"),
            &body,
            None,
        )
        .await?,
    )
}

pub async fn send_prompt_with_agent(
    client: &AsyncClient,
    session_id: &str,
    message: Value,
    agent: Option<&str>,
) -> Result<Value> {
    let mut normalized = match normalize_message(message)? {
        Value::Object(map) => map,
        _ => return Err(AnimaError::InvalidMessageFormat),
    };
    if let Some(agent) = agent {
        normalized.insert("agent".into(), Value::String(agent.to_string()));
    }

    let mut params = Map::new();
    if let Some(directory) = client.directory.as_deref() {
        params.insert("directory".into(), Value::String(directory.to_string()));
    }
    let params_ref = if params.is_empty() {
        None
    } else {
        Some(&params)
    };

    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/message"),
            &Value::Object(normalized),
            params_ref,
        )
        .await?,
    )
}

pub async fn send_prompt(
    client: &AsyncClient,
    session_id: &str,
    message: Value,
    agent: Option<&str>,
) -> Result<Value> {
    send_prompt_with_agent(client, session_id, message, agent).await
}

pub async fn subscribe_event_stream(
    client: &AsyncClient,
    directory: Option<&str>,
    workspace: Option<&str>,
) -> Result<reqwest::Response> {
    let mut params = Map::new();
    if let Some(directory) = directory.or(client.directory.as_deref()) {
        params.insert("directory".into(), Value::String(directory.to_string()));
    }
    if let Some(workspace) = workspace {
        params.insert("workspace".into(), Value::String(workspace.to_string()));
    }

    let params_ref = if params.is_empty() {
        None
    } else {
        Some(&params)
    };
    http::get_request_streaming(client, "/event", params_ref).await
}
