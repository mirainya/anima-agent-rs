use anima_types::Result;
use serde_json::{json, Map, Value};

use crate::{client_async as http, facade_async::AsyncClient, utils};

pub async fn list_sessions(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/session", params).await?)
}

pub async fn create_session(client: &AsyncClient, options: Option<Value>) -> Result<Value> {
    let body = options.unwrap_or_else(|| json!({}));
    utils::handle_response(http::post_request(client, "/session", &body, None).await?)
}

pub async fn get_session(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::get_request(client, &format!("/session/{session_id}"), params).await?,
    )
}

pub async fn update_session(
    client: &AsyncClient,
    session_id: &str,
    updates: &Value,
) -> Result<Value> {
    utils::handle_response(
        http::patch_request(client, &format!("/session/{session_id}"), updates, None).await?,
    )
}

pub async fn delete_session(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::delete_request(client, &format!("/session/{session_id}"), params).await?,
    )
}

pub async fn get_session_children(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::get_request(client, &format!("/session/{session_id}/children"), params).await?,
    )
}

pub async fn get_session_todo(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::get_request(client, &format!("/session/{session_id}/todo"), params).await?,
    )
}

pub async fn init_session(
    client: &AsyncClient,
    session_id: &str,
    body: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(body, &["modelID", "providerID", "messageID"])?;
    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/init"),
            &Value::Object(body.clone()),
            None,
        )
        .await?,
    )
}

pub async fn fork_session(
    client: &AsyncClient,
    session_id: &str,
    message_id: Option<&str>,
) -> Result<Value> {
    let body = match message_id {
        Some(message_id) => json!({"messageID": message_id}),
        None => json!({}),
    };
    utils::handle_response(
        http::post_request(client, &format!("/session/{session_id}/fork"), &body, None).await?,
    )
}

pub async fn abort_session(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let body = params
        .cloned()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    utils::handle_response(
        http::post_request(client, &format!("/session/{session_id}/abort"), &body, None).await?,
    )
}

pub async fn share_session(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let body = params
        .cloned()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}));
    utils::handle_response(
        http::post_request(client, &format!("/session/{session_id}/share"), &body, None).await?,
    )
}

pub async fn unshare_session(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::delete_request(client, &format!("/session/{session_id}/share"), params).await?,
    )
}

pub async fn get_session_diff(
    client: &AsyncClient,
    session_id: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(
        http::get_request(client, &format!("/session/{session_id}/diff"), params).await?,
    )
}

pub async fn summarize_session(
    client: &AsyncClient,
    session_id: &str,
    body: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(body, &["providerID", "modelID"])?;
    utils::handle_response(
        http::post_request(
            client,
            &format!("/session/{session_id}/summarize"),
            &Value::Object(body.clone()),
            None,
        )
        .await?,
    )
}
