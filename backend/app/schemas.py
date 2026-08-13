"""API 请求/响应模型。"""
from typing import Optional

from pydantic import BaseModel


class ParseRequest(BaseModel):
    url: str
    remove_platform_wm: bool = True


class MediaFile(BaseModel):
    kind: str  # video / image / gif / livephoto
    url: str
    label: Optional[str] = None
    cover: Optional[str] = None  # 封面/预览图（视频与 Live 图可选）
    image_url: Optional[str] = None  # Live 图的静态照片直链（与 url 视频组成原生 Live 图）


class ParseResult(BaseModel):
    platform: str
    title: str
    media_type: str  # video / image
    files: list[MediaFile]


class TaskCreate(BaseModel):
    type: str = "link"  # M1 仅 link
    url: Optional[str] = None
    options: dict = {}


class FeedbackRequest(BaseModel):
    content: str
    contact: Optional[str] = None  # 联系方式（邮箱等），便于开发者联系


class TaskRead(BaseModel):
    id: str
    type: str
    status: str  # pending / running / done / failed / cancelled
    progress: int
    output: Optional[str] = None
    error: Optional[str] = None
    created_at: str
