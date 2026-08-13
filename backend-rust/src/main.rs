//! Watermark Tool Rust 后端入口（替代 Python FastAPI，逻辑与 Python 版 backend/app/main.py 一致）。
//! 仅绑定 localhost（本地桌面工具后端）。

mod anticrawler;
mod config;
mod notify;
mod parsers;
mod schemas;
mod tasks;
mod x_login;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::header::{REFERER, USER_AGENT};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::config::{load_config, public_config, update_config};
use crate::schemas::{FeedbackRequest, ParseRequest, TaskCreate};
use crate::tasks::manager::{detect_platform, _referer_for, TaskManager};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Deserialize)]
struct MediaQuery {
    url: Option<String>,
}

#[derive(Deserialize)]
struct DiagnoseQuery {
    url: Option<String>,
}

fn api_error(status: StatusCode, detail: impl Into<String>) -> Response {
    (status, Json(json!({"detail": detail.into()}))).into_response()
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": "0.3.0",
        "platforms": parsers::PARSERS,
    }))
}

async fn parse(Json(req): Json<ParseRequest>) -> Response {
    let platform = detect_platform(&req.url);
    if platform.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "无法识别的平台链接");
    }
    match parsers::parse(platform, &req.url, req.remove_platform_wm).await {
        Ok(result) => Json(json!({
            "platform": result.platform,
            "title": result.title,
            "media_type": result.media_type,
            "files": result.files,
        }))
        .into_response(),
        Err(e) => api_error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

async fn create_task(
    State(mgr): State<Arc<TaskManager>>,
    Json(req): Json<TaskCreate>,
) -> Response {
    if req.task_type != "link" && req.task_type != "direct" {
        return api_error(StatusCode::BAD_REQUEST, format!("任务类型 {} 尚未支持", req.task_type));
    }
    let url = req.url.as_deref().unwrap_or("");
    if url.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "缺少 url");
    }
    let task = mgr.create(&req.task_type, Some(url), req.options).await;
    Json(task).into_response()
}

async fn list_tasks(State(mgr): State<Arc<TaskManager>>) -> Json<Value> {
    Json(json!(mgr.list()))
}

async fn get_task(State(mgr): State<Arc<TaskManager>>, Path(tid): Path<String>) -> Response {
    match mgr.get(&tid) {
        Some(task) => Json(json!(task)).into_response(),
        None => api_error(StatusCode::NOT_FOUND, "任务不存在"),
    }
}

async fn cancel_task(State(mgr): State<Arc<TaskManager>>, Path(tid): Path<String>) -> Response {
    if mgr.cancel(&tid) {
        Json(json!({"ok": true})).into_response()
    } else {
        api_error(StatusCode::BAD_REQUEST, "任务不存在或不可取消")
    }
}

/// 媒体代理：绕过 CDN 防盗链，带 UA/Referer 流式转发，供前端直接播放/显示。
/// 透传浏览器 Range 头并转发上游 206 分段响应，保证视频可拖拽、时长正常显示。
async fn media_proxy(Query(params): Query<MediaQuery>, headers: HeaderMap) -> Response {
    let url = params.url.unwrap_or_default();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return api_error(StatusCode::BAD_REQUEST, "非法媒体地址");
    }
    let mut upstream_headers = HeaderMap::new();
    upstream_headers.insert(USER_AGENT, HeaderValue::from_static(UA));
    upstream_headers.insert(REFERER, HeaderValue::from_static(_referer_for(&url)));
    if let Some(range) = headers.get(header::RANGE) {
        upstream_headers.insert(header::RANGE, range.clone());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::default())
        .timeout(Duration::from_secs(120))
        .build()
        .expect("构建 HTTP 客户端失败");
    let resp = match client.get(&url).headers(upstream_headers).send().await {
        Ok(r) => r,
        Err(e) => return api_error(StatusCode::BAD_GATEWAY, format!("媒体拉取失败：{e}")),
    };
    if !resp.status().is_success() {
        return api_error(StatusCode::BAD_GATEWAY, format!("媒体拉取失败：HTTP {}", resp.status()));
    }
    let status = resp.status();
    let mut builder = Response::builder()
        .status(status)
        .header("cache-control", "public, max-age=86400");
    // 透传关键响应头（content-type / length / range / accept-ranges），保证播放与拖拽
    for h in ["content-type", "content-length", "content-range", "accept-ranges"] {
        if let Some(v) = resp.headers().get(h) {
            builder = builder.header(h, v);
        }
    }
    match builder.body(axum::body::Body::from_stream(resp.bytes_stream())) {
        Ok(body) => body,
        Err(_) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "响应构建失败"),
    }
}

async fn get_config() -> Json<Value> {
    // SMTP 授权码、X 登录 Cookie 等敏感字段不回传前端
    Json(public_config(&load_config()))
}

async fn put_config(Json(body): Json<Value>) -> Json<Value> {
    Json(update_config(&body))
}

async fn feedback(Json(req): Json<FeedbackRequest>) -> Response {
    let content = req.content.trim();
    if content.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "反馈内容不能为空");
    }
    let cfg = load_config();
    match notify::send_feedback_email(&cfg, content, req.contact.as_deref()) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

async fn diagnose(Query(params): Query<DiagnoseQuery>) -> Json<Value> {
    Json(anticrawler::diagnose::run_diagnose(params.url).await)
}

async fn x_login_start() -> Json<Value> {
    Json(x_login::start_x_login())
}

async fn x_login_status_api() -> Json<Value> {
    Json(x_login::x_login_status())
}

/// 优雅关闭后端：先关闭常驻浏览器，再退出进程。
async fn shutdown() -> Json<Value> {
    // 关闭抖音常驻解析浏览器，避免残留 Chromium 进程（与 Python 版一致）
    parsers::teardown().await;
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(500));
        std::process::exit(0);
    });
    Json(json!({"ok": true}))
}

#[tokio::main]
async fn main() {
    let cfg = load_config();
    let port = cfg.backend_port;
    let manager = TaskManager::new();

    // Tauri 生产模式 WebView2 来源为 http://tauri.localhost，需跨域访问本地 API
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("tauri://localhost"),
        ]))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/parse", post(parse))
        .route("/api/v1/tasks", post(create_task).get(list_tasks))
        .route("/api/v1/tasks/{tid}", get(get_task))
        .route("/api/v1/tasks/{tid}/cancel", post(cancel_task))
        .route("/api/v1/media", get(media_proxy))
        .route("/api/v1/config", get(get_config).put(put_config))
        .route("/api/v1/feedback", post(feedback))
        .route("/api/v1/diagnose", get(diagnose))
        .route("/api/v1/x/login/start", post(x_login_start))
        .route("/api/v1/x/login/status", get(x_login_status_api))
        .route("/api/v1/shutdown", post(shutdown))
        .layer(cors)
        .with_state(manager);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect(&format!("端口 {port} 被占用，请检查是否有旧后端进程"));
    println!("Watermark Tool Rust 后端已启动: http://127.0.0.1:{port}");
    axum::serve(listener, app).await.expect("服务异常退出");
}
