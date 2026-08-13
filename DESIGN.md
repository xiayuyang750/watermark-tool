# 多平台作品解析与水印去除工具 — 详细设计文档

> 版本：v0.1（草稿，待确认）
> 日期：2026-08-07

## 1. 项目概述

一款本地优先的多平台作品解析、下载与水印去除桌面工具。

- **输入方式一**：粘贴抖音 / X 等平台的作品分享链接，解析并下载原始作品（可选去平台水印）。
- **输入方式二**：上传本地文件（图片 / 视频 / 实况照片），检测并去除内容水印（画面内水印）。
- **核心原则**：隐私优先，数据不出本机；功能模块解耦，UI 与后端分离，为未来 Android 端扩展留路。

### 1.1 目标平台

| 平台 | 阶段 | 说明 |
|---|---|---|
| Windows 桌面 | v1 | 全功能：解析 + 下载 + 本地 AI 修复 |
| Android | 扩展（M4 后评估） | Tauri 2 同套 Vue UI 打包 APK；推理走远端或仅保留网络功能 |

### 1.2 手动开关设计

- **开关 1：去平台水印**（解析时直接取无水印直链，成本≈0）
- **开关 2：去内容水印**（检测 + 修复，本地 AI 推理）
- 两个开关均关闭 = 保留原始文件

## 2. 系统架构

```
┌─────────────────────────────────────────────┐
│  Tauri 2 桌面壳 (Windows)                    │
│  Vue 3 + Vite + Element Plus                │
│  页面：链接解析 / 本地文件 / 任务中心 / 设置    │
└──────────────┬──────────────────────────────┘
               │ HTTP (localhost:17890)
┌──────────────▼──────────────────────────────┐
│  FastAPI 后端（Python 3.11）                  │
│  ┌──────────┬──────────┬──────────────────┐  │
│  │ parsers/ │ detect/  │  inpaint/        │  │
│  │ 抖音·X    │ YOLO·OCR │  LaMa·ProPainter│  │
│  └──────────┴──────────┴──────────────────┘  │
│  media/ (ffmpeg 拆帧合帧)   tasks/ (任务队列)  │
│  config/ (开关与引擎配置)                     │
└──────────────┬──────────────────────────────┘
               │ 本机文件系统（隐私优先）
        ~/.watermark-tool/ (tmp / output / models / tasks.db)
```

## 3. 技术选型汇总

| 层 | 选型 | 备注 |
|---|---|---|
| 桌面壳 | Tauri 2（Rust） | 支持 Android 扩展；包体小 |
| 前端 | Vue 3 + Vite + Element Plus + Pinia + Axios | 与后端仅 HTTP 通信 |
| 后端 | Python 3.11 + FastAPI + uvicorn | 本地服务，localhost 绑定 |
| 任务 | 线程 worker + queue.Queue + SQLite 持久化 | 单用户桌面场景够用，避免跨线程 asyncio 陷阱 |
| 媒体 | ffmpeg（拆帧/合帧/转码） | 通过 subprocess 调用 |
| 解析-抖音 | Playwright 浏览器方案（主）：页面安全 SDK 自动签名 + 浏览器指纹 | 实测 2026-08 抖音反爬升级后，纯 a_bogus 方案被静默拒绝；浏览器方案无需登录即可解析公开作品 |
| 解析-X | yt-dlp | 稳定 |
| 检测 | YOLOv8（水印框） + PaddleOCR（文字水印） + SAM2（手动精修，可选） | 自动 + 手动双模式 |
| 图像修复 | LaMa（默认） / SD-Inpainting（复杂背景可选） | LaMa 可 CPU |
| 视频修复 | ProPainter（主推） / STTN（轻量备选） / LaMa 逐帧 | 见 §4 显存约束 |
| 增强（可选） | Real-ESRGAN / GFPGAN | M4 后按需 |

## 4. 硬件评估（当前机器）

| 项 | 值 | 影响 |
|---|---|---|
| GPU | RTX 3060 Laptop **6GB** | ProPainter 可跑但显存紧张 |
| 内存 | 16GB DDR5 | 够用 |
| 硬盘 | 512GB + 1TB | 视频中间帧占空间，需清理策略 |

**6GB 显存对策**：
- ProPainter 使用 fp16 推理 + 降分辨率（≤720p）+ 减少传播帧数
- 长视频自动分片处理，逐段修复后合并
- 显存不足时自动降级：ProPainter → LaMa 逐帧（牺牲时序连贯性）
- 启动时探测 GPU 能力，在设置页展示并给出建议

## 5. 模块详细设计

### 5.1 解析器模块（parsers/）

统一接口：

```python
class PlatformParser(ABC):
    platform: str
    def parse(self, url: str, remove_platform_wm: bool) -> ParseResult: ...
```

- `douyin_browser.py`（主）：Playwright 启动本地 Chromium → 访问抖音 → 在页面上下文内 XHR 调用 `aweme/v1/web/aweme/detail`（页面安全 SDK 自动签名）→ 解析原生素材。**不注入外部 Cookie**（实测旧 Cookie 绑定浏览器指纹反而被拒）。公开作品无需登录。
- `douyin.py`（备）：a_bogus/X-Bogus 纯 HTTP 方案，2026-08 实测被反爬静默拒绝，保留作参考与极端环境兜底。
- `x/`：封装 yt-dlp（M4 接入）
- 任务级错误隔离：单平台/单任务失败不影响其他任务。

### 5.2 水印检测模块（detect/）

- **自动检测**：YOLOv8 输出水印框 → 生成 mask；PaddleOCR 识别文字水印区域，二者结果合并
- **手动模式**：前端 Canvas 涂抹生成 mask 上传；可选 SAM2 点选精修
- 输出统一为 `mask`（单通道 0/255 图像）

### 5.3 水印修复模块（inpaint/）

```python
class Inpainter(ABC):
    def inpaint(self, image: np.ndarray, mask: np.ndarray) -> np.ndarray: ...
```

- **图像**：LaMa（ONNX 部署，CPU/GPU 双支持）
- **视频**：ProPainter 批量帧输入；降级 STTN / LaMa 逐帧
- 引擎选择逻辑：配置项 + 自动降级（显存不足 / 视频过长）

### 5.4 媒体编排（media/）

- 拆帧：`ffmpeg -i in.mp4 -q:v 2 frames/%06d.png`
- 合帧：按原始 fps/编码回写 `libx264 -crf 18 -pix_fmt yuv420p`
- 实况照片（Live Photo）：解析 HEIC + MOV 组合，图片走 LaMa，MOV 拆帧修复后合回
- 中间帧自动清理（任务完成后删除 tmp 目录）

### 5.5 任务系统（tasks/）

- 任务状态机：`pending → running → done / failed`
- 字段：`id, type(link/upload), platform, input, options, progress(0-100), output, error, created_at`
- SQLite 持久化，失败可重试，进度通过轮询或 SSE 推送
- 单 worker 串行执行（本地单用户，避免多任务争抢 GPU）

### 5.6 配置与开关（config/）

```jsonc
{
  "remove_platform_wm": true,   // 开关1
  "remove_content_wm": false,   // 开关2
  "detect_mode": "auto",        // auto / manual
  "engine": {                   // 引擎选择
    "image": "lama",
    "video": "propainter"
  },
  "video": { "max_res": 720, "fp16": true, "clip_seconds": 30 },
  "models_dir": "~/.watermark-tool/models",
  "output_dir": "~/.watermark-tool/output"
}
```

## 6. API 设计（FastAPI，仅 localhost）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/health` | 含 GPU/CPU 探测信息 |
| POST | `/api/v1/parse` | `{url, remove_platform_wm}` → `{platform, title, files[]}` |
| POST | `/api/v1/tasks` | 创建任务（type=link 或 type=upload） |
| GET | `/api/v1/tasks` | 任务列表 |
| GET | `/api/v1/tasks/{id}` | 任务详情 + 进度 |
| POST | `/api/v1/tasks/{id}/cancel` | 取消任务 |
| POST | `/api/v1/files` | 上传本地文件 → `{file_id}` |
| POST | `/api/v1/tasks` | `{type:"upload", file_id, mask?, engine}` |
| GET/PUT | `/api/v1/config` | 读/写配置 |
| GET | `/api/v1/models` | 模型状态（已下载/可下载/大小） |
| POST | `/api/v1/models/download` | 触发模型下载（支持国内镜像源） |

## 7. 目录结构

```
watermark-tool/
├─ DESIGN.md
├─ desktop/                 # Tauri 2 + Vue 3
│   ├─ src/                 # Vue 前端
│   └─ src-tauri/           # Rust 壳
├─ backend/
│   ├─ app/
│   │   ├─ main.py          # FastAPI 入口
│   │   ├─ parsers/         # douyin_browser（主）/ douyin（备）/ x
│   │   ├─ detect/          # yolo / ocr / sam2
│   │   ├─ inpaint/         # lama / propainter / sttn
│   │   ├─ media/           # ffmpeg 编排
│   │   ├─ tasks/           # 线程任务队列 + SQLite
│   │   └─ config.py
│   └─ requirements.txt
└─ scripts/                 # 打包、模型下载脚本
```

## 8. MVP 里程碑与验收标准

| 里程碑 | 内容 | 验收标准 |
|---|---|---|
| **M1** | 桌面壳 + 链接解析下载 | 粘贴抖音链接 → 无水印视频落地本地 |
| **M2** | 本地上传图片 → 检测/框选 → LaMa 修复 | 水印消除，放大无明显痕迹 |
| **M3** | 视频拆帧 → ProPainter → 合帧 | ≤30s/720p 视频，修复后无闪烁 |
| **M4** | X 平台 + 批量队列 + 模型管理 + Android 评估 | 批量处理稳定，模型可管理 |

每个里程碑独立可交付，M1/M2 优先。

## 9. Android 扩展路径（M4 后评估）

- **能复用**：整套 Vue UI（Tauri 2 移动端）+ 后端 API 契约
- **不能复用**：Python + PyTorch 推理栈（Android 无可行部署）
- **候选方案**：
  1. 手机端瘦客户端 → 连回自建后端（家庭 PC + Tailscale/HTTPS）
  2. 手机端仅保留解析下载（纯网络请求）
  3. 维持纯桌面，不做移动端
- 国内分发 APK 需 App 备案，列入 M4 决策清单

## 10. 合规边界（明确写死）

- 工具定位：个人本地工具，**数据不出机**
- 仅面向用户合法持有的素材（自有内容 / 授权内容）
- 不包含任何规避内容审核的功能；不处理违禁内容
- 下载内容版权归原作者，仅限个人学习研究
- 界面内置免责声明；商用或分发前需重新评估合规

## 11. 风险清单

| 风险 | 影响 | 缓解 |
|---|---|---|
| 抖音反爬升级（签名/风控） | 解析失效 | 浏览器方案跟随页面 SDK 自动适配；任务级隔离；跟踪社区方案 |
| Chromium 首次安装 | 首次体验差 | 启动时检测并引导安装（playwright install chromium） |
| 6GB 显存 OOM | 视频修复失败 | 分片 / 降分辨率 / fp16 / 自动降级引擎 |
| 复杂背景水印效果差 | 用户观感差 | 手动 mask + 多引擎可选 + 结果前后对比 |
| 平台风控/封 IP | 下载失败 | 限速、失败重试 |
| 磁盘被中间帧占满 | 磁盘空间不足 | 任务结束自动清理 tmp |
