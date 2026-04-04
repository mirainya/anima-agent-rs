use anima_types::{AnimaError, ApiErrorKind, ApiResponse, Result};
use reqwest::blocking::RequestBuilder;
use serde_json::{Map, Value};
use std::time::Instant;

use crate::facade::Client;

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
    let raw = match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        _ => value.to_string(),
    };
    percent_encode(&raw)
}

/// Percent-encode a query parameter value (RFC 3986 unreserved characters pass through)
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
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

fn debug_http_enabled() -> bool {
    std::env::var("ANIMA_SDK_DEBUG_HTTP")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let preview = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        format!("{preview}…")
    } else {
        preview
    }
}

fn debug_http_event(label: &str, detail: &str) {
    if debug_http_enabled() {
        eprintln!("[anima-sdk/http] {label}: {detail}");
    }
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
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(client.http_client.get(url).header("accept", "application/json"))
}

pub fn post_request(
    client: &Client,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        client
            .http_client
            .post(url)
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
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        client
            .http_client
            .patch(url)
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
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        client
            .http_client
            .put(url)
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
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send(
        client
            .http_client
            .delete(url)
            .header("accept", "application/json"),
    )
}

/// 发送 GET 请求并返回原始 Response（不读取 body），用于 SSE 流式读取
pub fn get_request_streaming(
    client: &Client,
    endpoint: &str,
    params: Option<&Map<String, Value>>,
) -> Result<reqwest::blocking::Response> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    let started = Instant::now();

    debug_http_event(
        "streaming_send_start",
        &format!("url={url} method=GET accept=text/event-stream"),
    );

    let response = client
        .http_client
        .get(url.clone())
        .header("accept", "text/event-stream")
        .send()
        .map_err(|err| {
            let elapsed_ms = started.elapsed().as_millis();
            let detail = format!(
                "streaming request transport error after {elapsed_ms}ms: url={url} method=GET accept=text/event-stream error={err}"
            );
            debug_http_event("streaming_send_error", &detail);
            AnimaError::Transport(detail)
        })?;

    let elapsed_ms = started.elapsed().as_millis();
    let status = response.status().as_u16();
    debug_http_event(
        "streaming_send_response",
        &format!("url={url} status={status} elapsed_ms={elapsed_ms}"),
    );
    if status != 200 {
        let body_text = response
            .text()
            .map_err(|err| AnimaError::Transport(err.to_string()))?;
        let body_text_preview = preview_text(&body_text, 240);
        return Err(AnimaError::Transport(format!(
            "streaming request failed with status {status} after {elapsed_ms}ms: url={url} method=GET accept=text/event-stream response_body_preview={body_text_preview}"
        )));
    }

    Ok(response)
}
