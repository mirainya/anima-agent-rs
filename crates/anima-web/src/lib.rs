pub mod jobs;
pub mod jobs_derive;
pub mod jobs_store;
pub mod jobs_types;
pub mod routes;
pub mod routes_commands;
pub mod routes_queries;
pub mod routes_static;
pub mod sse;
pub mod web_channel;
pub mod web_snapshot;

use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    pub runtime: Mutex<anima_runtime::bootstrap::RuntimeBootstrap>,
    pub bus: Arc<anima_runtime::bus::Bus>,
    pub web_channel: Arc<web_channel::WebChannel>,
    pub jobs: Mutex<jobs::JobStore>,
}
