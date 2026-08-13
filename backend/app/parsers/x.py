"""X（Twitter）解析器。

链路：推文链接 → 提取推文 ID → vxtwitter 优先 / fxtwitter 兜底（公共 API，无需登录）
→ 仍失败且配置了 x_cookie 时用 Playwright 带 cookie 打开推文页抓取（登录墙/私密推文）。

媒体：X 的视频 / 动图（GIF）都是 mp4（无平台水印），图片在 pbs.twimg.com；
直链无防盗链、支持 Range，可直接播放与下载，无需本地代理。
"""
import json
import re

import httpx

from .base import MediaFile, ParseResult, PlatformParser

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
)
H = {"User-Agent": UA, "Accept": "application/json"}

VX_API = "https://api.vxtwitter.com/i/status/{tid}"
FX_API = "https://api.fxtwitter.com/status/{tid}"

_TWEET_ID_RE = re.compile(
    r"(?:x|twitter)\.com/(?:[^/?#]+/status/|i/status/)(\d{15,20})"
)
_RAW_ID_RE = re.compile(r"\b(\d{15,20})\b")


def _ensure_video_tag(url: str) -> str:
    """video.twimg.com 视频直链需带 tag 签名参数（如 tag=29），否则返回 403。"""
    if "tag=" in url:
        return url
    sep = "&" if "?" in url else "?"
    return f"{url}{sep}tag=29"


class XParser(PlatformParser):
    platform = "x"

    def parse(self, url: str, remove_platform_wm: bool = True) -> ParseResult:
        tid = self._extract_tweet_id(url)
        data = self._fetch_public(tid)
        if data is None:
            data = self._fetch_with_cookie(tid)
        if data is None:
            from ..config import load_config

            hint = ""
            if load_config().get("x_cookie"):
                hint = "（登录态可能已过期，请在设置中重新打开 X 登录）"
            raise RuntimeError(
                f"X 解析失败：该推文可能已删除、仅登录可见，或第三方解析服务暂时不可用{hint}"
            )
        return self._to_result(data, tid)

    # ---- ID 提取 ----

    @staticmethod
    def _extract_tweet_id(text: str) -> str:
        m = _TWEET_ID_RE.search(text)
        if m:
            return m.group(1)
        m = _RAW_ID_RE.search(text)
        if m:
            return m.group(1)
        raise RuntimeError("无法从链接中提取推文 ID，请确认是 X/Twitter 的推文链接")

    # ---- 公共解析（vxtwitter 优先，fxtwitter 兜底） ----

    @staticmethod
    def _fetch_public(tid: str) -> dict | None:
        try:
            r = httpx.get(VX_API.format(tid=tid), headers=H, follow_redirects=True, timeout=20)
            if r.status_code == 200:
                d = r.json()
                if d.get("media_extended") or d.get("mediaURLs"):
                    return {"source": "vxtwitter", "data": d}
        except Exception:
            pass
        try:
            r = httpx.get(FX_API.format(tid=tid), headers=H, follow_redirects=True, timeout=20)
            if r.status_code == 200:
                d = r.json()
                t = d.get("tweet") or {}
                if t.get("media") and t["media"].get("all"):
                    return {"source": "fxtwitter", "data": t}
        except Exception:
            pass
        return None

    # ---- 登录墙兜底（Playwright + x_cookie） ----

    def _fetch_with_cookie(self, tid: str) -> dict | None:
        from ..config import load_config

        cookie_str = (load_config().get("x_cookie") or "").strip()
        if not cookie_str:
            return None
        try:
            from playwright.sync_api import sync_playwright
        except ImportError:
            return None

        captured: dict = {}
        with sync_playwright() as p:
            browser = p.chromium.launch(headless=True)
            ctx = browser.new_context(user_agent=UA)
            try:
                cookies = json.loads(cookie_str)
                if isinstance(cookies, list):
                    ctx.add_cookies(cookies)
            except Exception:
                pass  # 非法 cookie 数据则直接走无 cookie 流程

            def on_response(resp):
                if "TweetResultByRestId" in resp.url:
                    try:
                        captured["graphql"] = resp.json()
                    except Exception:
                        pass

            page = ctx.new_page()
            page.on("response", on_response)
            try:
                page.goto(f"https://x.com/i/status/{tid}", wait_until="domcontentloaded", timeout=45000)
                for _ in range(20):
                    if captured.get("graphql"):
                        break
                    page.wait_for_timeout(2000)
            except Exception:
                pass
            ctx.close()
            browser.close()

        gql = captured.get("graphql")
        if not gql:
            return None
        return {"source": "graphql", "data": self._extract_from_graphql(gql)}

    @staticmethod
    def _extract_from_graphql(gql: dict) -> dict | None:
        """从 GraphQL TweetResultByRestId 响应中提取媒体列表（vxtwitter 兼容结构）。"""
        try:
            res = gql["data"]["tweetResult"]["result"]
        except Exception:
            return None
        media = []
        details = res.get("media_details") or []
        if not details:
            legacy = (res.get("legacy") or {}).get("extended_entities") or {}
            for m in legacy.get("media") or []:
                item = {"id_str": m.get("id_str", ""), "type": m.get("type", "photo")}
                if m.get("type") == "photo":
                    item["url"] = (m.get("media_url_https") or "").replace("http://", "https://")
                else:
                    item["url"] = m.get("video_info", {}).get("variants", [{}])[0].get("url", "")
                item["thumbnail_url"] = (m.get("media_url_https") or "").replace("http://", "https://")
                item["duration_millis"] = m.get("video_info", {}).get("duration_millis", 0)
                media.append(item)
        else:
            for m in details:
                item = {
                    "id_str": m.get("id", ""),
                    "type": m.get("type", "photo"),
                    "thumbnail_url": m.get("media_url_https") or m.get("thumbnail_url") or "",
                }
                if m.get("type") == "photo":
                    item["url"] = m.get("media_url_https", "")
                else:
                    variants = m.get("video", {}).get("variants") or []
                    url = ""
                    for v in sorted(variants, key=lambda v: v.get("bitrate") or 0, reverse=True):
                        if v.get("content_type") == "video/mp4":
                            url = v["url"]
                            break
                    item["url"] = url
                    item["duration_millis"] = m.get("video", {}).get("duration_millis", 0)
                media.append(item)
        if not media:
            return None
        text = (res.get("legacy") or {}).get("full_text") or (res.get("legacy") or {}).get("text") or ""
        return {"media_extended": media, "text": text}

    # ---- 转 ParseResult ----

    @staticmethod
    def _to_result(data: dict, tid: str) -> ParseResult:
        source = data["source"]
        if source == "vxtwitter":
            d = data["data"]
            items = d.get("media_extended") or []
            title = (d.get("text") or "").strip() or f"推文 {tid}"
        elif source == "fxtwitter":
            d = data["data"]
            items = (d.get("media") or {}).get("all") or []
            title = (d.get("text") or "").strip() or f"推文 {tid}"
        else:  # graphql（已转成 vxtwitter 兼容结构）
            d = data["data"]
            items = d.get("media_extended") or []
            title = (d.get("text") or "").strip() or f"推文 {tid}"

        files = []
        for it in items:
            mtype = (it.get("type") or "photo").lower()
            url = (it.get("url") or "").strip()
            if not url:
                continue
            if mtype in ("video", "gif"):
                # X 的动图（GIF）是无声 mp4，统一按视频处理；视频直链需带 tag 签名参数否则 403
                files.append(
                    MediaFile("video", _ensure_video_tag(url), cover=(it.get("thumbnail_url") or None))
                )
            else:  # photo / 图集
                files.append(MediaFile("image", url))
        if not files:
            raise RuntimeError("该推文没有可下载的视频/图片内容")
        return ParseResult("x", title, "video" if files[0].kind == "video" else "image", files)


# ---- 登录引导（应用内弹出浏览器登录 X，自动保存 cookie） ----

# 登录流程全局状态（单实例足够）：idle / running / done / error
_login_state: dict = {"status": "idle", "error": None}


def start_x_login() -> dict:
    """启动系统 Edge（调试端口）打开 X 登录页，后台轮询登录结果并保存 cookie。

    使用固定登录档案目录：登录态长期保留，下次打开登录窗口直接复用，
    无需反复输入账号密码。
    """
    import os
    import socket
    import subprocess
    import threading
    import time

    from ..config import APP_DIR

    if _login_state["status"] == "running":
        return {"ok": False, "error": "已有登录流程正在进行中"}
    _login_state.update(status="running", error=None)

    # 关闭上次残留的登录实例（同一固定档案），避免档案被占用导致启动失败
    subprocess.run(
        [
            "powershell", "-NoProfile", "-Command",
            "Get-CimInstance Win32_Process -Filter \"Name='msedge.exe'\" | "
            "Where-Object { $_.CommandLine -match 'x_login_profile' } | "
            "ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }",
        ],
        capture_output=True,
        timeout=20,
    )
    time.sleep(1)

    # 找可用端口
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()

    profile_dir = APP_DIR / "x_login_profile"
    profile_dir.mkdir(parents=True, exist_ok=True)
    edge = None
    for cand in (
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ):
        if os.path.exists(cand):
            edge = cand
            break
    if not edge:
        _login_state.update(status="error", error="未找到 Edge 浏览器，请改用设置中手动粘贴 Cookie")
        return {"ok": False, "error": "未找到 Edge 浏览器"}

    subprocess.Popen(
        [
            edge,
            f"--remote-debugging-port={port}",
            f"--user-data-dir={profile_dir}",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-blink-features=AutomationControlled",
            "--disable-features=AutomationControlled",
            "https://x.com/i/flow/login",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    # 等待调试端口就绪
    deadline = time.time() + 20
    while time.time() < deadline:
        try:
            import httpx as _h

            _h.get(f"http://127.0.0.1:{port}/json/version", timeout=2)
            break
        except Exception:
            time.sleep(0.5)

    def _poll():
        try:
            result = wait_x_login(port, profile_dir)
        except Exception as exc:
            result = {"ok": False, "error": f"登录流程异常：{exc}"}
        if result.get("ok"):
            _login_state.update(status="done", error=None)
        else:
            _login_state.update(status="error", error=result.get("error"))

    threading.Thread(target=_poll, daemon=True).start()
    return {"ok": True, "message": "请在 Edge 窗口中完成 X 登录（建议用手机 X App 扫码，比输入账号更稳定）"}


def x_login_status() -> dict:
    """查询登录流程状态。"""
    return {
        "ok": _login_state["status"] == "done",
        "status": _login_state["status"],
        "error": _login_state.get("error"),
    }


def wait_x_login(port: int, profile: str, timeout: int = 300) -> dict:
    """轮询登录结果：出现 auth_token 即视为登录成功，保存 cookie 到配置并关闭浏览器。"""
    import json as _json
    import time

    from ..config import load_config, save_config

    deadline = time.time() + timeout
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        return {"ok": False, "error": "缺少 playwright 依赖"}

    browser = None
    ctx = None
    try:
        with sync_playwright() as p:
            try:
                browser = p.chromium.connect_over_cdp(f"http://127.0.0.1:{port}")
            except Exception:
                return {"ok": False, "error": "无法连接登录浏览器（可能已关闭），请重试"}
            ctx = browser.contexts[0] if browser.contexts else browser.new_context()
            while time.time() < deadline:
                if not browser.is_connected():
                    return {"ok": False, "error": "登录窗口已关闭，登录未完成"}
                try:
                    cookies = ctx.cookies()
                except Exception:
                    time.sleep(2)
                    continue
                if any(c.get("name") == "auth_token" and c.get("value") for c in cookies):
                    save_config(load_config() | {"x_cookie": _json.dumps(cookies)})
                    return {"ok": True}
                time.sleep(2)
            return {"ok": False, "error": "登录超时，请重试"}
    finally:
        try:
            if ctx:
                ctx.close()
        except Exception:
            pass
        try:
            if browser:
                browser.close()
        except Exception:
            pass
