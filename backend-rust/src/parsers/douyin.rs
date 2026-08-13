//! 抖音解析器（对齐 Python backend/app/parsers/douyin.py + douyin_browser.py）。
//!
//! 链路：分享文本 → 提取 URL → 短链重定向得作品 ID → CDP 浏览器页面内 XHR 调
//! `aweme/v1/web/aweme/detail` 接口（安全 SDK 自动签名，规避纯 HTTP 反爬）
//! → 按内容类型返回原生素材：视频 / 图集 / 动图 / Live 图。
//!
//! 平台水印去除：视频播放地址 playwm→play；Live 图剥离 watermark/logo_name 参数。

use regex::Regex;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::schemas::{MediaFile, ParseResult};
use crate::tasks::manager::UA;

use super::cdp::CdpBrowser;

/// 常驻浏览器单例：所有解析串行复用同一页面（跨 await 安全）。
pub struct DouyinParser {
    browser: Mutex<CdpBrowser>,
}

impl DouyinParser {
    pub fn new() -> Self {
        DouyinParser {
            browser: Mutex::new(CdpBrowser::new()),
        }
    }

    pub async fn parse(&self, url: &str, remove_platform_wm: bool) -> Result<ParseResult, String> {
        let (aweme_id, _is_note) = resolve_aweme_id(url).await?;
        let browser = self.browser.lock().await;
        let detail = fetch_detail(&browser, &aweme_id).await?;
        let aweme = detail.get("aweme_detail").cloned().unwrap_or(Value::Null);
        if aweme.is_null() {
            return Err("抖音接口未返回作品信息：可能未登录或作品已删除".to_string());
        }
        parse_aweme(&aweme, remove_platform_wm)
    }

    /// 关闭常驻浏览器（应用退出时调用）。
    pub async fn teardown(&self) {
        self.browser.lock().await.teardown();
    }
}

impl Default for DouyinParser {
    fn default() -> Self {
        Self::new()
    }
}

// ---- 分享链接解析（纯 HTTP，对应 Python _resolve_aweme_id） ----

pub async fn resolve_aweme_id(share_text: &str) -> Result<(String, bool), String> {
    let url = extract_url(share_text)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::default())
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("User-Agent", UA)
        .header("Referer", "https://www.douyin.com/")
        .send()
        .await
        .map_err(|e| format!("短链请求失败：{e}"))?;
    let final_url = resp.url().to_string();
    let re_video = Regex::new(r"/(video|note)/(\d+)").unwrap();
    if let Some(c) = re_video.captures(&final_url) {
        return Ok((c.get(2).unwrap().as_str().to_string(), c.get(1).unwrap().as_str() == "note"));
    }
    let re_detail = Regex::new(r"/aweme/detail/(\d+)").unwrap();
    if let Some(c) = re_detail.captures(&final_url) {
        return Ok((c.get(1).unwrap().as_str().to_string(), false));
    }
    let re_raw = Regex::new(r"\b(\d{15,21})\b").unwrap();
    if let Some(c) = re_raw.captures(&final_url) {
        return Ok((c.get(1).unwrap().as_str().to_string(), false));
    }
    Err("无法从链接中提取作品 ID，请确认是抖音分享链接".to_string())
}

fn extract_url(text: &str) -> Result<String, String> {
    let re = Regex::new(r"https?://\S+").unwrap();
    let m = re.find(text).ok_or("未在输入中找到可用的链接")?;
    Ok(m.as_str().trim_end_matches(['，', '。', '；', ';', '.']).to_string())
}

// ---- detail 接口（CDP 浏览器页面内 XHR，对应 Python douyin_browser._fetch_detail） ----

const API_DETAIL: &str = "https://www.douyin.com/aweme/v1/web/aweme/detail/";

/// 与 Python BASE_PARAMS 保持一致的 Web 接口基础参数（顺序一致）。
const BASE_PARAMS: &[(&str, &str)] = &[
    ("device_platform", "webapp"),
    ("aid", "6383"),
    ("channel", "channel_pc_web"),
    ("pc_client_type", "1"),
    ("version_code", "190500"),
    ("version_name", "19.5.0"),
    ("cookie_enabled", "true"),
    ("browser_language", "zh-CN"),
    ("browser_platform", "Win32"),
    ("browser_name", "Edge"),
    ("browser_online", "true"),
    ("engine_name", "Blink"),
    ("os_name", "Windows"),
    ("os_version", "10"),
    ("platform", "PC"),
    ("screen_width", "1920"),
    ("screen_height", "1080"),
];

fn build_api_url(aweme_id: &str) -> String {
    let mut params = BASE_PARAMS
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    params.push_str(&format!("&aweme_id={aweme_id}"));
    format!("{API_DETAIL}?{params}")
}

async fn fetch_detail(browser: &CdpBrowser, aweme_id: &str) -> Result<Value, String> {
    let api_url = build_api_url(aweme_id);
    let text = browser.fetch_detail(&api_url).await?;
    serde_json::from_str::<Value>(&text)
        .map_err(|_| "抖音接口返回非 JSON 数据（可能触发风控）".to_string())
}

// ---- 内容分类（对应 Python parse_aweme / image_file） ----

pub fn parse_aweme(aweme: &Value, remove_platform_wm: bool) -> Result<ParseResult, String> {
    let title = aweme
        .get("desc")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let title = if title.is_empty() { "未命名作品".to_string() } else { title };

    let images = aweme.get("images").and_then(|v| v.as_array());
    if let Some(imgs) = images {
        let mut files = Vec::new();
        for img in imgs {
            if let Some(f) = image_file(img, remove_platform_wm) {
                files.push(f);
            }
        }
        if files.is_empty() {
            return Err("图集作品未包含图片地址".to_string());
        }
        return Ok(ParseResult {
            platform: "douyin".to_string(),
            title,
            media_type: "image".to_string(),
            files,
        });
    }

    let video = aweme.get("video").cloned().unwrap_or(Value::Null);
    let play_addr = video.get("play_addr").cloned().unwrap_or(Value::Null);
    let url_list = play_addr.get("url_list").and_then(|v| v.as_array());
    let url = url_list.and_then(|a| a.first()).and_then(|v| v.as_str()).map(|s| s.to_string());
    let Some(url) = url else {
        return Err("视频作品未包含播放地址".to_string());
    };
    let url = url.replace("http://", "https://");
    let url = if remove_platform_wm { url.replace("playwm", "play") } else { url };
    let cover = video
        .get("cover")
        .and_then(|c| c.get("url_list"))
        .and_then(|l| l.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| s.replace("http://", "https://"))
        .filter(|s| !s.is_empty());
    Ok(ParseResult {
        platform: "douyin".to_string(),
        title,
        media_type: "video".to_string(),
        files: vec![MediaFile {
            kind: "video".to_string(),
            url,
            label: None,
            cover,
            image_url: None,
        }],
    })
}

/// 单张图片：Live 图 > 动图 > 静态图（对应 Python image_file）。
fn image_file(img: &Value, remove_platform_wm: bool) -> Option<MediaFile> {
    // Live 图：带 video 字段（图片会动）→ 静态照片 + 3 秒视频双文件
    let video = img.get("video").cloned().unwrap_or(Value::Null);
    if !video.is_null() {
        let play_addr = video.get("play_addr").cloned().unwrap_or(Value::Null);
        let urls = play_addr.get("url_list").and_then(|v| v.as_array());
        if let Some(url) = urls.and_then(|a| a.first()).and_then(|v| v.as_str()) {
            let url = url.replace("http://", "https://");
            let url = if remove_platform_wm { url.replace("playwm", "play") } else { url };
            let cover = video
                .get("cover")
                .and_then(|c| c.get("url_list"))
                .and_then(|l| l.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(|s| s.replace("http://", "https://"))
                .filter(|s| !s.is_empty());
            // 静态照片直链：Live 图由「照片 + 视频」组成，缺一不可
            let photo = img
                .get("url_list")
                .and_then(|l| l.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(|s| s.replace("http://", "https://"))
                .filter(|s| !s.is_empty());
            if photo.is_some() {
                return Some(MediaFile {
                    kind: "livephoto".to_string(),
                    url,
                    label: None,
                    cover,
                    image_url: photo,
                });
            }
        }
    }
    // 动图：animated/gif 字段
    for field in ["animated_url_list", "gif_url_list", "animated_url", "gif_url"] {
        if let Some(v) = img.get(field) {
            let url = if let Some(s) = v.as_str() {
                if s.is_empty() { None } else { Some(s.to_string()) }
            } else if let Some(a) = v.as_array() {
                a.first().and_then(|x| x.as_str()).map(String::from)
            } else {
                None
            };
            if let Some(u) = url {
                return Some(MediaFile {
                    kind: "gif".to_string(),
                    url: u,
                    label: None,
                    cover: None,
                    image_url: None,
                });
            }
        }
    }
    // 静态图
    let urls = img.get("url_list").and_then(|v| v.as_array());
    let url = urls.and_then(|a| a.first()).and_then(|v| v.as_str()).map(String::from)?;
    Some(MediaFile {
        kind: "image".to_string(),
        url: url.replace("http://", "https://"),
        label: None,
        cover: None,
        image_url: None,
    })
}
