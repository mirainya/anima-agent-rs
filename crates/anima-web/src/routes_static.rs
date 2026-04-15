use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use std::path::{Component, Path as StdPath, PathBuf};

static FRONTEND_BUILD_REQUIRED_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Anima Agent</title>
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      font-family: 'Segoe UI', system-ui, sans-serif;
      background: #0f0f13;
      color: #e0e0e8;
    }
    .panel {
      max-width: 560px;
      padding: 24px 28px;
      border-radius: 14px;
      border: 1px solid #2a2a3a;
      background: #1a1a24;
      box-shadow: 0 10px 30px rgba(0,0,0,0.25);
    }
    h1 { margin: 0 0 12px; font-size: 20px; }
    p { margin: 8px 0; line-height: 1.6; color: #b8b8c8; }
    code {
      display: inline-block;
      margin-top: 8px;
      padding: 2px 8px;
      border-radius: 999px;
      background: rgba(162, 155, 254, 0.15);
      color: #c8c4ff;
    }
  </style>
</head>
<body>
  <main class="panel">
    <h1>前端构建产物不存在</h1>
    <p>当前 anima-web 已切换为 React + Vite 工作台。请先构建前端资源后再刷新页面。</p>
    <p><code>cd crates/anima-web/frontend && npm install && npm run build</code></p>
  </main>
</body>
</html>
"#;

fn frontend_dist_dir() -> PathBuf {
    StdPath::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("static")
        .join("dist")
}

fn frontend_dist_index_path() -> PathBuf {
    frontend_dist_dir().join("index.html")
}

async fn render_frontend_index() -> Response {
    match tokio::fs::read_to_string(frontend_dist_index_path()).await {
        Ok(content) => Html(content).into_response(),
        Err(_) => Html(FRONTEND_BUILD_REQUIRED_HTML).into_response(),
    }
}

pub async fn index_page() -> Response {
    render_frontend_index().await
}

pub async fn spa_fallback(uri: Uri) -> Response {
    if uri.path().starts_with("/api/") || uri.path().starts_with("/assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }

    render_frontend_index().await
}

pub async fn static_asset(Path(path): Path<String>) -> Response {
    let Some(safe_path) = sanitize_relative_path(&path) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let asset_path = frontend_dist_dir().join("assets").join(safe_path);
    let Ok(bytes) = tokio::fs::read(&asset_path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut response = Response::new(bytes.into_response().into_body());
    let content_type = content_type_for_path(&asset_path);
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn sanitize_relative_path(path: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        return None;
    }

    let mut sanitized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }

    Some(sanitized)
}

fn content_type_for_path(path: &StdPath) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}
