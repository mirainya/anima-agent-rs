pub mod jobs;
pub mod routes;
pub mod sse;
pub mod web_channel;

use std::sync::Arc;

pub struct AppState {
    pub runtime: std::sync::Mutex<anima_runtime::bootstrap::RuntimeBootstrap>,
    pub bus: Arc<anima_runtime::bus::Bus>,
    pub web_channel: Arc<web_channel::WebChannel>,
    pub jobs: std::sync::Mutex<jobs::JobStore>,
}
