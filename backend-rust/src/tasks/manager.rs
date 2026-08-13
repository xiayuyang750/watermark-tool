//! 任务管理器：tokio worker + 异步队列 + SQLite 持久化。
//! 逻辑与 Python 版 backend/app/tasks/manager.py 一致（线程 worker → tokio 异步等价）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, REFERER, USER_AGENT};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::db::DB;
use crate::config::load_config;
use crate::schemas::TaskRead;

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// 识别链接所属平台（与 Python detect_platform 一致）。
pub fn detect_platform(url: &str) -> &'static str {
    let u = url.to_lowercase();
    if u.contains("douyin.com") || u.contains("iesdouyin") || u.contains("v.douyin") {
        "douyin"
    } else if u.contains("x.com") || u.contains("twitter.com") {
        "x"
    } else {
        ""
    }
}

/// 按 CDN 域名选择合法 Referer（与 Python _referer_for 一致）。
pub fn _referer_for(url: &str) -> &'static str {
    if url.contains("twimg.com") {
        return "https://x.com/";
    }
    "https://www.douyin.com/"
}

fn _sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\r' | '\n' => '_',
            c => c,
        })
        .collect();
    let t = cleaned.trim();
    if t.is_empty() {
        "untitled".to_string()
    } else {
        t.chars().take(60).collect()
    }
}

fn _ext_from_url(url: &str) -> &'static str {
    let path = url.split('?').next().unwrap_or("").to_lowercase();
    for e in [".webp", ".png", ".gif", ".jpeg", ".jpg"] {
        if path.ends_with(e) {
            return e;
        }
    }
    ".jpg"
}

#[derive(Clone)]
struct TaskInner {
    id: String,
    task_type: String,
    status: String,
    progress: i32,
    output: Option<String>,
    error: Option<String>,
    created_at: String,
    url: Option<String>,
    options: Value,
    cancelled: bool,
}

pub struct TaskManager {
    db: DB,
    tx: mpsc::Sender<String>,
    tasks: Mutex<HashMap<String, TaskInner>>,
}

impl TaskManager {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<String>(1024);
        let mgr = Arc::new(TaskManager {
            db: DB::new(),
            tx,
            tasks: Mutex::new(HashMap::new()),
        });
        mgr.load_from_db();
        let worker = mgr.clone();
        tokio::spawn(async move {
            worker.run(rx).await;
        });
        mgr
    }

    // ---- 外部接口（与 Python 一致） ----

    pub async fn create(
        self: &Arc<Self>,
        task_type: &str,
        url: Option<&str>,
        options: Value,
    ) -> TaskRead {
        let tid = Uuid::new_v4().simple().to_string()[..12].to_string();
        let created_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let info = TaskInner {
            id: tid.clone(),
            task_type: task_type.to_string(),
            status: "pending".to_string(),
            progress: 0,
            output: None,
            error: None,
            created_at: created_at.clone(),
            url: url.map(|s| s.to_string()),
            options,
            cancelled: false,
        };
        let db_options = info.options.clone();
        {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(tid.clone(), info);
        }
        let _ = self.db.upsert(
            &tid,
            task_type,
            "pending",
            0,
            None,
            None,
            &created_at,
            &db_options,
        );
        let _ = self.tx.send(tid.clone()).await;
        self.public_view(&tid).unwrap()
    }

    pub fn get(&self, tid: &str) -> Option<TaskRead> {
        self.public_view(tid)
    }

    pub fn list(&self) -> Vec<TaskRead> {
        let tasks = self.tasks.lock().unwrap();
        let mut ids: Vec<&TaskInner> = tasks.values().collect();
        ids.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        ids.into_iter().map(|t| t.into()).collect()
    }

    pub fn cancel(&self, tid: &str) -> bool {
        let mut tasks = self.tasks.lock().unwrap();
        match tasks.get_mut(tid) {
            Some(info) if info.status == "pending" || info.status == "running" => {
                info.cancelled = true;
                true
            }
            _ => false,
        }
    }

    pub fn public_view(&self, tid: &str) -> Option<TaskRead> {
        let tasks = self.tasks.lock().unwrap();
        tasks.get(tid).map(|t| t.into())
    }

    fn update(&self, tid: &str, status: Option<&str>, progress: Option<i32>, output: Option<&str>, error: Option<&str>) {
        {
            let mut tasks = self.tasks.lock().unwrap();
            if let Some(info) = tasks.get_mut(tid) {
                if let Some(v) = status {
                    info.status = v.to_string();
                }
                if let Some(v) = progress {
                    info.progress = v;
                }
                if let Some(v) = output {
                    info.output = Some(v.to_string());
                }
                if let Some(v) = error {
                    info.error = Some(v.to_string());
                }
            }
        }
        let _ = self.db.update(tid, status, progress, output, error);
    }

    // ---- 内部 ----

    fn load_from_db(&self) {
        let rows = self.db.list_all().unwrap_or_default();
        let mut tasks = self.tasks.lock().unwrap();
        for mut row in rows {
            if row.status == "pending" || row.status == "running" {
                row.status = "failed".to_string();
                let err = Some("应用重启，任务中断".to_string());
                row.error = err.clone();
                let _ = self.db.update(&row.id, Some("failed"), None, None, err.as_deref());
            }
            tasks.insert(
                row.id.clone(),
                TaskInner {
                    id: row.id,
                    task_type: row.task_type,
                    status: row.status,
                    progress: row.progress,
                    output: row.output,
                    error: row.error,
                    created_at: row.created_at,
                    url: None,
                    options: serde_json::json!({}),
                    cancelled: false,
                },
            );
        }
    }

    async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<String>) {
        while let Some(tid) = rx.recv().await {
            let cancelled = {
                let tasks = self.tasks.lock().unwrap();
                tasks.get(&tid).map(|t| t.cancelled).unwrap_or(false)
            };
            if !cancelled {
                self.process(&tid).await;
            }
        }
    }

    async fn process(&self, tid: &str) {
        self.update(tid, Some("running"), Some(5), None, None);
        let outcome = self.process_inner(tid).await;
        match outcome {
            Ok(()) => {}
            Err((cancelled, err)) => {
                if cancelled {
                    self.update(tid, Some("cancelled"), None, None, Some("用户取消"));
                } else {
                    self.update(tid, Some("failed"), None, None, Some(&err));
                }
            }
        }
    }

    async fn process_inner(&self, tid: &str) -> Result<(), (bool, String)> {
        let (url, options, task_type) = {
            let tasks = self.tasks.lock().unwrap();
            let info = tasks.get(tid).unwrap();
            (info.url.clone(), info.options.clone(), info.task_type.clone())
        };
        let url = url.unwrap_or_default();
        let cfg = load_config();
        let out_dir = options
            .get("output_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&cfg.output_dir));
        std::fs::create_dir_all(&out_dir).map_err(|e| (false, e.to_string()))?;

        if task_type == "direct" {
            // 直链下载：url 为素材直链，options 携带 kind/title/image_url
            let kind = options.get("kind").and_then(|v| v.as_str()).unwrap_or("video").to_string();
            let title = options.get("title").and_then(|v| v.as_str()).unwrap_or("untitled").to_string();
            let out = self
                .download(&url, &out_dir, &title, &kind, tid, 15, 99, options.get("image_url").and_then(|v| v.as_str()))
                .await?;
            let output = out.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join(" | ");
            self.update(tid, Some("done"), Some(100), Some(&output), None);
            return Ok(());
        }

        // link 任务：解析后下载全部素材（M1 骨架解析尚未接入，将返回占位错误）
        let platform = detect_platform(&url);
        if platform.is_empty() {
            return Err((false, "无法识别的平台链接".to_string()));
        }
        let remove_platform_wm = options
            .get("remove_platform_wm")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let result = crate::parsers::parse(platform, &url, remove_platform_wm)
            .await
            .map_err(|e| (self.is_cancelled(tid), e))?;
        self.update(tid, None, Some(15), None, None);

        if result.files.is_empty() {
            return Err((self.is_cancelled(tid), "解析结果为空".to_string()));
        }
        let total = result.files.len();
        let mut paths: Vec<String> = Vec::new();
        for (i, f) in result.files.iter().enumerate() {
            let start_pct = 15 + (i * 80) / total;
            let end_pct = 15 + ((i + 1) * 80) / total;
            let name = if total == 1 {
                result.title.clone()
            } else {
                format!("{}_{}", result.title, i + 1)
            };
            let out = self
                .download(&f.url, &out_dir, &name, &f.kind, tid, start_pct as i32, end_pct as i32, f.image_url.as_deref())
                .await?;
            paths.extend(out.iter().map(|p| p.to_string_lossy().to_string()));
        }
        self.update(tid, Some("done"), Some(100), Some(&paths.join(" | ")), None);
        Ok(())
    }

    async fn download(
        &self,
        url: &str,
        out_dir: &Path,
        title: &str,
        kind: &str,
        tid: &str,
        start_pct: i32,
        end_pct: i32,
        image_url: Option<&str>,
    ) -> Result<Vec<PathBuf>, (bool, String)> {
        // Live 图（带 image_url）：下载「静态照片 + 视频」双文件
        if kind == "livephoto" {
            if let Some(img) = image_url {
                let mid = start_pct + (end_pct - start_pct) * 2 / 3;
                let video = Box::pin(self.download(url, out_dir, title, "video", tid, start_pct, mid, None)).await?;
                let photo = Box::pin(self.download(img, out_dir, title, "image", tid, mid, end_pct, None)).await?;
                let mut paths = video;
                paths.extend(photo);
                return Ok(paths);
            }
        }
        let ext = if kind == "video" || kind == "livephoto" {
            ".mp4"
        } else if kind == "gif" {
            ".gif"
        } else {
            _ext_from_url(url)
        };
        let filename = format!(
            "{}_{}{}",
            _sanitize_filename(title),
            Local::now().format("%Y%m%d%H%M%S"),
            ext
        );
        let out_path = out_dir.join(filename);

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        headers.insert(REFERER, HeaderValue::from_static(_referer_for(url)));

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::default())
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| (false, e.to_string()))?;
        let resp = client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| (false, format!("下载请求失败：{e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err((false, format!("下载失败：HTTP {}", status)));
        }
        let total_len = resp.content_length().unwrap_or(0);
        let mut file = tokio::fs::File::create(&out_path)
            .await
            .map_err(|e| (false, format!("创建文件失败：{e}")))?;
        let mut stream = resp.bytes_stream();
        let mut written: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| (false, format!("读取数据失败：{e}")))?;
            let cancelled = self.is_cancelled(tid);
            if cancelled {
                return Err((true, "cancelled".to_string()));
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| (false, format!("写入文件失败：{e}")))?;
            written += chunk.len() as u64;
            if total_len > 0 {
                let pct = start_pct + (written as f64 / total_len as f64 * (end_pct - start_pct) as f64) as i32;
                let pct = pct.clamp(start_pct, end_pct);
                self.update(tid, None, Some(pct), None, None);
            }
        }
        Ok(vec![out_path])
    }

    fn is_cancelled(&self, tid: &str) -> bool {
        self.tasks
            .lock()
            .unwrap()
            .get(tid)
            .map(|t| t.cancelled)
            .unwrap_or(false)
    }
}

impl From<&TaskInner> for TaskRead {
    fn from(t: &TaskInner) -> Self {
        TaskRead {
            id: t.id.clone(),
            task_type: t.task_type.clone(),
            status: t.status.clone(),
            progress: t.progress,
            output: t.output.clone(),
            error: t.error.clone(),
            created_at: t.created_at.clone(),
        }
    }
}
