use anima_runtime::bootstrap::RuntimeBootstrapBuilder;
use anima_web::{routes, sse, web_channel, AppState};
use axum::Router;
use parking_lot::Mutex;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

fn main() {
    let web_channel = Arc::new(web_channel::WebChannel::new());

    let builder = RuntimeBootstrapBuilder::new().with_cli_enabled(false);
    let mut runtime = builder.build();

    // 注册 WebChannel 到 ChannelRegistry
    runtime.registry.register(
        web_channel.clone() as Arc<dyn anima_runtime::channel::Channel>,
        None,
    );

    let bus = runtime.bus.clone();
    runtime.start();

    let state = Arc::new(AppState {
        runtime: Mutex::new(runtime),
        bus: bus.clone(),
        web_channel: web_channel.clone(),
        jobs: Mutex::new(anima_web::jobs::JobStore::default()),
    });

    // 启动 SSE 事件转发线程：监听 Bus internal 通道，推送给浏览器
    sse::start_internal_bus_forwarder(bus.clone(), web_channel.clone());

    // 在独立的 tokio runtime 中运行 axum
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    tokio_rt.block_on(async {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        let app = Router::new()
            .merge(routes::create_routes())
            .layer(cors)
            .with_state(state.clone());

        let addr = "0.0.0.0:3000";
        println!("anima-web 启动在 http://localhost:3000");

        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // 清理
    state.runtime.lock().stop();
}
