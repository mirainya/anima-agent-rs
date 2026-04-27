use anima_types::Result;
use serde_json::{Map, Value};

use crate::{client_async as http, facade_async::AsyncClient, utils};

pub async fn list_projects(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/project", params).await?)
}

pub async fn current_project(
    client: &AsyncClient,
    params: Option<&Map<String, Value>>,
) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/project/current", params).await?)
}
