"""应用配置：工作目录、配置文件读写、默认值。"""
import json
import os
from pathlib import Path

# 数据目录：默认用户目录；开发环境可用 WATERMARK_TOOL_HOME 覆盖（部分沙箱环境限制写用户目录）
APP_DIR = Path(os.environ.get("WATERMARK_TOOL_HOME") or Path.home() / ".watermark-tool")
CONFIG_PATH = APP_DIR / "config.json"

DEFAULT_CONFIG = {
    "remove_platform_wm": True,   # 开关1：去平台水印
    "remove_content_wm": False,   # 开关2：去内容水印（M2 开发中）
    "output_dir": str(APP_DIR / "output"),
    "backend_port": 17890,
    # Bug 反馈邮件（SMTP）；为空则反馈接口返回未配置提示
    "smtp_host": "smtp.qq.com",
    "smtp_port": 465,
    "smtp_user": "",
    "smtp_auth_code": "",
    "feedback_to": "",
    # X 平台登录 Cookie（登录墙推文解析用）；敏感字段，不回传前端
    "x_cookie": "",
}


def ensure_dirs() -> None:
    for d in ("output", "tmp", "models"):
        (APP_DIR / d).mkdir(parents=True, exist_ok=True)


def load_config() -> dict:
    ensure_dirs()
    if CONFIG_PATH.exists():
        try:
            saved = json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            saved = {}
        cfg = {**DEFAULT_CONFIG, **saved}
    else:
        cfg = dict(DEFAULT_CONFIG)
        save_config(cfg)
    return cfg


def save_config(cfg: dict) -> dict:
    ensure_dirs()
    CONFIG_PATH.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
    return cfg
