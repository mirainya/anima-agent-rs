use anima_types::{AnimaError, Result};
use serde_json::{Map, Value};

use crate::{client as http, facade::Client, utils};

pub fn get_config(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/config", params)?)
}

pub fn update_config(client: &Client, config: &Value) -> Result<Value> {
    utils::handle_response(http::patch_request(client, "/config", config, None)?)
}

pub fn list_providers(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/config/providers", params)?)
}

pub fn list_commands(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/command", params)?)
}

pub fn list_agents(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/agent", params)?)
}

pub fn get_tool_ids(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/experimental/tool/ids", params)?)
}

pub fn list_tools(client: &Client, params: &Map<String, Value>) -> Result<Value> {
    utils::validate_required(params, &["provider", "model"])?;
    utils::handle_response(http::get_request(
        client,
        "/experimental/tool",
        Some(params),
    )?)
}

pub fn get_path(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/path", params)?)
}

pub fn write_log(client: &Client, params: &Map<String, Value>) -> Result<Value> {
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

    utils::handle_response(http::post_request(
        client,
        "/log",
        &Value::Object(body),
        None,
    )?)
}

pub fn get_mcp_status(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/mcp", params)?)
}

pub fn get_lsp_status(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/lsp", params)?)
}

pub fn get_formatter_status(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/formatter", params)?)
}

pub fn set_auth(client: &Client, auth_id: &str, auth: &Value) -> Result<Value> {
    utils::handle_response(http::put_request(
        client,
        &format!("/auth/{auth_id}"),
        auth,
        None,
    )?)
}
