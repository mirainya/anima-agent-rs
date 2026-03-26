use anima_types::Result;
use serde_json::{Map, Value};

use crate::{client as http, facade::Client, utils};

pub fn list_files(client: &Client, params: &Map<String, Value>) -> Result<Value> {
    utils::validate_required(params, &["path"])?;
    utils::handle_response(http::get_request(client, "/file", Some(params))?)
}

pub fn read_file(client: &Client, params: &Map<String, Value>) -> Result<Value> {
    utils::validate_required(params, &["path"])?;
    utils::handle_response(http::get_request(client, "/file/content", Some(params))?)
}

pub fn get_file_status(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/file/status", params)?)
}

pub fn find_text(
    client: &Client,
    pattern: Value,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("pattern".into(), pattern);
    utils::validate_required(&query, &["pattern"])?;
    if let Some(params) = params {
        query.extend(params.clone());
    }
    utils::handle_response(http::get_request(client, "/find", Some(&query))?)
}

pub fn find_files(client: &Client, q: Value, params: Option<&Map<String, Value>>) -> Result<Value> {
    let mut query = Map::new();
    query.insert("query".into(), q);
    utils::validate_required(&query, &["query"])?;
    if let Some(params) = params {
        query.extend(params.clone());
    }
    utils::handle_response(http::get_request(client, "/find/file", Some(&query))?)
}

pub fn find_symbols(
    client: &Client,
    q: Value,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("query".into(), q);
    utils::validate_required(&query, &["query"])?;
    if let Some(params) = params {
        query.extend(params.clone());
    }
    utils::handle_response(http::get_request(client, "/find/symbol", Some(&query))?)
}
