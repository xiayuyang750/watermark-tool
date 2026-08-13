"""风控诊断工具：检测解析/下载链路是否被风控、被风控在哪个环节。

对应 ANTI_RISK_PLAN.md 的 D1-D8 检测维度。
用法：
  python -m app.anticrawler.diagnose [--url https://v.douyin.com/xxx] [--json]
API：GET /api/v1/diagnose?url=xxx
"""
import json
import sys
import time

import httpx

from ..config import APP_DIR
from ..parsers.douyin import UA, DouyinParser
from ..parsers.douyin_browser import DouyinBrowserParser

DEFAULT_URL = "https://v.douyin.com/7vbhy1d3U7s/"  # 内置测试链接（可用 --url 指定自己的）


def _item(did: str, name: str, level: str, evidence: dict, suggestion: str = "") -> dict:
    return {"id": did, "name": name, "level": level, "evidence": evidence, "suggestion": suggestion}


# ---- D1 平台连通性 ----

def _d1() -> dict:
    try:
        r = httpx.get(
            "https://www.douyin.com/",
            headers={"User-Agent": UA}, follow_redirects=True, timeout=15,
        )
        head = r.text[:2000]
        evi = {"status": r.status_code, "body_len": len(r.text)}
        if "验证码" in head:
            return _item("D1", "平台连通性", "FAIL", {**evi, "note": "返回验证码页"},
                         "IP/环境被风控：更换网络、等待冷却或启用代理（L4 需合规评估）")
        if r.status_code == 200:
            return _item("D1", "平台连通性", "PASS", evi)
        return _item("D1", "平台连通性", "FAIL", evi, "非 200，平台拒绝访问")
    except Exception as exc:
        return _item("D1", "平台连通性", "FAIL", {"error": str(exc)}, "网络不可达，检查网络/代理")


# ---- D2 分享链接解析 ----

def _d2(url: str) -> dict:
    try:
        aweme_id, is_note = DouyinParser()._resolve_aweme_id(url)
        return _item("D2", "分享链接解析", "PASS", {"aweme_id": aweme_id, "is_note": is_note})
    except Exception as exc:
        msg = str(exc)
        return _item("D2", "分享链接解析", "FAIL", {"error": msg},
                     "短链重定向被拦截或链接失效；若含验证码则属 IP 风控")


# ---- D3 浏览器解析（主方案） ----

def _d3(url: str) -> tuple[dict, str | None]:
    t0 = time.time()
    try:
        result = DouyinBrowserParser().parse(url, True)
        first = result.files[0].url if result.files else None
        return (_item("D3", "浏览器解析(主)", "PASS",
                      {"media_type": result.media_type, "files": len(result.files),
                       "cost_s": round(time.time() - t0, 1)}), first)
    except Exception as exc:
        msg = str(exc)
        level = "FAIL" if ("空数据" in msg or "非 JSON" in msg or "未返回" in msg) else "WARN"
        return (_item("D3", "浏览器解析(主)", level, {"error": msg, "cost_s": round(time.time() - t0, 1)},
                      "浏览器方案被拒：检查 chromium 安装、页面 SDK 注入、升级 playwright；"
                      "策略见 ANTI_RISK_PLAN.md L2"), None)


# ---- D4 纯 HTTP 签名（备用方案） ----

def _d4(url: str) -> tuple[dict, str | None]:
    t0 = time.time()
    try:
        result = DouyinParser().parse(url, True)  # a_bogus 签名路径
        first = result.files[0].url if result.files else None
        return (_item("D4", "纯HTTP签名(备)", "PASS",
                      {"media_type": result.media_type, "files": len(result.files),
                       "cost_s": round(time.time() - t0, 1)}), first)
    except Exception as exc:
        msg = str(exc)
        level = "FAIL" if ("encrypt_data_miss" in msg or "非 JSON" in msg or "空数据" in msg) else "WARN"
        return (_item("D4", "纯HTTP签名(备)", level, {"error": msg, "cost_s": round(time.time() - t0, 1)},
                      "a_bogus 方案被拒（encrypt_data_miss/空响应）：同步社区最新签名实现（L3）"), None)


# ---- D5 CDN 下载 ----

def _d5(first_url: str | None) -> dict:
    if not first_url:
        return _item("D5", "CDN下载", "WARN", {"note": "无直链可测（解析未成功）"}, "先修复 D3/D4")
    try:
        r = httpx.get(first_url, headers={"User-Agent": UA, "Referer": "https://www.douyin.com/"},
                      follow_redirects=True, timeout=30)
        ok = r.status_code == 200 and len(r.content) > 1000
        evi = {"status": r.status_code, "content_type": r.headers.get("content-type"),
               "bytes": len(r.content)}
        return _item("D5", "CDN下载", "PASS" if ok else "FAIL", evi,
                     "" if ok else "CDN 直链被拒或过期，重新解析获取新直链")
    except Exception as exc:
        return _item("D5", "CDN下载", "FAIL", {"error": str(exc)}, "下载超时/失败")


# ---- D6 登录态 ----

def _d6() -> dict:
    profile = APP_DIR / "browser_profile"
    if profile.exists():
        return _item("D6", "登录态", "PASS", {"profile": str(profile)})
    return _item("D6", "登录态", "WARN", {"profile": "none"},
                 "无持久化登录态：公开作品可解析；需登录的私密内容待接入登录流程（L5）")


# ---- D7 IP 频率 ----

def _d7() -> dict:
    hits = []
    try:
        with httpx.Client(headers={"User-Agent": UA}, timeout=10) as c:
            for _ in range(3):
                hits.append(c.get("https://www.douyin.com/").status_code)
        ok = all(h == 200 for h in hits)
        return _item("D7", "IP频率", "PASS" if ok else "WARN", {"statuses": hits},
                     "" if ok else "出现非 200，疑似频率风控：降低请求频率、加退避（L1）")
    except Exception as exc:
        return _item("D7", "IP频率", "FAIL", {"error": str(exc), "statuses": hits}, "请求异常")


# ---- D8 本地链路 ----

def _d8() -> dict:
    result: dict = {}
    try:
        result["backend"] = httpx.get("http://127.0.0.1:17890/api/v1/health", timeout=5).status_code
    except Exception as exc:
        result["backend"] = f"ERR {exc}"
    try:
        result["frontend"] = httpx.get("http://localhost:5173/", timeout=5).status_code
    except Exception as exc:
        result["frontend"] = f"ERR {exc}"
    if result["backend"] == 200 and result["frontend"] == 200:
        return _item("D8", "本地链路", "PASS", result)
    if result["backend"] == 200:
        return _item("D8", "本地链路", "WARN", result, "前端未启动，运行 scripts/start.ps1")
    return _item("D8", "本地链路", "FAIL", result, "后端未启动，运行 scripts/start.ps1")


# ---- 汇总 ----

def run_diagnose(url: str | None = None) -> dict:
    url = (url or "").strip() or DEFAULT_URL
    t0 = time.time()
    d3, d3_url = _d3(url)
    d4, d4_url = _d4(url)
    items = [
        _d1(),
        _d2(url),
        d3,
        d4,
        _d5(d3_url or d4_url),
        _d6(),
        _d7(),
        _d8(),
    ]
    counts = {"PASS": 0, "WARN": 0, "FAIL": 0}
    for it in items:
        counts[it["level"]] += 1
    return {
        "generated_at": time.strftime("%Y-%m-%d %H:%M:%S"),
        "url": url,
        "summary": counts,
        "items": items,
        "cost_s": round(time.time() - t0, 1),
    }


def main() -> None:
    args = sys.argv[1:]
    url = None
    if "--url" in args:
        i = args.index("--url")
        if i + 1 < len(args):
            url = args[i + 1]
    report = run_diagnose(url)
    print(f"诊断链接: {report['url']}  耗时 {report['cost_s']}s")
    for it in report["items"]:
        print(f"[{it['level']:<4}] {it['id']} {it['name']}: "
              f"{json.dumps(it['evidence'], ensure_ascii=False)}")
        if it["suggestion"]:
            print(f"       建议: {it['suggestion']}")
    s = report["summary"]
    print(f"汇总: PASS={s['PASS']} WARN={s['WARN']} FAIL={s['FAIL']}")
    if "--json" in args:
        print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
