use anima_types::Result;
use serde_json::{json, Map, Value};

use crate::{client as http, facade::Client, utils};

pub fn list_sessions(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/session", params)?)
}

pub fn create_session(client: &Client, options: Option<Value>) -> Result<Value> {
    let body = options.unwrap_or_else(|| json!({}));
    utils::handle_response(http::post_request(client, "/session", &body, None)?)
}

pub fn get_session(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(
        client,
        &format!("/session/{session_id}"),
        params,
    )?)
}

pub fn update_session(client: &Client, session_id: &str, updates: &Value) -> Result<Value> {
    utils::handle_response(http::patch_request(
        client,
        &format!("/session/{session_id}"),
        updates,
        None,
    )?)
}

pub fn delete_session(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::delete_request(
        client,
        &format!("/session/{session_id}"),
        params,
    )?)
}

pub fn get_session_children(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(
        client,
        &format!("/session/{session_id}/children"),
        params,
    )?)
}

pub fn get_session_todo(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(
        client,
        &format!("/session/{session_id}/todo"),
        params,
    )?)
}

pub fn init_session(client: &Client, session_id: &str, body: &Map<String, Value>) -> Result<Value> {
    utils::validate_required(body, &["modelID", "providerID", "messageID"])?;
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/init"),
        &Value::Object(body.clone()),
        None,
    )?)
}

pub fn fork_session(client: &Client, session_id: &str, message_id: Option<&str>) -> Result<Value> {
    let body = match message_id {
        Some(message_id) => json!({"messageID": message_id}),
        None => json!({}),
    };
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/fork"),
        &body,
        None,
    )?)
}

pub fn abort_session(
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
        &format!("/session/{session_id}/abort"),
        &body,
        None,
    )?)
}

pub fn share_session(
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
        &format!("/session/{session_id}/share"),
        &body,
        None,
    )?)
}

pub fn unshare_session(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::delete_request(
        client,
        &format!("/session/{session_id}/share"),
        params,
    )?)
}

pub fn get_session_diff(
    client: &Client,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(
        client,
        &format!("/session/{session_id}/diff"),
        params,
    )?)
}

pub fn summarize_session(
    client: &Client,
    session_id: &str,
    body: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(body, &["providerID", "modelID"])?;
    utils::handle_response(http::post_request(
        client,
        &format!("/session/{session_id}/summarize"),
        &Value::Object(body.clone()),
        None,
    )?)
}
