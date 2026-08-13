//! API 请求/响应模型（与 Python 版 backend/app/schemas.py 逐字段对齐）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ParseRequest {
    pub url: String,
    #[serde(default = "default_true")]
    pub remove_platform_wm: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaFile {
    pub kind: String, // video / image / gif / livephoto
    pub url: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cover: Option<String>, // 封面/预览图（视频与 Live 图可选）
    #[serde(default)]
    pub image_url: Option<String>, // Live 图的静态照片直链（与 url 视频组成原生 Live 图）
}

#[derive(Debug, Serialize)]
pub struct ParseResult {
    pub platform: String,
    pub title: String,
    pub media_type: String, // video / image
    pub files: Vec<MediaFile>,
}

#[derive(Debug, Deserialize)]
pub struct TaskCreate {
    #[serde(rename = "type", default = "default_task_type")]
    pub task_type: String, // link / direct
    pub url: Option<String>,
    #[serde(default = "default_options")]
    pub options: serde_json::Value,
}

fn default_task_type() -> String {
    "link".to_string()
}

fn default_options() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    pub content: String,
    pub contact: Option<String>, // 联系方式（邮箱等），便于开发者联系
}

#[derive(Debug, Serialize)]
pub struct TaskRead {
    pub id: String,
    #[serde(rename = "type")]
    pub task_type: String,
    pub status: String, // pending / running / done / failed / cancelled
    pub progress: i32,
    pub output: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
}
