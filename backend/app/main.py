"""FastAPI 入口（仅绑定 localhost，本地桌面工具后端）。"""
from contextlib import asynccontextmanager

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import StreamingResponse

from .config import load_config, save_config
from .notify import send_feedback_email
from .parsers import PARSERS
from .parsers.x import start_x_login, x_login_status
from .schemas import FeedbackRequest, ParseRequest, TaskCreate
from .tasks.manager import _referer_for, detect_platform, manager

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
)


@asynccontextmanager
async def lifespan(_: FastAPI):
    manager.start()
    yield
    manager.stop()
    # 关闭抖音常驻解析浏览器，避免残留 Chromium 进程
    try:
        from .parsers.douyin_browser import _teardown

        _teardown()
    except Exception:
        pass


app = FastAPI(title="Watermark Tool Backend", version="0.2.1", lifespan=lifespan)

# Tauri 生产模式 WebView2 来源为 http://tauri.localhost，需跨域访问本地 API
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://tauri.localhost", "tauri://localhost"],
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.get("/api/v1/health")
def health():
    return {"status": "ok", "version": "0.2.1", "platforms": list(PARSERS.keys())}


@app.post("/api/v1/parse")
def parse(req: ParseRequest):
    platform = detect_platform(req.url)
    if not platform:
        raise HTTPException(status_code=400, detail="无法识别的平台链接")
    parser = PARSERS.get(platform)
    if parser is None:
        raise HTTPException(
            status_code=400,
            detail=f"「{platform}」平台解析尚未支持（X 平台计划在 M4 里程碑接入）",
        )
    try:
        result = parser.parse(req.url, req.remove_platform_wm)
    except Exception as exc:
        raise HTTPException(status_code=422, detail=str(exc))
    return {
        "platform": result.platform,
        "title": result.title,
        "media_type": result.media_type,
        "files": [f.__dict__ for f in result.files],
    }


@app.post("/api/v1/tasks", response_model=None)
def create_task(req: TaskCreate):
    if req.type not in ("link", "direct"):
        raise HTTPException(status_code=400, detail=f"任务类型 {req.type} 尚未支持")
    if not req.url:
        raise HTTPException(status_code=400, detail="缺少 url")
    return manager.create(req.type, req.url, req.options)


@app.get("/api/v1/tasks")
def list_tasks():
    return manager.list()


@app.get("/api/v1/tasks/{tid}")
def get_task(tid: str):
    task = manager.get(tid)
    if not task:
        raise HTTPException(status_code=404, detail="任务不存在")
    return task


@app.post("/api/v1/tasks/{tid}/cancel")
def cancel_task(tid: str):
    if not manager.cancel(tid):
        raise HTTPException(status_code=400, detail="任务不存在或不可取消")
    return {"ok": True}


@app.get("/api/v1/media")
def media_proxy(url: str = "", request: Request = None):
    """媒体代理：绕过 CDN 防盗链，带 UA/Referer 流式转发，供前端直接播放/显示。

    透传浏览器 Range 头并转发上游 206 分段响应，保证视频可拖拽、时长正常显示。
    """
    if not url.startswith(("http://", "https://")):
        raise HTTPException(status_code=400, detail="非法媒体地址")
    upstream_headers = {"User-Agent": UA, "Referer": _referer_for(url)}
    rng = request.headers.get("range")
    if rng:
        upstream_headers["Range"] = rng
    client = httpx.Client(headers=upstream_headers, follow_redirects=True, timeout=120)
    try:
        resp = client.send(client.build_request("GET", url), stream=True)
        resp.raise_for_status()
    except Exception as exc:
        client.close()
        raise HTTPException(status_code=502, detail=f"媒体拉取失败：{exc}")

    headers = {"Cache-Control": "public, max-age=86400"}
    # 透传关键响应头（content-type / length / range / accept-ranges），保证播放与拖拽
    for h in ("content-type", "content-length", "content-range", "accept-ranges"):
        v = resp.headers.get(h)
        if v:
            headers[h] = v

    def gen():
        try:
            for chunk in resp.iter_bytes():
                yield chunk
        finally:
            client.close()

    return StreamingResponse(gen(), status_code=resp.status_code, headers=headers)


@app.get("/api/v1/config")
def get_config():
    cfg = load_config()
    # SMTP 授权码、X 登录 Cookie 等敏感字段不回传前端
    for k in ("smtp_auth_code", "smtp_user", "smtp_host", "smtp_port", "feedback_to", "x_cookie"):
        cfg.pop(k, None)
    return cfg


@app.post("/api/v1/feedback")
def feedback(req: FeedbackRequest):
    """Bug 反馈：SMTP 发送到开发者邮箱。"""
    if not req.content.strip():
        raise HTTPException(status_code=400, detail="反馈内容不能为空")
    err = send_feedback_email(load_config(), req.content.strip(), req.contact)
    if err:
        raise HTTPException(status_code=400, detail=err)
    return {"ok": True}


@app.get("/api/v1/diagnose")
def diagnose(url: str = ""):
    """风控诊断：检测解析/下载链路各环节是否被风控（D1-D8）。"""
    from .anticrawler.diagnose import run_diagnose
    return run_diagnose(url or None)


@app.post("/api/v1/x/login/start")
def x_login_start():
    """X 登录引导：启动 Edge 打开登录页，后台自动抓取并保存 cookie。"""
    return start_x_login()


@app.get("/api/v1/x/login/status")
def x_login_status_api():
    """查询 X 登录流程状态：idle / running / done / error。"""
    return x_login_status()


@app.post("/api/v1/shutdown")
def shutdown():
    """优雅关闭后端：先关闭常驻浏览器，再退出进程。

    供桌面应用退出时调用，确保不残留 Chromium / 后端进程。
    """
    import os
    import threading
    import time

    try:
        from .parsers.douyin_browser import _teardown

        _teardown()
    except Exception:
        pass
    # 稍等片刻让 HTTP 响应发完，再退出进程
    threading.Thread(
        target=lambda: (time.sleep(0.5), os._exit(0)), daemon=True
    ).start()
    return {"ok": True}


@app.put("/api/v1/config")
def put_config(cfg: dict):
    return save_config(load_config() | cfg)
