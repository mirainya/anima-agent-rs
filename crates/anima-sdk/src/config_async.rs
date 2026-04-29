use anima_types::{AnimaError, Result};
use serde_json::{Map, Value};

use crate::{client_async as http, facade_async::AsyncClient, utils};

pub async fn get_config(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/config", params).await?)
}

pub async fn update_config(client: &AsyncClient, config: &Value) -> Result<Value> {
    utils::handle_response(http::patch_request(client, "/config", config, None).await?)
}

pub async fn list_providers(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/config/providers", params).await?)
}

pub async fn list_commands(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/command", params).await?)
}

pub async fn list_agents(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/agent", params).await?)
}

pub async fn get_tool_ids(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/experimental/tool/ids", params).await?)
}

pub async fn list_tools(client: &AsyncClient, params: &Map<String, Value>) -> Result<Value> {
    utils::validate_required(params, &["provider", "model"])?;
    utils::handle_response(http::get_request(client, "/experimental/tool", Some(params)).await?)
}

pub async fn get_path(client: &AsyncClient, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/path", params).await?)
}

pub async fn write_log(client: &AsyncClient, params: &Map<String, Value>) -> Result<Value> {
    utils::validate_required(params, &["service", "level", "message"])?;
    let level = params
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(level, "debug" | "info" | "error" | "warn") {
        return Err(AnimaError::InvalidLogLevel);
    }

    let mut body = Map::new();
    body.insert("service".into(), utils::require_param(params, "service")?);
    body.insert("level".into(), utils::require_param(params, "level")?);
    body.insert("message".into(), utils::require_param(params, "message")?);
    if let Some(extra) = params.get("extra") {
        body.insert("extra".into(), extra.clone());
    }

    utils::handle_response(http::post_request(client, "/log", &Value::Object(body), None).await?)
}

pub async fn get_mcp_status(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/mcp", params).await?)
}

pub async fn get_lsp_status(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/lsp", params).await?)
}

pub async fn get_formatter_status(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/formatter", params).await?)
}

pub async fn set_auth(client: &AsyncClient, auth_id: &str, auth: &Value) -> Result<Value> {
    utils::handle_response(
        http::put_request(client, &format!("/auth/{auth_id}"), auth, None).await?,
    )
}
