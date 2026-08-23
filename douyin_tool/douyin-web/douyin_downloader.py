"""抖音视频下载器（纯协议 + Playwright 辅助签名）。

链路：
1. 短链接重定向 → 提取 aweme_id（纯 HTTP）
2. Playwright 打开 douyin.com 首页 → 安全 SDK 注入完成
3. 页面内 XHR 调用 detail 接口（SDK 自动生成有效 a_bogus 签名）
4. 提取视频 play_addr → 替换 playwm→play 去水印
5. 纯 HTTP 流式下载视频文件

用法（三种方式任选）：
    # 只粘链接
    python douyin_downloader.py "https://v.douyin.com/xxxxx/"

    # 粘抖音复制的完整分享文本（含标题、描述等文字，脚本自动提取链接）
    python douyin_downloader.py "2.84 :1pm bAg:/ j@p.Qk 01/30 暑假作息混乱？... https://v.douyin.com/xxxxx/"

    # 指定输出目录 + 音视频分离
    python douyin_downloader.py "https://v.douyin.com/xxxxx/" -o "D:\我的视频" --split

依赖：
    pip install httpx playwright playwright-stealth gmssl
    playwright install chromium
    # 音视频分离需要 ffmpeg（已安装则自动使用）
"""
import argparse
import json
import re
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlencode

import httpx

# ---- 常量 ----

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
)

API_DETAIL = "https://www.douyin.com/aweme/v1/web/aweme/detail/"

BASE_PARAMS = {
    "device_platform": "webapp",
    "aid": "6383",
    "channel": "channel_pc_web",
    "pc_client_type": "1",
    "version_code": "190500",
    "version_name": "19.5.0",
    "cookie_enabled": "true",
    "browser_language": "zh-CN",
    "browser_platform": "Win32",
    "browser_name": "Edge",
    "browser_online": "true",
    "engine_name": "Blink",
    "os_name": "Windows",
    "os_version": "10",
    "platform": "PC",
    "screen_width": "1920",
    "screen_height": "1080",
}

# ---- Playwright 常驻浏览器（线程安全单例） ----

_lock = threading.Lock()
_state = {"pw": None, "browser": None, "context": None, "page": None, "last_used": 0.0}
IDLE_TTL = 600  # 10 分钟空闲自动关闭


def _teardown():
    """关闭常驻浏览器。"""
    st = _state
    for key in ("page", "context", "browser"):
        try:
            obj = st.get(key)
            if obj is not None:
                obj.close()
        except Exception:
            pass
        st[key] = None
    try:
        if st.get("pw") is not None:
            st["pw"].stop()
    except Exception:
        pass
    st["pw"] = None


def _find_system_browser() -> str | None:
    """查找系统已安装的 Edge/Chrome 浏览器路径。"""
    import os
    candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        # macOS
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        # Linux
        "/usr/bin/google-chrome",
        "/usr/bin/microsoft-edge",
    ]
    for path in candidates:
        if os.path.isfile(path):
            return path
    return None


def _ensure_page():
    """确保 Playwright 浏览器与抖音首页就绪。返回 page。"""
    from playwright.sync_api import sync_playwright

    st = _state
    if st["page"] is not None:
        alive = False
        try:
            st["page"].evaluate("document.readyState")
            alive = True
        except Exception:
            pass
        if alive and time.time() - st.get("last_used", 0) < IDLE_TTL:
            st["last_used"] = time.time()
            return st["page"]
        _teardown()

    pw = sync_playwright().start()

    # 优先使用系统真实浏览器（更难被反爬检测）
    system_browser = _find_system_browser()
    launch_opts = {
        "headless": True,
        "args": [
            "--disable-blink-features=AutomationControlled",
            "--disable-features=AutomationControlled",
            "--no-sandbox",
            "--disable-dev-shm-usage",
        ],
    }
    if system_browser:
        launch_opts["executable_path"] = system_browser

    browser = pw.chromium.launch(**launch_opts)
    context = browser.new_context(
        user_agent=UA,
        viewport={"width": 1920, "height": 1080},
        locale="zh-CN",
    )
    # playwright-stealth 反检测注入
    try:
        from playwright_stealth import stealth_sync
        page = context.new_page()
        stealth_sync(page)
    except ImportError:
        page = context.new_page()

    # 首页预热：等安全 SDK 注入完成
    page.goto("https://www.douyin.com/", wait_until="domcontentloaded", timeout=30000)
    page.wait_for_timeout(5000)
    st.update(pw=pw, browser=browser, context=context, page=page, last_used=time.time())
    return page


# ---- 短链接解析 ----

def resolve_aweme_id(share_text: str) -> tuple[str, bool]:
    """从分享文本提取链接 → 重定向 → 得到 aweme_id 和是否图文笔记。"""
    url = _extract_url(share_text)
    with httpx.Client(
        headers={"User-Agent": UA, "Referer": "https://www.douyin.com/"},
        follow_redirects=True,
        timeout=20,
    ) as client:
        resp = client.get(url)
    final_url = str(resp.url)

    m = re.search(r"/(video|note)/(\d+)", final_url)
    if m:
        return m.group(2), m.group(1) == "note"
    m = re.search(r"/aweme/detail/(\d+)", final_url)
    if m:
        return m.group(1), False
    m = re.search(r"\b(\d{15,21})\b", final_url)
    if m:
        return m.group(1), False
    raise RuntimeError(f"无法从链接中提取作品 ID: {final_url}")


def _extract_url(text: str) -> str:
    m = re.search(r"https?://\S+", text)
    if m:
        return m.group(0).rstrip("，。,。；;")
    raise RuntimeError("未在输入中找到可用的链接")


# ---- Detail API 调用（通过浏览器页面内 XHR） ----

def fetch_aweme_detail(aweme_id: str) -> dict:
    """通过 Playwright 拦截页面 detail API 响应，获取作品详情。

    策略：导航到作品页 → 页面 JS VM 自动生成 a_bogus 并发起 detail 请求
    → 拦截该请求的响应体 → 纯 HTTP 重放获取数据。
    """
    with _lock:
        page = _ensure_page()
        captured = []

        def on_response(response):
            if "aweme/v1/web/aweme/detail" in response.url:
                try:
                    body = response.text()
                    if body and "aweme_detail" in body:
                        captured.append(body)
                except Exception:
                    pass

        page.on("response", on_response)

        for attempt in range(3):
            captured.clear()
            page.goto(
                f"https://www.douyin.com/video/{aweme_id}",
                wait_until="domcontentloaded",
                timeout=20000,
            )
            # 等待页面 JS VM 执行并发起 detail API 请求
            page.wait_for_timeout(8000)
            if captured:
                break
            # 重试前刷新
            if attempt < 2:
                page.goto("https://www.douyin.com/", wait_until="domcontentloaded", timeout=15000)
                page.wait_for_timeout(3000)

        page.remove_listener("response", on_response)

    if not captured:
        # 回退：尝试手动 XHR
        return _fetch_detail_xhr(aweme_id)

    try:
        return json.loads(captured[0])
    except json.JSONDecodeError:
        raise RuntimeError("拦截到的 detail 数据解析失败")


def _fetch_detail_xhr(aweme_id: str) -> dict:
    """回退方案：页面内手动 XHR。"""
    params = {**BASE_PARAMS, "aweme_id": aweme_id}
    api_url = f"{API_DETAIL}?{urlencode(params)}"
    script = (
        "(function(){var xhr = new XMLHttpRequest();"
        "xhr.open('GET', %s, false);xhr.send();return xhr.responseText;})()"
        % json.dumps(api_url)
    )
    with _lock:
        page = _ensure_page()
        text = ""
        for attempt in range(3):
            if attempt > 0:
                page.reload(wait_until="domcontentloaded")
                page.wait_for_timeout(4000)
            text = (page.evaluate(script) or "").strip()
            if text and len(text) > 50:
                break
    if not text:
        raise RuntimeError("抖音接口返回空数据（可能需要登录抖音）")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        raise RuntimeError("抖音接口返回非 JSON 数据（可能触发风控）")


# ---- 解析作品信息 ----

def parse_aweme(aweme: dict, remove_platform_wm: bool = True) -> dict:
    """从 aweme_detail 提取视频下载信息。"""
    title = (aweme.get("desc") or "").strip() or "未命名作品"

    video = aweme.get("video") or {}
    play_addr = video.get("play_addr") or {}
    url_list = play_addr.get("url_list") or []
    if not url_list:
        raise RuntimeError("视频作品未包含播放地址")

    url = url_list[0].replace("http://", "https://")
    if remove_platform_wm:
        url = url.replace("playwm", "play")

    cover = ((video.get("cover") or {}).get("url_list") or [""])[0].replace(
        "http://", "https://"
    )
    return {
        "title": title,
        "video_url": url,
        "cover": cover or None,
        "aweme_id": aweme.get("aweme_id", ""),
    }


# ---- 下载 ----

def download_video(url: str, output_path: Path, title: str = "video") -> Path:
    """流式下载视频文件。"""
    output_path.parent.mkdir(parents=True, exist_ok=True)
    total = 0
    with httpx.Client(
        headers={"User-Agent": UA, "Referer": "https://www.douyin.com/"},
        follow_redirects=True,
        timeout=120,
    ) as client:
        with client.stream("GET", url) as resp:
            resp.raise_for_status()
            content_length = int(resp.headers.get("content-length") or 0)
            with open(output_path, "wb") as f:
                for chunk in resp.iter_bytes():
                    f.write(chunk)
                    total += len(chunk)
                    if content_length:
                        pct = total / content_length * 100
                        print(f"\r  下载进度: {pct:.1f}% ({total}/{content_length})", end="", flush=True)
    print()
    return output_path


def split_audio_video(video_path: Path) -> tuple[Path, Path]:
    """用 ffmpeg 将视频拆分为无声视频 + 纯音频。

    返回 (video_only_path, audio_only_path)。
    参考文件命名风格：{原名}-视频.mp4, {原名}-音频.mp4
    """
    import shutil
    import subprocess

    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise RuntimeError(
            "未找到 ffmpeg，无法分离音视频。"
            "请安装 ffmpeg 并确保在 PATH 中：https://ffmpeg.org/download.html"
        )

    stem = video_path.stem
    parent = video_path.parent
    video_only = parent / f"{stem}-视频.mp4"
    audio_only = parent / f"{stem}-音频.mp4"

    # 无声视频
    print("  生成无声视频...")
    subprocess.run(
        [ffmpeg, "-y", "-i", str(video_path), "-an", "-c:v", "copy", str(video_only)],
        capture_output=True, check=True,
    )
    # 纯音频
    print("  生成纯音频...")
    subprocess.run(
        [ffmpeg, "-y", "-i", str(video_path), "-vn", "-c:a", "copy", str(audio_only)],
        capture_output=True, check=True,
    )
    return video_only, audio_only


# ---- 文件名清理 ----

def sanitize_filename(name: str) -> str:
    name = re.sub(r'[\\/:*?"<>|\r\n]', "_", name).strip() or "untitled"
    return name[:60]


# ---- 主流程 ----

def main():
    parser = argparse.ArgumentParser(
        description="抖音视频下载器",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "使用示例:\n"
            '  python douyin_downloader.py "https://v.douyin.com/xxxxx/"\n'
            '  python douyin_downloader.py "复制的完整分享文本..." -o "D:\\下载"\n'
            '  python douyin_downloader.py "https://v.douyin.com/xxxxx/" --split\n'
        ),
    )
    parser.add_argument("url", help="抖音分享链接或复制的完整分享文本")
    parser.add_argument("--output", "-o", default="downloads", help="输出目录 (默认: downloads)")
    parser.add_argument("--keep-watermark", action="store_true", help="保留平台水印")
    parser.add_argument("--split", action="store_true", help="音视频分离（需要 ffmpeg）")
    args = parser.parse_args()

    output_dir = Path(args.output)
    output_dir.mkdir(parents=True, exist_ok=True)

    try:
        # Step 1: 解析短链接获取 aweme_id
        print("[1/4] 解析分享链接...")
        aweme_id, is_note = resolve_aweme_id(args.url)
        print(f"  作品 ID: {aweme_id} ({'图文笔记' if is_note else '视频'})")

        # Step 2: 通过浏览器获取作品详情
        print("[2/4] 获取作品详情（通过浏览器签名）...")
        detail = fetch_aweme_detail(aweme_id)
        aweme = detail.get("aweme_detail") or {}
        if not aweme:
            print("  接口未返回作品信息，可能已删除或需要登录")
            sys.exit(1)

        # Step 3: 提取视频信息
        print("[3/4] 提取视频信息...")
        info = parse_aweme(aweme, remove_platform_wm=not args.keep_watermark)
        print(f"  标题: {info['title']}")
        print(f"  视频 URL: {info['video_url'][:80]}...")

        # Step 4: 下载视频
        filename = sanitize_filename(info["title"]) + ".mp4"
        output_path = output_dir / filename
        print(f"[4/4] 下载视频: {filename}")
        download_video(info["video_url"], output_path, info["title"])
        print(f"  文件大小: {output_path.stat().st_size / 1024 / 1024:.1f} MB")

        # Step 5 (可选): 音视频分离
        if args.split:
            print("[5/5] 音视频分离...")
            try:
                video_only, audio_only = split_audio_video(output_path)
                print(f"  无声视频: {video_only.name} ({video_only.stat().st_size / 1024 / 1024:.1f} MB)")
                print(f"  纯音频:   {audio_only.name} ({audio_only.stat().st_size / 1024 / 1024:.1f} MB)")
            except RuntimeError as e:
                print(f"  跳过音视频分离: {e}")

        print(f"\n下载完成!")

    except Exception as e:
        print(f"\n错误: {e}", file=sys.stderr)
        sys.exit(1)
    finally:
        _teardown()


if __name__ == "__main__":
    main()
