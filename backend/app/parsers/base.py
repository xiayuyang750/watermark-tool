"""平台解析器统一接口。"""
from abc import ABC, abstractmethod
from dataclasses import dataclass


@dataclass
class MediaFile:
    kind: str  # video / image / gif / livephoto
    url: str
    label: str | None = None
    cover: str | None = None  # 封面/预览图（视频与 Live 图可选）
    image_url: str | None = None  # Live 图的静态照片直链（与 url 视频组成原生 Live 图）


@dataclass
class ParseResult:
    platform: str
    title: str
    media_type: str  # video / image
    files: list[MediaFile]


class PlatformParser(ABC):
    platform: str

    @abstractmethod
    def parse(self, url: str, remove_platform_wm: bool) -> ParseResult:
        """解析分享链接，返回作品信息。失败时抛出带说明的异常。"""
        raise NotImplementedError
