use anima_types::{AnimaError, ApiResponse, Result};
use reqwest::RequestBuilder;
use serde_json::{Map, Value};
use std::time::Duration;

use crate::client::{add_query_params, build_url, parse_response};
use crate::facade_async::AsyncClient;

async fn send_with_retry<F>(client: &AsyncClient, mut make_builder: F) -> Result<ApiResponse>
where
    F: FnMut() -> RequestBuilder,
{
    let mut attempt = 0;
    loop {
        let response = make_builder().send().await;
        match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response
                    .text()
                    .await
                    .map_err(|err| AnimaError::Transport(err.to_string()))?;
                if status >= 500 && attempt < client.options.max_retries {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(
                        client.options.backoff_delay_ms(attempt),
                    ))
                    .await;
                    continue;
                }
                return Ok(parse_response(status, &body));
            }
            Err(err) => {
                if attempt < client.options.max_retries {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(
                        client.options.backoff_delay_ms(attempt),
                    ))
                    .await;
                    continue;
                }
                return Err(AnimaError::Transport(err.to_string()));
            }
        }
    }
}

pub async fn get_request(
    client: &AsyncClient,
    endpoint: &str,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send_with_retry(client, || {
        client
            .http_client
            .get(url.clone())
            .header("accept", "application/json")
    })
    .await
}

pub async fn post_request(
    client: &AsyncClient,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    let request_body =
        serde_json::to_string(body).map_err(|err| AnimaError::Json(err.to_string()))?;
    send_with_retry(client, || {
        client
            .http_client
            .post(url.clone())
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(request_body.clone())
    })
    .await
}

pub async fn patch_request(
    client: &AsyncClient,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    let request_body =
        serde_json::to_string(body).map_err(|err| AnimaError::Json(err.to_string()))?;
    send_with_retry(client, || {
        client
            .http_client
            .patch(url.clone())
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(request_body.clone())
    })
    .await
}

pub async fn put_request(
    client: &AsyncClient,
    endpoint: &str,
    body: &Value,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    let request_body =
        serde_json::to_string(body).map_err(|err| AnimaError::Json(err.to_string()))?;
    send_with_retry(client, || {
        client
            .http_client
            .put(url.clone())
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .body(request_body.clone())
    })
    .await
}

pub async fn delete_request(
    client: &AsyncClient,
    endpoint: &str,
    params: Option<&Map<String, Value>>,
) -> Result<ApiResponse> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    send_with_retry(client, || {
        client
            .http_client
            .delete(url.clone())
            .header("accept", "application/json")
    })
    .await
}

pub async fn get_request_streaming(
    client: &AsyncClient,
    endpoint: &str,
    params: Option<&Map<String, Value>>,
) -> Result<reqwest::Response> {
    let url = add_query_params(build_url(&client.base_url, endpoint), params);
    let started = std::time::Instant::now();

    let response = client
        .http_client
        .get(url.clone())
        .header("accept", "text/event-stream")
        .send()
        .await
        .map_err(|err| {
            let elapsed_ms = started.elapsed().as_millis();
            AnimaError::Transport(format!(
                "streaming request transport error after {elapsed_ms}ms: url={url} error={err}"
            ))
        })?;

    let status = response.status().as_u16();
    if status != 200 {
        let elapsed_ms = started.elapsed().as_millis();
        let body_text = response
            .text()
            .await
            .map_err(|err| AnimaError::Transport(err.to_string()))?;
        let preview: String = body_text.chars().take(240).collect();
        return Err(AnimaError::Transport(format!(
            "streaming request failed with status {status} after {elapsed_ms}ms: url={url} response_body_preview={preview}"
        )));
    }

    Ok(response)
}
