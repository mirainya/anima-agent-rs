use anima_types::Result;
use serde_json::{Map, Value};

use crate::{client_async as http, facade_async::AsyncClient, utils};

pub async fn list_files(
    client: &AsyncClient,
    params: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(params, &["path"])?;
    utils::handle_response(http::get_request(client, "/file", Some(params)).await?)
}

pub async fn read_file(
    client: &AsyncClient,
    params: &Map<String, Value>,
) -> Result<Value> {
    utils::validate_required(params, &["path"])?;
    utils::handle_response(http::get_request(client, "/file/content", Some(params)).await?)
}

pub async fn get_file_status(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/file/status", params).await?)
}

pub async fn find_text(
    client: &AsyncClient,
    pattern: Value,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("pattern".into(), pattern);
    utils::validate_required(&query, &["pattern"])?;
    if let Some(params) = params {
        query.extend(params.clone());
    }
    utils::handle_response(http::get_request(client, "/find", Some(&query)).await?)
}

pub async fn find_files(
    client: &AsyncClient,
    q: Value,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("query".into(), q);
    utils::validate_required(&query, &["query"])?;
    if let Some(params) = params {
        query.extend(params.clone());
    }
    utils::handle_response(http::get_request(client, "/find/file", Some(&query)).await?)
}

pub async fn find_symbols(
    client: &AsyncClient,
    q: Value,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("query".into(), q);
    utils::validate_required(&query, &["query"])?;
    if let Some(params) = params {
        query.extend(params.clone());
    }
    utils::handle_response(http::get_request(client, "/find/symbol", Some(&query)).await?)
}
