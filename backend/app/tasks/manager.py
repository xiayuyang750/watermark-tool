"""任务管理器：线程 worker + 线程安全队列 + SQLite 持久化。

FastAPI 同步接口在线程池执行，任务入队来自任意线程；
使用 queue.Queue（线程安全）而非 asyncio.Queue，避免跨线程唤醒问题。
任务级错误隔离：单个任务失败只影响自己，不中断队列与其他平台任务。
"""
from __future__ import annotations  # 类体内方法注解延迟求值，避免 list 等被类内方法遮蔽

import queue
import re
import threading
import time
import uuid
from pathlib import Path

import httpx

from ..config import load_config
from .db import DB

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
)


def detect_platform(url: str) -> str:
    u = url.lower()
    if "douyin.com" in u or "iesdouyin" in u or "v.douyin" in u:
        return "douyin"
    if "x.com" in u or "twitter.com" in u:
        return "x"
    return ""


def _sanitize_filename(name: str) -> str:
    name = re.sub(r'[\\/:*?"<>|\r\n]', "_", name).strip() or "untitled"
    return name[:60]


def _referer_for(url: str) -> str:
    """按 CDN 域名选择合法 Referer，避免陌生来源被拒（抖音/ X 各自主机校验）。"""
    if "twimg.com" in url:
        return "https://x.com/"
    return "https://www.douyin.com/"


class TaskManager:
    def __init__(self):
        self.db = DB()
        self.queue: queue.Queue = queue.Queue()
        self.worker: threading.Thread | None = None
        self._tasks: dict[str, dict] = {}
        self._lock = threading.Lock()
        self._load_from_db()

    # ---- 生命周期 ----

    def start(self) -> None:
        if self.worker is None or not self.worker.is_alive():
            self.worker = threading.Thread(target=self._run, name="task-worker", daemon=True)
            self.worker.start()

    def stop(self) -> None:
        pass  # 守护线程随进程退出

    # ---- 外部接口 ----

    def create(self, task_type: str = "link", url: str | None = None, options: dict | None = None) -> dict:
        tid = uuid.uuid4().hex[:12]
        created_at = time.strftime("%Y-%m-%d %H:%M:%S")
        info = {
            "id": tid, "type": task_type, "status": "pending", "progress": 0,
            "output": None, "error": None, "created_at": created_at,
            "url": url, "options": options or {}, "cancelled": False,
        }
        with self._lock:
            self._tasks[tid] = info
        self.db.upsert(tid, type=task_type, status="pending", progress=0,
                       created_at=created_at, options=options or {})
        self.queue.put(tid)
        return self.public_view(tid)

    def get(self, tid: str) -> dict | None:
        return self.public_view(tid)

    def list(self) -> list[dict]:
        with self._lock:
            ids = sorted(self._tasks, key=lambda t: self._tasks[t]["created_at"], reverse=True)
        return [self.public_view(tid) for tid in ids]

    def cancel(self, tid: str) -> bool:
        with self._lock:
            info = self._tasks.get(tid)
            if not info or info["status"] not in ("pending", "running"):
                return False
            info["cancelled"] = True
        return True

    def public_view(self, tid: str) -> dict | None:
        with self._lock:
            info = self._tasks.get(tid)
            if not info:
                return None
            return {
                "id": info["id"], "type": info["type"], "status": info["status"],
                "progress": info["progress"], "output": info["output"],
                "error": info["error"], "created_at": info["created_at"],
            }

    def _update(self, tid: str, **fields) -> None:
        with self._lock:
            info = self._tasks.get(tid)
            if info:
                for k, v in fields.items():
                    if k in ("status", "progress", "output", "error"):
                        info[k] = v
        self.db.update(tid, **fields)

    # ---- 内部 ----

    def _load_from_db(self) -> None:
        for row in self.db.list_all():
            if row["status"] in ("pending", "running"):
                row["status"] = "failed"
                row["error"] = "应用重启，任务中断"
                self.db.update(row["id"], status="failed", error=row["error"])
            with self._lock:
                self._tasks[row["id"]] = {**row, "url": None, "options": {}, "cancelled": False}

    def _run(self) -> None:
        while True:
            tid = self.queue.get()
            try:
                with self._lock:
                    info = self._tasks.get(tid)
                if info and not info["cancelled"]:
                    self._process(tid)
            finally:
                self.queue.task_done()

    def _process(self, tid: str) -> None:
        self._update(tid, status="running", progress=5)
        try:
            with self._lock:
                info = self._tasks[tid]
                url = info.get("url") or ""
                options = info.get("options") or {}
            cfg = load_config()
            out_dir = Path(options.get("output_dir") or cfg["output_dir"])
            out_dir.mkdir(parents=True, exist_ok=True)

            if info["type"] == "direct":
                # 直链下载：url 为素材直链，options 携带 kind/title/image_url
                kind = options.get("kind") or "video"
                title = options.get("title") or "untitled"
                out = self._download(
                    url, out_dir, title, kind, tid,
                    image_url=options.get("image_url"),
                )
                output = " | ".join(map(str, out)) if isinstance(out, list) else str(out)
                self._update(tid, status="done", progress=100, output=output)
                return

            # link 任务：解析后下载全部素材
            platform = detect_platform(url)
            if not platform:
                raise RuntimeError("无法识别的平台链接")
            from ..parsers import PARSERS
            parser = PARSERS.get(platform)
            if parser is None:
                raise RuntimeError(f"「{platform}」平台解析尚未支持（X 平台计划在 M4 里程碑接入）")

            result = parser.parse(url, options.get("remove_platform_wm", True))
            self._update(tid, progress=15)

            if not result.files:
                raise RuntimeError("解析结果为空")
            total = len(result.files)
            paths = []
            for i, f in enumerate(result.files):
                start_pct = 15 + int(i / total * 80)
                end_pct = 15 + int((i + 1) / total * 80)
                name = result.title if total == 1 else f"{result.title}_{i + 1}"
                out = self._download(
                    f.url, out_dir, name, f.kind, tid, start_pct, end_pct,
                    image_url=f.image_url,
                )
                if isinstance(out, list):
                    paths.extend(map(str, out))
                else:
                    paths.append(str(out))
            self._update(tid, status="done", progress=100, output=" | ".join(paths))
        except Exception as exc:
            with self._lock:
                cancelled = self._tasks.get(tid, {}).get("cancelled", False)
            if cancelled:
                self._update(tid, status="cancelled", error="用户取消")
            else:
                self._update(tid, status="failed", error=str(exc))

    def _download(
        self, url: str, out_dir: Path, title: str, kind: str, tid: str,
        start_pct: int = 15, end_pct: int = 99, image_url: str | None = None,
    ) -> Path | list[Path]:
        """下载单个素材；Live 图（带 image_url）下载「静态照片 + 视频」双文件。"""
        if kind == "livephoto" and image_url:
            mid = start_pct + (end_pct - start_pct) * 2 // 3
            video_path = self._download(url, out_dir, title, "video", tid, start_pct, mid)
            photo_path = self._download(image_url, out_dir, title, "image", tid, mid, end_pct)
            return [video_path, photo_path]
        ext = ".mp4" if kind in ("video", "livephoto") else ".gif" if kind == "gif" else self._ext_from_url(url)
        out_path = out_dir / f"{_sanitize_filename(title)}_{time.strftime('%Y%m%d%H%M%S')}{ext}"
        total = 0
        with httpx.Client(
            headers={"User-Agent": UA, "Referer": _referer_for(url)},
            follow_redirects=True,
            timeout=120,
        ) as client:
            with client.stream("GET", url) as resp:
                resp.raise_for_status()
                content_length = int(resp.headers.get("content-length") or 0)
                with open(out_path, "wb") as f:
                    for chunk in resp.iter_bytes():
                        with self._lock:
                            cancelled = self._tasks[tid].get("cancelled", False)
                        if cancelled:
                            raise RuntimeError("cancelled")
                        f.write(chunk)
                        total += len(chunk)
                        if content_length:
                            pct = start_pct + int(total / content_length * (end_pct - start_pct))
                            self._update(tid, progress=max(start_pct, min(end_pct, pct)))
        return out_path

    @staticmethod
    def _ext_from_url(url: str) -> str:
        """按直链路径推断扩展名，避免 .webp 等格式存成 .jpg。"""
        path = url.split("?")[0].lower()
        for e in (".webp", ".png", ".gif", ".jpeg", ".jpg"):
            if path.endswith(e):
                return e
        return ".jpg"


manager = TaskManager()
