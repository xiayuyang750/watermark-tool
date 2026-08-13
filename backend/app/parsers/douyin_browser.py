"""抖音浏览器解析器（Playwright 方案）。

背景：抖音 Web 接口反爬升级后，纯 HTTP + a_bogus 签名方案常被静默拒绝（返回空）。
真实浏览器可绕过：页面安全 SDK 自动生成有效签名 + 真实指纹 + Cookie。

性能：浏览器「常驻复用」——首次解析启动并打开抖音首页，后续解析直接复用页面内
XHR 调接口（<1 秒），不再每次重新启动浏览器。页面失效由程序自动探测、刷新重试，
无需用户介入。

链路：分享链接 → 作品 ID → 浏览器内 XHR 调 detail 接口 → 分类返回原生素材。
"""
import json
import threading
from urllib.parse import urlencode

from .base import ParseResult
from .douyin import API_DETAIL, BASE_PARAMS, DouyinParser, UA, parse_aweme

try:
    from playwright.sync_api import sync_playwright
except ImportError:  # playwright 未安装时给出明确提示
    sync_playwright = None

# 常驻浏览器单例（线程安全）：所有解析串行复用同一页面
# 超过 IDLE_TTL 秒未使用自动关闭（释放内存），下次解析自动重启；
# 页面失效也自动重建，无需用户介入
_lock = threading.Lock()
_state = {"p": None, "browser": None, "context": None, "page": None, "last_used": 0.0}
IDLE_TTL = 600  # 10 分钟空闲自动关闭


def _teardown() -> None:
    """关闭常驻浏览器（应用退出或空闲超时调用）。"""
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
        if st.get("p") is not None:
            st["p"].stop()
    except Exception:
        pass
    st["p"] = None


def _ensure_page():
    """确保常驻浏览器与抖音首页就绪；失效或空闲超时自动重建。返回 page。"""
    import time

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
        _teardown()  # 页面失效或空闲超时：关闭旧的，下面重建
    p = sync_playwright().start()
    browser = p.chromium.launch(headless=True)
    context = browser.new_context(user_agent=UA)
    page = context.new_page()
    page.goto("https://www.douyin.com/", wait_until="domcontentloaded", timeout=30000)
    page.wait_for_timeout(2500)  # 首次创建：等安全 SDK 注入完成，避免首个请求落空重载
    st.update(p=p, browser=browser, context=context, page=page, last_used=time.time())
    return page


class DouyinBrowserParser:
    platform = "douyin"

    def parse(self, url: str, remove_platform_wm: bool = True) -> ParseResult:
        if sync_playwright is None:
            raise RuntimeError("缺少 playwright 依赖，请执行: pip install playwright && playwright install chromium")
        # 复用纯 HTTP 方案的短链解析（重定向不受反爬影响）
        aweme_id, is_note = DouyinParser()._resolve_aweme_id(url)
        detail = self._fetch_detail(aweme_id)
        aweme = (detail or {}).get("aweme_detail") or {}
        if not aweme:
            raise RuntimeError("抖音接口未返回作品信息：可能未登录或作品已删除")
        return parse_aweme(aweme, remove_platform_wm)

    def _fetch_detail(self, aweme_id: str) -> dict:
        params = {**BASE_PARAMS, "aweme_id": aweme_id}
        api_url = f"{API_DETAIL}?{urlencode(params)}"
        script = (
            "(function(){var xhr = new XMLHttpRequest();"
            "xhr.open('GET', %s, false);xhr.send();return xhr.responseText;})()"
            % json.dumps(api_url)
        )
        with _lock:
            page = _ensure_page()
            # 偶发空响应（风控抖动/SDK 未就绪）：重试，空则刷新页面再试
            # 首次创建页面后 SDK 可能未注入完成，会走一次刷新等待；常驻页面则直接命中
            text = ""
            for attempt in range(3):
                if attempt > 0:
                    page.reload(wait_until="domcontentloaded")
                    page.wait_for_timeout(2500)  # 刷新后等待安全 SDK 注入
                text = (page.evaluate(script) or "").strip()
                if text:
                    break
        text = (text or "").strip()
        if not text:
            raise RuntimeError("抖音接口返回空数据（可能需要登录抖音）")
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            raise RuntimeError("抖音接口返回非 JSON 数据（可能触发风控，请稍后重试）")
