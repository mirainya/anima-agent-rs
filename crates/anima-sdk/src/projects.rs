use anima_types::Result;
use serde_json::{Map, Value};

use crate::{client as http, facade::Client, utils};

pub fn list_projects(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/project", params)?)
}

pub fn current_project(client: &Client, params: Option<&Map<String, Value>>) -> Result<Value> {
    utils::handle_response(http::get_request(client, "/project/current", params)?)
}
