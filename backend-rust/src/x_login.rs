//! X 登录引导（对齐 Python backend/app/parsers/x.py 的 start_x_login / wait_x_login）。
//!
//! 流程：启动系统 Edge（调试端口 + 固定登录档案 x_login_profile）打开 X 登录页
//! → 用户在窗口中扫码 / 账号登录 → 后台轮询捕获 auth_token cookie
//! → 保存到 config.json（x_cookie，JSON 数组）并关闭浏览器。
//!
//! 固定档案持久化登录态：后续解析登录墙推文直接复用同一档案，无需反复登录。

use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{data_dir, load_config, save_config};
use crate::parsers::cdp::CdpBrowser;

// 登录流程全局状态：idle / running / done / error
static LOGIN_STATE: Mutex<XLoginState> = Mutex::new(XLoginState {
    status: "idle",
    error: None,
});
// 登录浏览器进程句柄（后台轮询任务检测窗口关闭、结束最终关闭）
static LOGIN_CHILD: Mutex<Option<Child>> = Mutex::new(None);

struct XLoginState {
    status: &'static str,
    error: Option<String>,
}

const LOGIN_URL: &str = "https://x.com/i/flow/login";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// 启动 X 登录引导：弹出 Edge 登录窗口并后台轮询登录结果。
pub fn start_x_login() -> Value {
    {
        let state = LOGIN_STATE.lock().unwrap();
        if state.status == "running" {
            return json!({"ok": false, "error": "已有登录流程正在进行中"});
        }
    }
    // 关闭上次残留的登录实例（同一固定档案），避免档案被占用导致启动失败
    kill_stale_login_edge();
    std::thread::sleep(Duration::from_millis(1000));

    let port = CdpBrowser::find_free_port();
    let profile = data_dir().join("x_login_profile");
    if let Err(e) = std::fs::create_dir_all(&profile) {
        set_state("error", Some(format!("创建登录数据目录失败：{e}")));
        return json!({"ok": false, "error": format!("创建登录数据目录失败：{e}")});
    }
    let edge = match CdpBrowser::edge_path() {
        Some(e) => e,
        None => {
            set_state("error", Some("未找到 Edge 浏览器".to_string()));
            return json!({"ok": false, "error": "未找到 Edge 浏览器，请改用设置中手动粘贴 Cookie"});
        }
    };
    // 有头模式：用户需要在弹出的窗口中完成登录（建议手机 X App 扫码）。
    // 残留实例未退出会导致新实例转发后立即退出，故每次启动前清理并最多重试 3 次
    let mut child = None;
    for attempt in 0..3 {
        kill_stale_login_edge();
        std::thread::sleep(Duration::from_millis(1500));
        match Command::new(&edge)
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-features=AutomationControlled")
            .arg(LOGIN_URL)
            .spawn()
        {
            Ok(c) => {
                child = Some(c);
                break;
            }
            Err(e) => {
                if attempt == 2 {
                    set_state("error", Some(format!("启动 Edge 失败：{e}")));
                    return json!({"ok": false, "error": format!("启动 Edge 失败：{e}")});
                }
            }
        }
    }
    let child = child.unwrap();
    *LOGIN_CHILD.lock().unwrap() = Some(child);
    set_state("running", None);

    tokio::task::spawn(async move {
        let result = wait_x_login(port).await;
        let (status, err) = match result {
            Ok(cookies_json) => {
                let mut cfg = load_config();
                cfg.x_cookie = cookies_json;
                match save_config(&cfg) {
                    Ok(()) => ("done", None),
                    Err(e) => ("error", Some(format!("保存登录信息失败：{e}"))),
                }
            }
            Err(e) => ("error", Some(e)),
        };
        // 登录流程结束：关闭登录浏览器，释放固定档案供解析复用
        if let Some(mut c) = LOGIN_CHILD.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        set_state(status, err);
    });
    json!({"ok": true, "message": "请在 Edge 窗口中完成 X 登录（建议用手机 X App 扫码，比输入账号更稳定）"})
}

/// 查询登录流程状态：idle / running / done / error。
pub fn x_login_status() -> Value {
    let state = LOGIN_STATE.lock().unwrap();
    json!({
        "ok": state.status == "done",
        "status": state.status,
        "error": state.error,
    })
}

// ---- 登录结果轮询 ----

async fn wait_x_login(port: u16) -> Result<String, String> {
    // 等待调试端口就绪（最多 30s，覆盖启动重试时间）
    let ready_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if debug_port_ready(port).await {
            break;
        }
        if Instant::now() >= ready_deadline {
            let hint = if LOGIN_CHILD
                .lock()
                .unwrap()
                .as_mut()
                .map(|c| c.try_wait().ok().flatten().is_some())
                .unwrap_or(false)
            {
                "（浏览器进程提前退出，可能是安全软件拦截，请稍后重试）"
            } else {
                ""
            };
            return Err(format!("浏览器启动超时，未出现登录窗口{hint}"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let timeout = Instant::now() + LOGIN_TIMEOUT;
    let mut ws_url: Option<String> = None;
    loop {
        // 仅当「调试端口连不上」且「spawn 的进程已退出」时才判定窗口关闭：
        // msedge 主进程句柄可能提前退出（实例复用 / 后台模式 / 新版进程模型），
        // 但登录窗口仍正常打开，单靠进程状态会误报「登录窗口已关闭」，
        // 导致用户明明在登录却被判定失败。
        let port_alive = debug_port_ready(port).await;
        let child_exited = LOGIN_CHILD
            .lock()
            .unwrap()
            .as_mut()
            .map(|c| c.try_wait().ok().flatten().is_some())
            .unwrap_or(false);
        if !port_alive && child_exited {
            return Err("登录窗口已关闭，登录未完成".to_string());
        }
        // 每轮刷新页面调试地址（登录导航 / 新标签可能导致旧地址失效）
        if let Some(new_ws) = current_target_ws(port).await {
            ws_url = Some(new_ws);
        }
        // 轮询浏览器 cookie，出现 auth_token 视为登录成功
        if let Some(url) = &ws_url {
            if let Some(cookies) = get_all_cookies(url).await {
                let authed = cookies
                    .as_array()
                    .map(|arr| {
                        arr.iter().any(|c| {
                            c.get("name").and_then(|n| n.as_str()) == Some("auth_token")
                                && c.get("value")
                                    .and_then(|v| v.as_str())
                                    .map(|v| !v.is_empty())
                                    .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
                if authed {
                    return Ok(serde_json::to_string(&cookies).unwrap_or_default());
                }
            }
        }
        if Instant::now() >= timeout {
            return Err("登录超时，请重试".to_string());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// 通过 CDP 获取浏览器全部 cookie（等价 Playwright ctx.cookies()）。
async fn get_all_cookies(ws_url: &str) -> Option<Value> {
    let (mut ws, _) = connect_async(ws_url).await.ok()?;
    let cmd = json!({"id": 1, "method": "Network.getAllCookies", "params": {}});
    ws.send(Message::Text(cmd.to_string().into())).await.ok()?;
    loop {
        let msg = ws.next().await?.ok()?;
        if let Message::Text(t) = msg {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                if v.get("id").and_then(|x| x.as_u64()) == Some(1) {
                    return v.get("result").and_then(|r| r.get("cookies")).cloned();
                }
            }
        }
    }
}

// ---- 通用工具（供 parsers/x.rs 登录墙兜底复用） ----

/// Edge 调试端口是否就绪。
pub(crate) async fn debug_port_ready(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/json/version");
    match reqwest_get(&url).await {
        Ok(status) => status == 200,
        Err(_) => false,
    }
}

/// 当前第一个页面 target 的 WebSocket 调试地址。
pub(crate) async fn current_target_ws(port: u16) -> Option<String> {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let text = reqwest_text(&url).await?;
    let list: Value = serde_json::from_str(&text).ok()?;
    let arr = list.as_array()?;
    let target = arr.iter().find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))?;
    target.get("webSocketDebuggerUrl").and_then(|v| v.as_str()).map(String::from)
}

/// 结束所有命令行含 x_login_profile 的 msedge 进程（进程树 + 二次确认）。
/// Edge 主进程句柄可能提前退出但浏览器实例仍存活（实例复用/转发），
/// 残留实例会占用档案导致新实例启动即退出，必须清理干净。
fn kill_stale_login_edge() {
    let script = r#"
$ps = Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'x_login_profile' }
foreach ($p in $ps) { taskkill /PID $p.ProcessId /T /F 2>$null | Out-Null }
Start-Sleep -Milliseconds 800
$ps = Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'x_login_profile' }
foreach ($p in $ps) { taskkill /PID $p.ProcessId /T /F 2>$null | Out-Null }
"#;
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output();
}

fn set_state(status: &'static str, error: Option<String>) {
    *LOGIN_STATE.lock().unwrap() = XLoginState { status, error };
}

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
