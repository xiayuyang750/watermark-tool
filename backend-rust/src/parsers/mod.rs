//! 解析器注册表（对齐 Python backend/app/parsers/__init__.py）。

pub mod cdp;
pub mod douyin;
pub mod sign;
pub mod x;

use std::sync::OnceLock;

use crate::schemas::ParseResult;

pub const PARSERS: [&str; 2] = ["douyin", "x"];

/// 抖音解析器单例（浏览器常驻复用，全局唯一）。
fn douyin() -> &'static douyin::DouyinParser {
    static P: OnceLock<douyin::DouyinParser> = OnceLock::new();
    P.get_or_init(douyin::DouyinParser::new)
}

/// 解析入口。
pub async fn parse(platform: &str, url: &str, remove_platform_wm: bool) -> Result<ParseResult, String> {
    match platform {
        "x" => x::parse(url, remove_platform_wm).await,
        "douyin" => douyin().parse(url, remove_platform_wm).await,
        _ => Err(format!("「{platform}」平台解析尚未支持")),
    }
}

/// 应用退出时关闭常驻解析浏览器（避免残留 Chromium 进程）。
pub async fn teardown() {
    douyin().teardown().await;
}
