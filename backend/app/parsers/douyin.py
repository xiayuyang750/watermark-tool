"""抖音解析器。

链路：分享链接（含复制文本）→ 提取作品 ID → a_bogus 签名调用
`aweme/v1/web/aweme/detail` 接口 → 按内容类型返回原生素材：
- 视频（aweme_type 0/4/99 等，video 字段）
- 图集（images 字段，多张静态图）
- 动图（image 含 animated/gif 字段）
- Live 图（image 含 video 字段）

平台水印去除：视频播放地址 playwm→play；Live 图剥离 watermark/logo_name 参数。
"""
import re
from urllib.parse import quote, urlencode

import httpx

from .base import MediaFile, ParseResult, PlatformParser
from .douyin_sig import ABogus

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
)

# 与 douyin_parse 保持一致的 Web 接口基础参数
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

API_DETAIL = "https://www.douyin.com/aweme/v1/web/aweme/detail/"


class DouyinParser(PlatformParser):
    platform = "douyin"

    def __init__(self):
        self.abogus = ABogus()

    def parse(self, url: str, remove_platform_wm: bool = True) -> ParseResult:
        aweme_id, is_note = self._resolve_aweme_id(url)
        data = self._fetch_detail(aweme_id, is_note)
        aweme = (data or {}).get("aweme_detail") or {}
        if not aweme:
            raise RuntimeError("接口未返回作品信息（可能已删除或需要登录）")
        return self._to_result(aweme, remove_platform_wm)

    # ---- 内部实现 ----

    def _fetch_detail(self, aweme_id: str, is_note: bool) -> dict:
        params = {**BASE_PARAMS, "aweme_id": aweme_id}
        referer = f"https://www.douyin.com/{'note' if is_note else 'video'}/{aweme_id}"
        headers = {
            "User-Agent": UA,
            "Accept": "application/json, text/plain, */*",
            "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
            "Referer": referer,
            "Origin": "https://www.douyin.com",
            "Sec-Fetch-Site": "same-site",
            "Sec-Fetch-Mode": "cors",
            "Sec-Fetch-Dest": "empty",
        }
        try:
            sig = self.abogus.get_value(params)
        except Exception:
            raise RuntimeError("签名生成失败（a_bogus 算法异常）")
        signed_url = f"{API_DETAIL}?{urlencode(params)}&a_bogus={quote(sig, safe='')}"
        with httpx.Client(headers=headers, timeout=20) as client:
            resp = client.get(signed_url)
        try:
            data = resp.json()
        except Exception:
            raise RuntimeError("接口返回非 JSON 数据（可能触发风控）")
        if data.get("status_code") != 0:
            raise RuntimeError(f"接口返回异常: {data.get('status_msg', '未知错误')}")
        return data

    def _resolve_aweme_id(self, share_text: str) -> tuple[str, bool]:
        """从分享文本中提取链接 → 重定向 → 得到作品 ID 与类型（video/note）。"""
        url = self._extract_url(share_text)
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
        raise RuntimeError("无法从链接中提取作品 ID，请确认是抖音分享链接")

    @staticmethod
    def _extract_url(text: str) -> str:
        """从复制的分享文本中提取 URL（兼容文本+链接混合格式）。"""
        m = re.search(r"https?://\S+", text)
        if m:
            return m.group(0).rstrip("，。,。；;")
        raise RuntimeError("未在输入中找到可用的链接")

    def _to_result(self, aweme: dict, remove_platform_wm: bool) -> ParseResult:
        return parse_aweme(aweme, remove_platform_wm)

    def _image_file(self, img: dict, remove_platform_wm: bool) -> MediaFile | None:
        return image_file(img, remove_platform_wm)


def parse_aweme(aweme: dict, remove_platform_wm: bool) -> ParseResult:
    """按内容类型返回原生素材（视频 / 图集 / 动图 / Live 图）。"""
    title = (aweme.get("desc") or "").strip() or "未命名作品"

    images = aweme.get("images")
    if images:
        files = [f for f in (image_file(img, remove_platform_wm) for img in images) if f]
        if not files:
            raise RuntimeError("图集作品未包含图片地址")
        return ParseResult("douyin", title, "image", files)

    video = aweme.get("video") or {}
    play_addr = video.get("play_addr") or {}
    url_list = play_addr.get("url_list") or []
    if not url_list:
        raise RuntimeError("视频作品未包含播放地址")
    url = url_list[0].replace("http://", "https://")
    if remove_platform_wm:
        url = url.replace("playwm", "play")
    cover = ((video.get("cover") or {}).get("url_list") or [""])[0].replace("http://", "https://")
    return ParseResult("douyin", title, "video", [MediaFile("video", url, cover=cover or None)])


def image_file(img: dict, remove_platform_wm: bool) -> MediaFile | None:
    """单张图片：Live 图 > 动图 > 静态图。"""
    # Live 图：带 video 字段（图片会动）→ 静态照片 + 3 秒视频双文件
    video = img.get("video") or {}
    if video:
        play_addr = video.get("play_addr") or {}
        urls = play_addr.get("url_list") or []
        if urls:
            url = urls[0].replace("http://", "https://")
            cover = ((video.get("cover") or {}).get("url_list") or [""])[0].replace("http://", "https://")
            # 静态照片直链：Live 图由「照片 + 视频」组成，缺一不可
            photo = ((img.get("url_list") or [""])[0] or "").replace("http://", "https://")
            return MediaFile("livephoto", url, cover=cover or None, image_url=photo or None)
    # 动图：animated/gif 字段
    for field in ("animated_url_list", "gif_url_list", "animated_url", "gif_url"):
        v = img.get(field)
        if isinstance(v, str) and v:
            return MediaFile("gif", v)
        if isinstance(v, list) and v:
            return MediaFile("gif", v[0])
    # 静态图
    urls = img.get("url_list") or []
    if urls:
        return MediaFile("image", urls[0].replace("http://", "https://"))
    return None
