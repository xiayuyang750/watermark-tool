//! CDP 浏览器管理器（对齐 Python backend/app/parsers/douyin_browser.py 的常驻复用逻辑）。
//!
//! 用系统 Edge 替代 Playwright/Chromium：
//! - 启动：`msedge --remote-debugging-port=<port> --user-data-dir=<profile> --headless=new`
//! - 连接：HTTP `GET /json/version` 就绪检测 + `/json/list` 获取页面 target WebSocket 地址
//! - 命令子集：`Runtime.evaluate`（页面内 XHR 解析）、`Page.reload`（失效重建）
//!
//! 常驻复用：首次解析启动并打开抖音首页，后续解析直接复用页面内 XHR；
//! 超过 IDLE_TTL 未使用自动关闭（释放内存），页面失效自动重建，无需用户介入。

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::config::data_dir;
use crate::tasks::manager::UA;

/// 空闲自动关闭时长（10 分钟，与 Python 版一致）
const IDLE_TTL: Duration = Duration::from_secs(600);
const HOME_URL: &str = "https://www.douyin.com/";

pub struct CdpBrowser {
    port: AtomicU16,
    profile_dir: PathBuf,
    child: Mutex<Option<Child>>,
    ws_url: Mutex<Option<String>>, // 常驻页面 target 的 WebSocket 调试地址
    last_used: Mutex<Instant>,
}

impl CdpBrowser {
    pub fn new() -> Self {
        CdpBrowser {
            port: AtomicU16::new(0),
            profile_dir: data_dir().join("cdp_profile"),
            child: Mutex::new(None),
            ws_url: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        }
    }

    /// 查找可用端口（绑定 0 端口获取，随后释放）
    pub(crate) fn find_free_port() -> u16 {
        use std::net::TcpListener;
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(l) => l.local_addr().map(|a| a.port()).unwrap_or(9222),
            Err(_) => 9222,
        }
    }

    pub(crate) fn edge_path() -> Option<PathBuf> {
        for cand in [
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ] {
            if std::path::Path::new(cand).exists() {
                return Some(PathBuf::from(cand));
            }
        }
        None
    }

    /// 启动 Edge 调试实例并等待端口就绪；返回是否成功。
    /// 注意：可能在 async 上下文调用，内部不得使用 reqwest::blocking。
    fn launch_edge(&self) -> Result<(), String> {
        let edge = Self::edge_path().ok_or("未找到 Edge 浏览器，无法执行抖音解析")?;
        let port = Self::find_free_port();
        let profile = &self.profile_dir;
        if let Err(e) = std::fs::create_dir_all(profile) {
            eprintln!("[cdp] 创建 profile 目录失败 {profile:?}: {e}");
            return Err(format!("创建浏览器数据目录失败：{e}"));
        }

        let spawn_result = Command::new(&edge)
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--headless=new")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-gpu")
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-features=AutomationControlled")
            .arg("--mute-audio")
            // 用命令行参数固定 UA：CDP setUserAgentOverride 不跨连接持久，
            // 抖音会把 headless 特征（HeadlessChrome）识别出来并重定向到验证码页
            .arg(format!("--user-agent={UA}"))
            .arg("about:blank")
            .spawn();
        let child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[cdp] 启动 Edge 失败: {e}");
                return Err(format!("启动 Edge 失败：{e}"));
            }
        };

        self.port.store(port, Ordering::Relaxed);
        *self.child.lock().unwrap() = Some(child);
        Ok(())
    }

    /// 等待 Edge 调试端口就绪（最多 20s）。
    async fn wait_debug_port_ready(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self.debug_port_ready().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        eprintln!("[cdp] Edge 调试端口未就绪");
        false
    }

    async fn debug_port_ready(&self) -> bool {
        let port = self.port.load(Ordering::Relaxed);
        if port == 0 {
            return false;
        }
        let url = format!("http://127.0.0.1:{port}/json/version");
        match reqwest_get(&url).await {
            Ok(status) => status == 200,
            Err(_) => false,
        }
    }

    /// 获取当前常驻页面的 WebSocket 调试地址（按 first target）。
    async fn current_target_ws(&self) -> Option<String> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port.load(Ordering::Relaxed));
        let text = reqwest_text(&url).await?;
        let list: Value = serde_json::from_str(&text).ok()?;
        let arr = list.as_array()?;
        let target = arr.iter().find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))?;
        target.get("webSocketDebuggerUrl").and_then(|v| v.as_str()).map(String::from)
    }

    /// 通过 CDP 执行 JS，返回 JSON 结果（对应 Python page.evaluate）。
    async fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let ws_url = {
            let guard = self.ws_url.lock().unwrap();
            guard.clone().ok_or("浏览器页面未就绪")?
        };
        let (mut ws, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("连接调试页面失败：{e}"))?;
        let cmd = json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {"expression": expression, "returnByValue": true}
        });
        ws.send(Message::Text(cmd.to_string().into()))
            .await
            .map_err(|e| format!("发送命令失败：{e}"))?;
        // 读取响应直至匹配 id=1
        loop {
            let msg = ws.next().await.ok_or("调试连接中断")?;
            let msg = msg.map_err(|e| format!("读取响应失败：{e}"))?;
            if let Message::Text(t) = msg {
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    if v.get("id").and_then(|x| x.as_u64()) == Some(1) {
                        return Ok(v);
                    }
                }
            }
        }
    }

    /// 页面内同步 XHR 调用 detail 接口（对应 Python _fetch_detail 的脚本）。
    /// 返回 (HTTP 状态, 响应文本)。
    async fn page_fetch(&self, api_url: &str) -> Result<(u16, String), String> {
        let expr = format!(
            "(function(){{try{{var xhr = new XMLHttpRequest();xhr.open('GET', {}, false);xhr.send();return JSON.stringify({{status:xhr.status, text:xhr.responseText}});}}catch(e){{return JSON.stringify({{error:String(e)}});}}}})()",
            serde_json::to_string(api_url).map_err(|e| e.to_string())?
        );
        let resp = self.evaluate(&expr).await?;
        // Runtime.evaluate 结果在 result.result.value
        let value = resp
            .get("result")
            .and_then(|r| r.get("result"))
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parsed: Value = serde_json::from_str(&value).unwrap_or(Value::Null);
        let status = parsed.get("status").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
        let text = parsed
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(e) = parsed.get("error").and_then(|v| v.as_str()) {
            eprintln!("[cdp] 页面 XHR 异常: {e}");
        }
        Ok((status, text))
    }

    /// 刷新常驻页面并等待安全 SDK 重新注入（对应 Python page.reload + wait_for_timeout）。
    async fn reload_page(&self) -> Result<(), String> {
        let ws_url = self.ws_url.lock().unwrap().clone().ok_or("浏览器页面未就绪")?;
        let (mut ws, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("连接调试页面失败：{e}"))?;
        let cmd = json!({"id": 1, "method": "Page.reload", "params": {}});
        ws.send(Message::Text(cmd.to_string().into()))
            .await
            .map_err(|e| format!("刷新命令失败：{e}"))?;
        drop(ws);
        tokio::time::sleep(Duration::from_millis(2500)).await;
        Ok(())
    }

    /// 页面是否存活（执行轻量 JS 探测，对应 Python _ensure_page 的 evaluate readyState 检查）。
    async fn page_alive(&self) -> bool {
        match self.evaluate("document.readyState").await {
            Ok(v) => v
                .get("result")
                .and_then(|r| r.get("result"))
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
                .map(|s| s == "complete" || s == "interactive" || s == "loading")
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// 确保常驻浏览器与抖音首页就绪；失效或空闲超时自动重建。返回页面是否就绪。
    pub async fn ensure_page(&self) -> Result<(), String> {
        // 已有页面且存活且未空闲超时 → 复用
        if self.ws_url.lock().unwrap().is_some()
            && self.debug_port_ready().await
            && self.page_alive().await
        {
            if Instant::now() - *self.last_used.lock().unwrap() < IDLE_TTL {
                *self.last_used.lock().unwrap() = Instant::now();
                return Ok(());
            }
            // 空闲超时：关闭旧的，下面重建
            self.teardown();
        } else {
            // 无页面或页面失效：清理后重建
            self.teardown();
        }
        // 启动（或重启）Edge
        if !self.debug_port_ready().await {
            self.launch_edge()?;
            if !self.wait_debug_port_ready().await {
                return Err("Edge 调试端口未就绪".to_string());
            }
        }
        // 获取页面 target 并记录 ws 地址
        let ws = self
            .current_target_ws()
            .await
            .ok_or("无法获取 Edge 页面调试地址")?;
        *self.ws_url.lock().unwrap() = Some(ws);
        // 打开抖音首页（新开的 target 默认 about:blank，需导航）
        self.navigate(HOME_URL).await?;
        // 首次创建：等安全 SDK 注入完成，避免首个请求落空重载
        tokio::time::sleep(Duration::from_millis(2500)).await;
        *self.last_used.lock().unwrap() = Instant::now();
        Ok(())
    }

    async fn navigate(&self, url: &str) -> Result<(), String> {
        let ws_url = self.ws_url.lock().unwrap().clone().ok_or("浏览器页面未就绪")?;
        let (mut ws, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("连接调试页面失败：{e}"))?;
        // UA 已通过启动参数 --user-agent 固定（避免 CDP 跨连接不持久的问题）
        let nav = json!({"id": 1, "method": "Page.navigate", "params": {"url": url}});
        ws.send(Message::Text(nav.to_string().into()))
            .await
            .map_err(|e| format!("导航命令失败：{e}"))?;
        drop(ws);
        Ok(())
    }

    /// 页面内解析 detail 接口（对应 Python _fetch_detail 的重试逻辑）。
    pub async fn fetch_detail(&self, api_url: &str) -> Result<String, String> {
        self.ensure_page().await?;
        // 偶发空响应（风控抖动/SDK 未就绪）：重试，空则刷新页面再试
        let mut text = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                self.reload_page().await?;
            }
            let (status, body) = self.page_fetch(api_url).await?;
            eprintln!("[cdp] detail 接口 attempt={attempt} status={status} len={}", body.len());
            if status == 200 && !body.trim().is_empty() {
                text = body;
                break;
            }
        }
        if text.is_empty() {
            return Err("抖音接口返回空数据（可能需要登录抖音）".to_string());
        }
        Ok(text)
    }

/// 关闭常驻浏览器（应用退出或空闲超时调用）。
    pub fn teardown(&self) {
        *self.ws_url.lock().unwrap() = None;
        let mut child = self.child.lock().unwrap();
        if let Some(mut c) = child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.port.store(0, Ordering::Relaxed);
    }
}

impl Drop for CdpBrowser {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// reqwest async 辅助（调试端口就绪检测/获取 target 列表用）。
async fn reqwest_get(url: &str) -> Result<u16, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

async fn reqwest_text(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    client.get(url).send().await.ok()?.text().await.ok()
}
