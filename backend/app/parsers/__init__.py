"""解析器注册表。每个平台一个独立包，互不依赖，单平台异常不影响其他平台。"""
from .douyin_browser import DouyinBrowserParser
from .x import XParser

PARSERS = {
    "douyin": DouyinBrowserParser(),
    "x": XParser(),
}
