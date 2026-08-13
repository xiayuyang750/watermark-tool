//! 风控诊断工具：D1-D8（对齐 Python backend/app/anticrawler/diagnose.py）。
//! M1 骨架：D2/D3/D4 依赖解析器，暂返回「尚未接入」占位项；D1/D5/D6/D7/D8 为真实检查。

use std::time::{Duration, Instant};

use reqwest::header::{REFERER, USER_AGENT};
use serde_json::{json, Value};

use crate::config::data_dir;
use crate::tasks::manager::UA;

const DEFAULT_URL: &str = "https://v.douyin.com/7vbhy1d3U7s/";

fn _item(did: &str, name: &str, level: &str, evidence: Value, suggestion: &str) -> Value {
    json!({"id": did, "name": name, "level": level, "evidence": evidence, "suggestion": suggestion})
}

async fn get(url: &str, timeout: Duration, referer: Option<&str>) -> Result<(u16, String), String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(url).header(USER_AGENT, UA);
    if let Some(r) = referer {
        req = req.header(REFERER, r);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    Ok((status, text))
}

// ---- D1 平台连通性 ----

async fn d1() -> Value {
    match get("https://www.douyin.com/", Duration::from_secs(15), None).await {
        Ok((status, text)) => {
            let head: String = text.chars().take(2000).collect();
            let evi = json!({"status": status, "body_len": text.len()});
            if head.contains("验证码") {
                _item("D1", "平台连通性", "FAIL", json!({"status": status, "body_len": text.len(), "note": "返回验证码页"}), "IP/环境被风控：更换网络、等待冷却或启用代理（L4 需合规评估）")
            } else if status == 200 {
                _item("D1", "平台连通性", "PASS", evi, "")
            } else {
                _item("D1", "平台连通性", "FAIL", evi, "非 200，平台拒绝访问")
            }
        }
        Err(e) => _item("D1", "平台连通性", "FAIL", json!({"error": e}), "网络不可达，检查网络/代理"),
    }
}

// ---- D2 分享链接解析 ----

async fn d2(_url: &str) -> Value {
    _item("D2", "分享链接解析", "WARN", json!({"error": "解析尚未接入（M1 骨架）"}), "短链重定向逻辑将在 M3 里程碑接入")
}

// ---- D3 浏览器解析（主方案） ----

async fn d3(_url: &str) -> (Value, Option<String>) {
    (
        _item("D3", "浏览器解析(主)", "WARN", json!({"error": "解析尚未接入（M1 骨架）"}), "浏览器解析将在 M3 里程碑接入"),
        None,
    )
}

// ---- D4 纯 HTTP 签名（备用方案） ----

async fn d4(_url: &str) -> (Value, Option<String>) {
    (
        _item("D4", "纯HTTP签名(备)", "WARN", json!({"error": "解析尚未接入（M1 骨架）"}), "纯 HTTP 签名解析将在 M2/M3 里程碑接入"),
        None,
    )
}

// ---- D5 CDN 下载 ----

async fn d5(first_url: Option<String>) -> Value {
    let Some(url) = first_url else {
        return _item("D5", "CDN下载", "WARN", json!({"note": "无直链可测（解析未成功）"}), "先修复 D3/D4");
    };
    match get(&url, Duration::from_secs(30), Some("https://www.douyin.com/")).await {
        Ok((status, _)) => {
            let ok = status == 200;
            _item("D5", "CDN下载", if ok { "PASS" } else { "FAIL" }, json!({"status": status}), if ok { "" } else { "CDN 直链被拒或过期，重新解析获取新直链" })
        }
        Err(e) => _item("D5", "CDN下载", "FAIL", json!({"error": e}), "下载超时/失败"),
    }
}

// ---- D6 登录态 ----

fn d6() -> Value {
    let profile = data_dir().join("browser_profile");
    if profile.exists() {
        _item("D6", "登录态", "PASS", json!({"profile": profile.to_string_lossy()}), "")
    } else {
        _item("D6", "登录态", "WARN", json!({"profile": "none"}), "无持久化登录态：公开作品可解析；需登录的私密内容待接入登录流程（L5）")
    }
}

// ---- D7 IP 频率 ----

async fn d7() -> Value {
    let mut statuses: Vec<u16> = Vec::new();
    for _ in 0..3 {
        match get("https://www.douyin.com/", Duration::from_secs(10), None).await {
            Ok((s, _)) => statuses.push(s),
            Err(_) => statuses.push(0),
        }
    }
    let ok = statuses.iter().all(|&h| h == 200);
    _item("D7", "IP频率", if ok { "PASS" } else { "WARN" }, json!({"statuses": statuses}), if ok { "" } else { "出现非 200，疑似频率风控：降低请求频率、加退避（L1）" })
}

// ---- D8 本地链路 ----

async fn d8() -> Value {
    let mut result = json!({});
    let backend = get("http://127.0.0.1:17890/api/v1/health", Duration::from_secs(5), None).await;
    result["backend"] = json!(backend.map(|(s, _)| s).unwrap_or(0));
    let frontend = get("http://localhost:5173/", Duration::from_secs(5), None).await;
    result["frontend"] = json!(frontend.map(|(s, _)| s).unwrap_or(0));
    if result["backend"] == 200 && result["frontend"] == 200 {
        _item("D8", "本地链路", "PASS", result, "")
    } else if result["backend"] == 200 {
        _item("D8", "本地链路", "WARN", result, "前端未启动，运行 scripts/start.ps1")
    } else {
        _item("D8", "本地链路", "FAIL", result, "后端未启动，运行 scripts/start.ps1")
    }
}

// ---- 汇总 ----

pub async fn run_diagnose(url: Option<String>) -> Value {
    let url = url.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_URL.to_string());
    let t0 = Instant::now();
    let (d3, d3_url) = d3(&url).await;
    let (d4, d4_url) = d4(&url).await;
    let items = vec![
        d1().await,
        d2(&url).await,
        d3,
        d4,
        d5(d3_url.or(d4_url)).await,
        d6(),
        d7().await,
        d8().await,
    ];
    let mut counts = json!({"PASS": 0, "WARN": 0, "FAIL": 0});
    for it in &items {
        if let Some(lv) = it.get("level").and_then(|v| v.as_str()) {
            if let Some(n) = counts.get(lv).and_then(|v| v.as_u64()) {
                counts[lv] = json!(n + 1);
            }
        }
    }
    json!({
        "generated_at": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "url": url,
        "summary": counts,
        "items": items,
        "cost_s": (t0.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    })
}
