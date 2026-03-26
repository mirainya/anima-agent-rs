use anima_types::{AnimaError, ApiErrorKind, ApiResponse, Result};
use reqwest::blocking::{Client as HttpClient, RequestBuilder};
use serde_json::{Map, Value};
use std::time::Duration;

use crate::facade::Client;

#[derive(Debug, Clone, PartialEq)]
pub struct RequestOptions {
    pub throw_exceptions: bool,
    pub accept_json: bool,
    pub content_type_json: bool,
    pub socket_timeout_ms: u64,
    pub connection_timeout_ms: u64,
}

pub fn default_opts() -> RequestOptions {
    RequestOptions {
        throw_exceptions: false,
        accept_json: true,
        content_type_json: true,
        socket_timeout_ms: 600_000,
        connection_timeout_ms: 60_000,
    }
}

pub fn build_url(base_url: &str, endpoint: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = endpoint.trim_start_matches('/');
    format!("{base}/{path}")
}

pub fn add_query_params(mut url: String, params: Option<&Map<String, Value>>) -> String {
    let Some(params) = params else {
        return url;
    };
    if params.is_empty() {
        return url;
    }

    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, query_value(v)))
        .collect::<Vec<_>>()
        .join("&");

    if url.contains('?') {
        url.push('&');
    } else {
        url.push('?');
    }
    url.push_str(&query);
    url
}

pub(crate) fn query_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        _ => value.to_string(),
    }
}

pub fn parse_response(status: u16, body: &str) -> ApiResponse {
    let parsed = if body.is_empty() {
        None
    } else {
        serde_json::from_str::<Value>(body)
            .ok()
            .or_else(|| Some(Value::String(body.to_string())))
    };

    match status {
        200 | 201 => ApiResponse {
            success: true,
            data: parsed,
            error: None,
            status: None,
        },
        400 => ApiResponse {
            success: false,
            data: parsed,
            error: Some(ApiErrorKind::BadRequest),
            status: None,
        },
        404 => ApiResponse {
            success: false,
            data: parsed,
            error: Some(ApiErrorKind::NotFound),
            status: None,
        },
        500 => ApiResponse {
            success: false,
            data: parsed,
            error: Some(ApiErrorKind::ServerError),
            status: None,
        },
        other => ApiResponse {
            success: false,
            data: parsed,
            error: Some(ApiErrorKind::Unknown),
            status: Some(other),
        },
    }
}

fn http_client_with_timeouts() -> Result<HttpClient> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(600_000))
        .connect_timeout(Duration::from_millis(60_000))
        .build()
        .map_err(|err| AnimaError::Transport(err.to_string()))
}

fn send(builder: RequestBuilder) -> Result<ApiResponse> {
    let response = builder
        .send()
        .map_err(|err| AnimaError::Transport(err.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .map_err(|err| AnimaError::Transport(err.to_string()))?;
    Ok(parse_response(status, &body))
}

pub fn get_request(
    client: &Client,
    endpoint: &str,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let http = http_client_with_timeouts()?;
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(http.get(url).header("accept", "application/json"))
}

pub fn post_request(
    client: &Client,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let http = http_client_with_timeouts()?;
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        http.post(url)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(serde_json::to_string(body).map_err(|err| AnimaError::Json(err.to_string()))?),
    )
}

pub fn patch_request(
    client: &Client,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let http = http_client_with_timeouts()?;
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        http.patch(url)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(serde_json::to_string(body).map_err(|err| AnimaError::Json(err.to_string()))?),
    )
}

pub fn put_request(
    client: &Client,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let http = http_client_with_timeouts()?;
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        http.put(url)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(serde_json::to_string(body).map_err(|err| AnimaError::Json(err.to_string()))?),
    )
}

pub fn delete_request(
    client: &Client,
    endpoint: &str,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let http = http_client_with_timeouts()?;
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(http.delete(url).header("accept", "application/json"))
}
