"""抖音视频下载 Web 服务。

启动方式：
    python douyin_web.py

启动后浏览器打开：
    http://localhost:17892

用户只需在页面输入框粘贴抖音分享文本，点击解析 → 下载即可。
"""
import json
import re
import threading
import time
from pathlib import Path
from urllib.parse import quote, urlencode

import httpx
from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse, HTMLResponse
import uvicorn

# ---- 复用 douyin_downloader 的核心逻辑 ----
from douyin_downloader import (
    _ensure_page,
    _teardown,
    _extract_url,
    _fetch_detail_xhr,
    _find_system_browser,
    _lock,
    _state,
    API_DETAIL,
    BASE_PARAMS,
    UA,
    download_video,
    parse_aweme,
    resolve_aweme_id,
    sanitize_filename,
    fetch_aweme_detail,
)

# ---- 配置 ----

PORT = 17892
OUTPUT_DIR = Path(__file__).parent / "downloads"

app = FastAPI(title="抖音视频下载器")


# ---- 静态页面 ----

@app.get("/", response_class=HTMLResponse)
def index():
    html_path = Path(__file__).parent / "douyin_web.html"
    return HTMLResponse(html_path.read_text(encoding="utf-8"))


# ---- API ----

@app.get("/api/parse")
def api_parse(url: str):
    """解析抖音分享链接，返回视频信息（标题、封面、下载 URL）。"""
    if not url.strip():
        raise HTTPException(400, "请输入链接")

    try:
        aweme_id, is_note = resolve_aweme_id(url)
    except RuntimeError as e:
        raise HTTPException(400, f"链接解析失败: {e}")

    try:
        detail = fetch_aweme_detail(aweme_id)
    except RuntimeError as e:
        raise HTTPException(422, str(e))

    aweme = detail.get("aweme_detail") or {}
    if not aweme:
        raise HTTPException(422, "接口未返回作品信息，可能已删除或需要登录")

    info = parse_aweme(aweme, remove_platform_wm=True)
    return {
        "title": info["title"],
        "cover": info.get("cover"),
        "aweme_id": info.get("aweme_id", ""),
        "download_url": f"/api/download?video_url={quote(info['video_url'], safe='')}&title={quote(info['title'], safe='')}",
    }


@app.get("/api/download")
def api_download(video_url: str, title: str = "video"):
    """下载视频并返回文件。"""
    if not video_url:
        raise HTTPException(400, "缺少 video_url")

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    filename = sanitize_filename(title) + ".mp4"
    output_path = OUTPUT_DIR / filename

    try:
        download_video(video_url, output_path, title)
    except Exception as e:
        raise HTTPException(500, f"下载失败: {e}")

    if not output_path.exists() or output_path.stat().st_size == 0:
        raise HTTPException(500, "下载的文件为空")

    return FileResponse(
        path=str(output_path),
        filename=filename,
        media_type="video/mp4",
        headers={"Content-Disposition": f'attachment; filename="{filename}"'},
    )


# ---- 启动 ----

if __name__ == "__main__":
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    print(f"启动抖音视频下载服务...")
    print(f"打开浏览器访问: http://localhost:{PORT}")
    uvicorn.run(app, host="127.0.0.1", port=PORT, log_level="info")
