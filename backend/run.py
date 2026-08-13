import os
import sys
import threading
import time
from pathlib import Path

import uvicorn

from app.config import load_config

# PyInstaller 打包后：playwright 浏览器随包分发，指向包内 ms-playwright 目录（sys._MEIPASS）
if getattr(sys, "frozen", False):
    os.environ.setdefault(
        "PLAYWRIGHT_BROWSERS_PATH",
        str(Path(getattr(sys, "_MEIPASS", Path(sys.executable).parent)) / "ms-playwright"),
    )

# 被 Tauri 拉起时：监听父进程（桌面应用），父进程退出则自杀，避免残留占用端口
if os.environ.get("WATERMARK_TOOL_SPAWNED") == "1" and sys.platform == "win32":
    import ctypes

    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    STILL_ACTIVE = 259
    _parent_pid = os.getppid()

    def _is_alive(pid: int) -> bool:
        kernel32 = ctypes.windll.kernel32
        handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
        if not handle:
            return False
        code = ctypes.c_ulong()
        ok = kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
        kernel32.CloseHandle(handle)
        return bool(ok) and code.value == STILL_ACTIVE

    def _watch_parent():
        while True:
            time.sleep(5)
            if not _is_alive(_parent_pid):
                os._exit(0)

    threading.Thread(target=_watch_parent, daemon=True).start()

if __name__ == "__main__":
    from app.main import app

    cfg = load_config()
    uvicorn.run(app, host="127.0.0.1", port=cfg["backend_port"], log_level="info")
