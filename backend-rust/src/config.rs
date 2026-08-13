//! 应用配置：工作目录、配置文件读写、默认值。
//! 逻辑与 Python 版 backend/app/config.py 保持一致。

use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_true")]
    pub remove_platform_wm: bool, // 开关1：去平台水印
    #[serde(default)]
    pub remove_content_wm: bool, // 开关2：去内容水印（开发中）
    #[serde(default)]
    pub output_dir: String, // 下载输出目录
    #[serde(default = "default_port")]
    pub backend_port: u16, // 后端端口
    // Bug 反馈邮件（SMTP）；为空则反馈接口返回未配置提示
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_user: String,
    #[serde(default)]
    pub smtp_auth_code: String,
    #[serde(default)]
    pub feedback_to: String,
    // X 平台登录 Cookie（登录墙推文解析用）；敏感字段，不回传前端
    #[serde(default)]
    pub x_cookie: String,
}

fn default_true() -> bool {
    true
}
fn default_port() -> u16 {
    17890
}
fn default_smtp_port() -> u16 {
    465
}

/// 数据目录：默认用户目录；可用 WATERMARK_TOOL_HOME 覆盖（同 Python 版回退逻辑）。
pub fn data_dir() -> PathBuf {
    if let Ok(home) = env::var("WATERMARK_TOOL_HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home);
        }
    }
    let base = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".watermark-tool")
}

fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

/// 确保数据目录下的子目录存在（output/tmp/models，同 Python 版 ensure_dirs）。
pub fn ensure_dirs() {
    for d in ["output", "tmp", "models"] {
        let _ = std::fs::create_dir_all(data_dir().join(d));
    }
}

/// 加载配置：文件存在则读取（缺失字段用默认值），否则写入默认配置。
pub fn load_config() -> Config {
    ensure_dirs();
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let cfg = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Config>(&s).ok())
            .unwrap_or_default()
    } else {
        let cfg = Config::default();
        let _ = save_config(&cfg);
        cfg
    };
    // 与 Python 版一致：读取到的配置原样使用（output_dir 等字段由文件/默认值决定）
    cfg
}

impl Default for Config {
    fn default() -> Self {
        Config {
            remove_platform_wm: true,
            remove_content_wm: false,
            output_dir: data_dir().join("output").to_string_lossy().to_string(),
            backend_port: 17890,
            smtp_host: "smtp.qq.com".to_string(),
            smtp_port: 465,
            smtp_user: String::new(),
            smtp_auth_code: String::new(),
            feedback_to: String::new(),
            x_cookie: String::new(),
        }
    }
}

pub fn save_config(cfg: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// 覆盖更新配置（对应 Python put_config 的 load_config() | cfg，整体合并、保留未知字段）。
pub fn update_config(body: &serde_json::Value) -> serde_json::Value {
    let path = config_path();
    let mut cfg = read_raw_or_default();
    if let Some(obj) = body.as_object() {
        for (k, v) in obj {
            cfg[k.clone()] = v.clone();
        }
    }
    let json = serde_json::to_string_pretty(&cfg).unwrap_or_default();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, &json) {
        eprintln!("[config] 写入配置失败 {path:?}: {e}");
    }
    cfg
}

/// 读取 config.json 原始 JSON；不存在或损坏时返回默认配置 JSON（同 Python 版回退逻辑）。
fn read_raw_or_default() -> serde_json::Value {
    ensure_dirs();
    if let Some(parent) = config_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if config_path().exists() {
        if let Ok(s) = std::fs::read_to_string(config_path()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                return v;
            }
        }
    }
    let cfg = Config::default();
    let _ = save_config(&cfg);
    serde_json::to_value(cfg).unwrap()
}

/// 回传前端的配置（过滤敏感字段：SMTP 授权码、X 登录 Cookie 等，同 Python 版）。
pub fn public_config(cfg: &Config) -> serde_json::Value {
    serde_json::json!({
        "remove_platform_wm": cfg.remove_platform_wm,
        "remove_content_wm": cfg.remove_content_wm,
        "output_dir": cfg.output_dir,
        "backend_port": cfg.backend_port,
    })
}
