//! X（Twitter）解析器（对齐 Python backend/app/parsers/x.py）。
//!
//! 链路：推文链接 → 提取推文 ID → vxtwitter 优先 / fxtwitter 兜底（公共 API，无需登录）
//! → 登录墙兜底（配置了 x_cookie 时用浏览器带 cookie 抓取，见 cdp 模块）。
//!
//! 媒体：X 的视频 / 动图（GIF）都是 mp4（无平台水印），图片在 pbs.twimg.com；
//! 直链无防盗链、支持 Range，可直接播放与下载，无需本地代理。

use std::process::Command;
use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use regex::Regex;
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{data_dir, load_config};
use crate::parsers::cdp::CdpBrowser;
use crate::schemas::{MediaFile, ParseResult};
use crate::tasks::manager::UA;
use crate::x_login::{current_target_ws, debug_port_ready};

const VX_API: &str = "https://api.vxtwitter.com/i/status/{tid}";
const FX_API: &str = "https://api.fxtwitter.com/status/{tid}";

fn tweet_id_re() -> Regex {
    Regex::new(r"(?:x|twitter)\.com/(?:[^/?#]+/status/|i/status/)(\d{15,20})").unwrap()
}

fn raw_id_re() -> Regex {
    Regex::new(r"\b(\d{15,20})\b").unwrap()
}

/// video.twimg.com 视频直链需带 tag 签名参数（如 tag=29），否则返回 403。
fn ensure_video_tag(url: &str) -> String {
    if url.contains("tag=") {
        return url.to_string();
    }
    let sep = if url.contains('?') { "&" } else { "?" };
    format!("{url}{sep}tag=29")
}

fn extract_tweet_id(text: &str) -> Result<String, String> {
    if let Some(c) = tweet_id_re().captures(text) {
        return Ok(c.get(1).unwrap().as_str().to_string());
    }
    if let Some(c) = raw_id_re().captures(text) {
        return Ok(c.get(1).unwrap().as_str().to_string());
    }
    Err("无法从链接中提取推文 ID，请确认是 X/Twitter 的推文链接".to_string())
}

/// 解析入口。返回 ParseResult。
pub async fn parse(url: &str, _remove_platform_wm: bool) -> Result<ParseResult, String> {
    let tid = extract_tweet_id(url)?;
    let data = fetch_public(&tid).await;
    let data = match data {
        Some(d) => d,
        None => {
            // 登录墙兜底：浏览器 + x_cookie（M3 CDP 阶段接入）；无 cookie 时直接失败
            fetch_with_cookie(&tid).await?
        }
    };
    to_result(&data, &tid)
}

// ---- 公共解析（vxtwitter 优先，fxtwitter 兜底） ----

async fn fetch_public(tid: &str) -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    // vxtwitter 优先
    let url = VX_API.replace("{tid}", tid);
    if let Ok(resp) = client.get(&url).headers(headers()).send().await {
        if resp.status() == 200 {
            if let Ok(d) = resp.json::<Value>().await {
                if d.get("media_extended").is_some() || d.get("mediaURLs").is_some() {
                    return Some(json!({"source": "vxtwitter", "data": d}));
                }
            }
        }
    }
    // fxtwitter 兜底
    let url = FX_API.replace("{tid}", tid);
    if let Ok(resp) = client.get(&url).headers(headers()).send().await {
        if resp.status() == 200 {
            if let Ok(d) = resp.json::<Value>().await {
                let t = d.get("tweet").cloned().unwrap_or(Value::Null);
                if let Some(media) = t.get("media") {
                    if media.get("all").is_some() {
                        return Some(json!({"source": "fxtwitter", "data": t}));
                    }
                }
            }
        }
    }
    None
}

fn headers() -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
    let mut m = HeaderMap::new();
    m.insert(USER_AGENT, HeaderValue::from_static(UA));
    m.insert(ACCEPT, HeaderValue::from_static("application/json"));
    m
}

// ---- 登录墙兜底（浏览器 + x_cookie；登录态由 x_login.rs 引导流程保存） ----

async fn fetch_with_cookie(tid: &str) -> Result<Value, String> {
    let cookie_str = load_config().x_cookie.trim().to_string();
    if cookie_str.is_empty() {
        return Err("X 解析失败：该推文可能已删除、仅登录可见，或第三方解析服务暂时不可用".to_string());
    }
    // 用固定登录档案启动 headless Edge：登录态持久化，无需反复登录
    let port = CdpBrowser::find_free_port();
    let profile = data_dir().join("x_login_profile");
    let edge = CdpBrowser::edge_path().ok_or("未找到 Edge 浏览器")?;
    let mut child = Command::new(&edge)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--disable-features=AutomationControlled")
        .arg(format!("--user-agent={UA}"))
        .arg("about:blank")
        .spawn()
        .map_err(|e| format!("启动浏览器失败：{e}"))?;

    let result = async {
        // 等待调试端口就绪（最多 20s）
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if debug_port_ready(port).await {
                break;
            }
            if Instant::now() >= deadline {
                return Err("浏览器启动超时".to_string());
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let ws_url = current_target_ws(port).await.ok_or("无法获取浏览器页面调试地址")?;
        let cookies: Value = serde_json::from_str(&cookie_str).unwrap_or(Value::Array(vec![]));
        let gql = fetch_tweet_graphql(&ws_url, &cookies, tid).await?;
        let data = extract_from_graphql(&gql).ok_or_else(|| {
            "该推文没有可下载的视频/图片内容（或登录态已失效，请重新打开 X 登录）".to_string()
        })?;
        Ok(json!({"source": "graphql", "data": data}))
    }
    .await;

    let _ = child.kill();
    let _ = child.wait();
    result
}

/// 打开推文页并监听 TweetResultByRestId GraphQL 响应（对齐 Python _fetch_with_cookie 的 on_response）。
async fn fetch_tweet_graphql(ws_url: &str, cookies: &Value, tid: &str) -> Result<Value, String> {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    let (mut ws, _) = connect_async(ws_url)
        .await
        .map_err(|e| format!("连接调试页面失败：{e}"))?;
    let mut next_id = 1u64;

    // 1. 注入登录 cookie（双保险：档案意外丢失登录态时恢复）。
    //    CDP 只接受标准字段：过滤掉 partitionKey 等私有字段，避免整个请求被拒。
    send_cmd(
        &mut ws,
        next_id,
        "Network.setCookies",
        json!({"cookies": sanitize_cookies(cookies)}),
    )
    .await?;
    next_id += 1;
    // 2. 开启 Network 域以接收响应事件
    send_cmd(&mut ws, next_id, "Network.enable", json!({})).await?;
    next_id += 1;
    // 3. 打开推文页（GraphQL 请求由页面自行发出）
    send_cmd(
        &mut ws,
        next_id,
        "Page.navigate",
        json!({"url": format!("https://x.com/i/status/{tid}")}),
    )
    .await?;
    next_id += 1;

    // 4. 监听响应事件（最多 45s），命中 TweetResultByRestId 即取响应体
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let msg = ws.next().await.ok_or("调试连接中断")?;
        let msg = msg.map_err(|e| format!("读取响应失败：{e}"))?;
        if let Message::Text(t) = msg {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
                if method == "Network.responseReceived" {
                    let url = v.get("params").and_then(|p| p.get("response")).and_then(|r| r.get("url")).and_then(|u| u.as_str()).unwrap_or("");
                    if url.contains("TweetResultByRestId") {
                        let request_id = v.get("params").and_then(|p| p.get("requestId")).and_then(|r| r.as_str()).unwrap_or("");
                        eprintln!("[x] 捕获 TweetResultByRestId 响应 url_len={}", url.len());
                        let resp = send_cmd(
                            &mut ws,
                            next_id,
                            "Network.getResponseBody",
                            json!({"requestId": request_id}),
                        )
                        .await?;
                        next_id += 1;
                        let base64 = resp.get("result").and_then(|r| r.get("base64Encoded")).and_then(|b| b.as_bool()).unwrap_or(false);
                        let body = resp.get("result").and_then(|r| r.get("body")).and_then(|b| b.as_str()).unwrap_or("");
                        let text = if base64 {
                            STANDARD.decode(body).map_err(|e| format!("响应体解码失败：{e}"))?
                        } else {
                            body.as_bytes().to_vec()
                        };
                        let gql: Value = serde_json::from_slice(&text).map_err(|e| format!("GraphQL 响应解析失败：{e}"))?;
                        return Ok(gql);
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            return Err("推文页面加载超时（登录态可能已失效，请重新打开 X 登录）".to_string());
        }
    }
}

/// 过滤 CDP cookie 为只含标准字段（name/value/domain/path/expires/httpOnly/secure/sameSite）。
fn sanitize_cookies(cookies: &Value) -> Value {
    let arr = cookies.as_array().cloned().unwrap_or_default();
    let cleaned: Vec<Value> = arr
        .into_iter()
        .map(|c| {
            let mut o = serde_json::Map::new();
            for k in ["name", "value", "domain", "path", "expires", "httpOnly", "secure", "sameSite"] {
                if let Some(v) = c.get(k) {
                    o.insert(k.to_string(), v.clone());
                }
            }
            Value::Object(o)
        })
        .collect();
    Value::Array(cleaned)
}

/// 发送 CDP 命令并等待同 id 响应（同一连接内保持会话，脚本注册跨命令有效）。
async fn send_cmd<S>(ws: &mut S, id: u64, method: &str, params: Value) -> Result<Value, String>
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    let cmd = json!({"id": id, "method": method, "params": params});
    ws.send(Message::Text(cmd.to_string().into()))
        .await
        .map_err(|e| format!("发送命令失败：{e}"))?;
    loop {
        let msg = ws.next().await.ok_or("调试连接中断")?;
        let msg = msg.map_err(|e| format!("读取响应失败：{e}"))?;
        if let Message::Text(t) = msg {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                    return Ok(v);
                }
            }
        }
    }
}

/// 从 GraphQL TweetResultByRestId 响应提取媒体（转成 vxtwitter 兼容结构）。
fn extract_from_graphql(gql: &Value) -> Option<Value> {
    let res = gql.get("data")?.get("tweetResult")?.get("result")?;
    let mut media: Vec<Value> = Vec::new();
    let details = res
        .get("media_details")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    if details.is_empty() {
        // 旧结构：legacy.extended_entities.media
        let legacy = res.get("legacy").and_then(|l| l.get("extended_entities"));
        if let Some(arr) = legacy.and_then(|l| l.get("media")).and_then(|m| m.as_array()) {
            for m in arr {
                let mtype = m.get("type").and_then(|v| v.as_str()).unwrap_or("photo");
                let mut item = json!({
                    "id_str": m.get("id_str").and_then(|v| v.as_str()).unwrap_or(""),
                    "type": mtype,
                });
                let thumb = m
                    .get("media_url_https")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .replace("http://", "https://");
                if mtype == "photo" {
                    item["url"] = json!(thumb);
                } else {
                    let mut url = String::new();
                    if let Some(variants) = m
                        .get("video_info")
                        .and_then(|v| v.get("variants"))
                        .and_then(|v| v.as_array())
                    {
                        for v in variants {
                            if v.get("content_type").and_then(|c| c.as_str()) == Some("video/mp4") {
                                url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                                break;
                            }
                        }
                    }
                    item["url"] = json!(url);
                }
                item["thumbnail_url"] = json!(thumb);
                media.push(item);
            }
        }
    } else {
        // 新结构：media_details
        for m in details {
            let mtype = m.get("type").and_then(|v| v.as_str()).unwrap_or("photo");
            let mut item = json!({
                "id_str": m.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "type": mtype,
            });
            let thumb = m
                .get("media_url_https")
                .and_then(|v| v.as_str())
                .or_else(|| m.get("thumbnail_url").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();
            if mtype == "photo" {
                item["url"] = json!(m.get("media_url_https").and_then(|v| v.as_str()).unwrap_or(""));
            } else {
                let mut url = String::new();
                if let Some(variants) = m
                    .get("video")
                    .and_then(|v| v.get("variants"))
                    .and_then(|v| v.as_array())
                {
                    for v in variants {
                        if v.get("content_type").and_then(|c| c.as_str()) == Some("video/mp4") {
                            url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
                            break;
                        }
                    }
                }
                item["url"] = json!(url);
            }
            item["thumbnail_url"] = json!(thumb);
            media.push(item);
        }
    }
    if media.is_empty() {
        return None;
    }
    let text = res
        .get("legacy")
        .and_then(|l| l.get("full_text"))
        .and_then(|v| v.as_str())
        .or_else(|| res.get("legacy").and_then(|l| l.get("text")).and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    Some(json!({"media_extended": media, "text": text}))
}

// ---- 转 ParseResult ----

fn to_result(data: &Value, tid: &str) -> Result<ParseResult, String> {
    let source = data.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let d = data.get("data").unwrap_or(&Value::Null);
    let (items, title) = if source == "vxtwitter" {
        let items = d.get("media_extended").cloned().unwrap_or(Value::Array(vec![]));
        let t = (d.get("text").and_then(|v| v.as_str()).unwrap_or("")).trim();
        let title = if t.is_empty() { format!("推文 {tid}") } else { t.to_string() };
        (items, title)
    } else if source == "fxtwitter" {
        let items = d
            .get("media")
            .and_then(|m| m.get("all"))
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        let t = (d.get("text").and_then(|v| v.as_str()).unwrap_or("")).trim();
        let title = if t.is_empty() { format!("推文 {tid}") } else { t.to_string() };
        (items, title)
    } else if source == "graphql" {
        // graphql 已转成 vxtwitter 兼容结构（media_extended + text）
        let items = d.get("media_extended").cloned().unwrap_or(Value::Array(vec![]));
        let t = (d.get("text").and_then(|v| v.as_str()).unwrap_or("")).trim();
        let title = if t.is_empty() { format!("推文 {tid}") } else { t.to_string() };
        (items, title)
    } else {
        return Err("X 解析失败：第三方解析服务返回异常".to_string());
    };

    let mut files: Vec<MediaFile> = Vec::new();
    if let Some(arr) = items.as_array() {
        for it in arr {
            let mtype = it.get("type").and_then(|v| v.as_str()).unwrap_or("photo").to_lowercase();
            let url = it.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if url.is_empty() {
                continue;
            }
            if mtype == "video" || mtype == "gif" {
                // X 的动图（GIF）是无声 mp4，统一按视频处理；视频直链需带 tag 签名参数否则 403
                let cover = it.get("thumbnail_url").and_then(|v| v.as_str()).map(String::from);
                files.push(MediaFile {
                    kind: "video".to_string(),
                    url: ensure_video_tag(&url),
                    label: None,
                    cover,
                    image_url: None,
                });
            } else {
                // photo / 图集
                files.push(MediaFile {
                    kind: "image".to_string(),
                    url,
                    label: None,
                    cover: None,
                    image_url: None,
                });
            }
        }
    }
    if files.is_empty() {
        return Err("该推文没有可下载的视频/图片内容".to_string());
    }
    let media_type = if files[0].kind == "video" { "video" } else { "image" };
    Ok(ParseResult {
        platform: "x".to_string(),
        title,
        media_type: media_type.to_string(),
        files,
    })
}
