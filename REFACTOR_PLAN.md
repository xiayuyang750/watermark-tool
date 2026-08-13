# 原生重构计划（REFACTOR_PLAN）

> 状态：规划稿 v0.3 · 2026-08-08
> 用途：作为未来"套壳 → 轻量原生"重构的唯一参考文档，按里程碑逐步执行，每步可独立验证。
> 触发条件：启动本计划中的任一里程碑时，先通读本文档并与现状代码核对。
> 总约束：**换语言不换逻辑**——仅把 Python 实现迁移为 Rust，功能、接口契约、数据格式与解析/下载/风控策略保持不变，非必要不改变既有行为。

---

## 1. 背景与动机

### 1.1 现状

当前产品为"Tauri 2 套壳"架构：

```
Vue 3 前端（WebView2 渲染）
        │  HTTP (127.0.0.1:17890)
Python FastAPI 后端（PyInstaller 打包为 sidecar exe）
        │
        └─ Playwright 驱动 Chromium headless（解析抖音）
```

- 安装包：**134MB**；解压后约 **440MB**
- 重量来源：Python 运行时（~50MB）+ Chromium headless（~180MB）+ Playwright driver + 依赖
- 运行时进程：WebView2 + Python + Chromium，三套运行时

### 1.2 痛点（重构动机）

| 痛点 | 表现 |
|------|------|
| 体积大 | 安装包 134MB，解压 440MB，磁盘占用高 |
| 启动慢 | Python 解释器预热 + Chromium 加载 |
| 占用高 | 常驻 3 个运行时进程 |
| 平台受限 | Python 后端无法跑安卓，移动端被锁死 |

### 1.3 目标平台（2026-08-08 需求澄清后更新）

- **Android（核心目标）**：安卓手机**侧载 APK 直接安装使用**，不上架应用商店
- **Windows**：桌面版保留，与安卓共用同一套 Rust 核心逻辑

> 需求澄清：用户核心诉求为安卓手机安装使用。原计划"Windows 首发、安卓后置"调整为**安卓优先**。Windows 桌面版保留但优先级低于安卓。

### 1.4 三端关系（2026-08-08 架构确认）

- **网页 + 桌面 = 一套代码**：前端 Vue 共用同一份；后端为 Rust 核心（桌面端以 sidecar 形式随应用分发）。网页是桌面软件的开发预览形态（本地起 Rust 后端即可），**不单独维护一套前后端**
- **Android 独立打包**：复用同一 Rust 核心（编译为 `.so` 内嵌），前端复用 Vue（WebView 渲染）
- **Python 后端退役**：重构完成后，网页预览 / 桌面 / 安卓全部使用 Rust 后端，不再维护 Python 后端
- 差异化通过环境/平台判断处理（如 dev/prod API 地址），**不复制代码文件**

---

## 2. 重构目标与边界

### 2.1 目标（必须达成）

1. **体积**：Windows 安装包从 134MB 降到 **≤ 30MB**（不含浏览器运行时）
2. **启动速度**：应用启动到可操作 **≤ 2s**（无解释器预热）
3. **跨平台**：同一套 Rust 核心逻辑在 Windows（sidecar exe）与 Android（Tauri 插件/内嵌服务）复用
4. **前端零改动**：API 契约与现有前端完全兼容
5. **平台范围**：**抖音 + X（Twitter）** 两个平台全部迁移
6. **换语言不换逻辑**：功能、接口契约、数据格式与策略（去水印、重试、Referer/tag 处理、Live 图双文件、登录引导等）与 Python 版保持一致，仅语言与实现方式变化，性能/体积/跨平台收益顺带获得

### 2.2 非目标（明确不做）

- ❌ UI 层原生化（Vue + WebView 保留，非痛点）
- ❌ 重写前端界面
- ❌ 改变产品功能范围（抖音 + X 解析、下载、双开关、反馈、X 登录）

### 2.3 "原生"的定义

本计划所称"原生"= **逻辑层原生**（核心解析/下载逻辑用 Rust 编译为原生代码），**UI 层仍为 WebView**（行业通行做法，非痛点来源）。

---

## 3. 目标架构

```
┌─────────────────────────────────────────────┐
│  Vue 3 前端（WebView2 / Android WebView）     │  ← 不动（网页/桌面/安卓共用）
│         │  HTTP /api/v1（契约不变）           │
├─────────┴───────────────────────────────────┤
│  Tauri 壳（Rust）                             │  ← 壳保留，职责不变
│    ├─ Windows：启动/管理 Rust 后端 sidecar    │
│    └─ Android：内嵌后端服务（同进程）          │
├──────────────────────────────────────────────┤
│  Rust 后端核心（同一 crate 库）                │  ← 本计划全部工作量
│    ├─ HTTP 服务（axum，绑定 127.0.0.1）       │
│    ├─ 解析器 douyin：签名 + CDP 浏览器        │
│    ├─ 解析器 x：纯 HTTP 公共 API + 登录 cookie │  ← 无浏览器依赖
│    ├─ 媒体代理 /media（Range + 按域名 Referer）│
│    ├─ 任务队列（tokio + SQLite）              │
│    ├─ 下载器（reqwest，tag/Referer 策略）     │
│    ├─ 配置（config.json，沿用数据目录约定）    │
│    └─ X 登录引导（Edge CDP，已有实战基础）     │
│                                              │
│  Windows: 编译为独立 exe（sidecar）           │
│  Android: 编译为 .so，作为 Tauri 插件内嵌     │
├──────────────────────────────────────────────┤
│  系统浏览器（CDP 驱动，仅抖音需要）            │
│    ├─ Windows: 系统自带 Edge                  │  ← 免打包浏览器
│    └─ Android: 系统 WebView                  │
└──────────────────────────────────────────────┘
```

**体积收益**：去掉 Python 运行时 + Chromium → Rust 后端 exe 约 10-20MB，安装包 ≤ 30MB。
**X 平台收益**：X 解析为纯 HTTP，安卓端无需任何浏览器即可解析 X，是安卓产品的可靠支柱。

---

## 4. 技术方案

### 4.1 Rust 后端（axum）

| 组件 | 选型 | 说明 |
|------|------|------|
| HTTP 框架 | `axum` | 与现有 FastAPI 路由一一对应 |
| 异步运行时 | `tokio` | 任务队列/并发下载 |
| HTTP 客户端 | `reqwest` | 下载、分享链接重定向 |
| SQLite | `rusqlite`（tokio 封装 `tokio-rusqlite` 或独立线程） | 任务持久化，表结构与现有一致 |
| 配置 | `serde_json` | config.json 读写，字段与现有一致 |

**API 契约（必须与现状完全一致，前端零改动）**：

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/v1/health` | GET | 健康检查 |
| `/api/v1/parse` | POST | 解析（同步返回素材清单，含 cover/image_url 字段） |
| `/api/v1/media` | GET | 媒体流式代理（Range 206 + 按域名 Referer） |
| `/api/v1/tasks` | POST/GET | 创建/列表任务 |
| `/api/v1/tasks/{id}` | GET | 查询任务 |
| `/api/v1/tasks/{id}/cancel` | POST | 取消任务 |
| `/api/v1/config` | GET/PUT | 配置读写（SMTP、x_cookie 等敏感字段过滤） |
| `/api/v1/feedback` | POST | Bug 反馈（SMTP） |
| `/api/v1/diagnose` | GET | 风控诊断（D1-D8） |
| `/api/v1/x/login/start` | POST | X 登录引导（Edge CDP） |
| `/api/v1/x/login/status` | GET | X 登录状态查询 |
| `/api/v1/shutdown` | POST | 优雅关闭（先关浏览器再退出进程） |

请求/响应 JSON 结构与现有 [schemas.py](backend/app/schemas.py)、[main.py](backend/app/main.py) 逐字段对齐（含 `MediaFile.cover` / `image_url`、任务 options 等新增字段）。

### 4.2 浏览器解析（CDP，核心难点）

**原理**：现有 Playwright 方案本质是"启动浏览器 → 打开抖音首页 → 页面内 XHR 调 detail 接口（安全 SDK 自动签名）→ 抓取 JSON"。CDP 方案用系统浏览器实现同样流程，**省掉 180MB 的 Chromium**。

> **已有实战基础（2026-08）**：X 登录引导已用「系统 Edge + `--remote-debugging-port` + `connect_over_cdp`」跑通，验证 CDP 链路在 Windows 可用；M3 风险由"高风险"下调为"有现成经验可复用"。

**Windows（Edge）**：
1. 启动系统 Edge：`msedge --remote-debugging-port=9222 --user-data-dir=<目录> --headless=new`
2. 通过 CDP 连接：HTTP `GET /json/new?url=...` 创建 target，WebSocket 发命令
3. 用到的命令子集：`Page.navigate`、`Runtime.evaluate`、`Target.*`、`Page.reload`
4. 复用现有解析逻辑：加载抖音首页 → 等待安全 SDK → 页面内 `fetch` detail 接口 → 解析 JSON
5. **沿用「常驻复用」策略**（与 Python 版一致）：首次启动后复用页面（后续解析 <1s）、空闲自动关闭释放内存、页面失效/超时自动重建——需用 CDP 命令子集实现同等行为

**CDP 客户端选型**：自研最小实现（`tungstenite` WebSocket + `serde_json`），只实现上述命令子集，避免引入大依赖；若自研遇到协议细节坑，回退到 `chromiumoxide`。

**Android（WebView）**：
- **可行性由 M0 PoC 前置验证**，按结论在以下路线中择一：
  - 方案 A：Android WebView 启用调试 → `adb forward` 端口转发 → CDP 连接，加载抖音执行页面内解析
  - 方案 B：安卓端移动 UA 纯 HTTP + Rust 签名直连
  - 方案 C：应用内 WebView 直接加载抖音页面 + JS 注入抓取
- **风险提示**：安卓解析策略是产品成败关键，M0 设 PoC 验证；若全部不可行则收缩安卓范围（见 M0 失败预案）。**X 平台不受此影响**（纯 HTTP，见 4.4）

### 4.3 签名算法移植

- 现状：[douyin_sig.py](backend/app/parsers/douyin_sig.py) 含 ABogus（GPL v3）与 XBogus（Apache 2.0）
- Rust 移植范围：
  - `XBogus`（Apache 2.0，无许可证污染）→ 直接移植
  - `ABogus`（GPL v3）→ **许可证注意**：GPL 传染，不建议原样移植；策略为（a）优先用 XBogus/纯 HTTP 备用通道，或（b）基于公开实现重写非 GPL 版本
- 验收：Rust 实现与 Python 实现输出签名**逐字符一致**（单元测试对比样本）

### 4.4 X 平台解析（纯 HTTP，无浏览器依赖）

- 现状 [x.py](backend/app/parsers/x.py) 三级链路：vxtwitter 公共 API 优先 → fxtwitter 兜底 → Playwright + cookie（登录墙）
- Rust 移植要点：
  - **主链路为纯 HTTP**（vxtwitter/fxtwitter），reqwest 即可完成，**无需浏览器**——安卓端解析 X 零浏览器依赖
  - 策略保持一致：视频直链补 `tag=29` 签名参数、下载/代理按域名选 Referer（twimg → `https://x.com/`）、GIF 按无声 mp4 处理、视频无平台水印
  - 登录墙：cookie（含 `auth_token`）存 config.json（敏感字段不回传）；登录引导复用 Edge CDP 实战经验（固定登录档案 + 反自动化参数 + 状态轮询）
- 验收：与 Python 版解析结果一致（真实推文链接对比，含敏感/公开/登录墙场景）

### 4.5 解析逻辑移植（内容分类）

- [douyin.py](backend/app/parsers/douyin.py) 的 `parse_aweme` / `image_file`：
  - 内容分类优先级：Live图（video 字段）> 动图（animated/gif）> 静态图
  - Live 图 = 静态照片（`url_list[0]`）+ 视频（`play_addr`）双文件（`image_url` / `cover` 字段保持）
  - 去水印：`playwm` → `play`
  - 图集多图、视频单文件
- 分享文本 → URL 提取 → 短链重定向 → aweme_id 的流程照搬

### 4.6 任务队列与下载

- 现状：[manager.py](backend/app/tasks/manager.py) 线程 worker + `queue.Queue` + SQLite
- Rust：tokio worker 消费 `mpsc::channel` + rusqlite 持久化，状态机（pending/running/done/failed/cancelled）不变
- 下载策略保持一致（[manager.py](backend/app/tasks/manager.py) 现状）：
  - 扩展名：video/livephoto→mp4、gif→gif、图片按 URL 推断（webp/png/jpg）
  - Live 图下载「照片 + 视频」双文件（direct 任务带 `image_url` options）
  - **Referer 按 CDN 域名选择**：twimg.com → `https://x.com/`，其余 → `https://www.douyin.com/`（`_referer_for`）
  - 下载头带 UA + 正确 Referer；进度按 content-length 计算

### 4.7 媒体代理与优雅关闭

- `/api/v1/media`：httpx/reqwest 流式转发，透传浏览器 Range（206）、按域名 Referer、`Cache-Control: public, max-age=86400`
- `/api/v1/shutdown`：先关闭常驻浏览器再退出进程（桌面端退出调用，避免残留 Chromium）
- 浏览器生命周期策略（抖音）：首次解析懒启动 → 常驻复用 → 空闲 10 分钟自动关闭 → 失效/超时自动重建

### 4.8 数据与配置约定（保持）

- 数据目录：`WATERMARK_TOOL_HOME` 环境变量 or `~/.watermark-tool`（沿用现有回退逻辑）
- `config.json` 字段：remove_platform_wm / remove_content_wm / output_dir / backend_port / smtp_* / x_cookie
- 端口：17890（不变）
- 敏感字段不回传：smtp_auth_code / smtp_user / smtp_host / smtp_port / feedback_to / x_cookie

---

## 5. 模块映射（Python → Rust）

| Python 模块 | Rust 对应 | 工作量 |
|-------------|----------|--------|
| app/main.py（FastAPI） | axum 路由 | 低 |
| app/schemas.py | serde 结构体 | 低 |
| app/config.py | serde_json 配置 | 低 |
| app/notify.py（SMTP 反馈） | 邮件发送（可选 lettre） | 低 |
| app/parsers/douyin.py | douyin 解析器 | 中 |
| app/parsers/douyin_sig.py | 签名算法 | 中（注意许可证） |
| app/parsers/douyin_browser.py | CDP 浏览器解析 | **高（核心难点）** |
| app/parsers/x.py | X 解析器（纯 HTTP）+ 登录引导（Edge CDP） | 中 |
| app/tasks/manager.py + db.py | tokio 任务队列 | 中 |
| app/anticrawler/diagnose.py | 诊断 D1-D8 | 中 |
| /api/v1/media（媒体代理） | axum 流式转发（Range/Referer） | 低 |

---

## 6. 里程碑与验收标准

每个里程碑独立可验证，全部通过后进入下一个。

> **执行阶段（2026-08-08 用户决策，替代原"安卓优先"排序）**：分两步执行。
> - **第一步（当前）**：网页端 + Windows 桌面端 Rust 重构（M1 → M2 → M3 → 打包发布），验证 Rust 后端与现有 Python 版功能/逻辑完全一致
> - **第二步（后置，另起文档细化）**：安卓端（M0 安卓解析 PoC → M4 安卓接入 → M5 可选内容去水印）；因 Tauri Android 构建链复杂，届时单独编写安卓执行方案
>
> 排序原则：先用相对简单的 Windows/网页端完成语言迁移与验证，积累 CDP/签名移植经验，再投入安卓端。

### M0：安卓抖音解析 PoC（第二步执行，前置概念验证）

目标：以最小成本验证"安卓上能否解析抖音"（**仅抖音需要 PoC；X 为纯 HTTP，无需验证**），结论决定安卓路线。**本里程碑属于第二步，先在第一步完成后单独细化再执行。**

- [ ] PoC-1 移动端解析验证（Windows 上模拟，低成本）：用移动端 UA 打开抖音移动版页面，验证（a）页面内抓接口（b）JS 带签名直连 API 两条路是否可取回真实 aweme JSON
- [ ] PoC-2 真机验证：Tauri 安卓最小工程，系统 WebView 加载抖音页面试探解析
- 验收：**真机拿到真实作品 JSON**（视频/图集各一）；输出 PoC 结论报告（可行/部分可行/不可行 + 推荐路线）
- 失败预案：若安卓抖音解析确不可行 → 收缩安卓范围为"X 解析 + 播放/管理壳 + 抖音需桌面配合解析"（更新需求，重新评审）

### M1：Rust 后端骨架（不接解析） ✅ 已完成（2026-08-08）

- [x] axum 服务跑通全部 API 端点（见 4.1 表，含 /media、/feedback、/x/login/*、/shutdown），JSON 与现有完全一致
- [x] config 读写 + 数据目录逻辑 + 敏感字段过滤
- [x] SQLite 任务表 + 队列状态机（任务可用 mock 下载验证）
- 验证：现有前端直接连 Rust 后端，健康检查/设置页/任务中心/反馈/X 登录状态查询全通过（解析返回占位错误）

### M2：签名算法移植 ✅ 已完成（2026-08-08）

- [x] XBogus Rust 实现，与 Python 输出样本逐字符一致
- [x] ABogus 许可证评估结论（保留纯 HTTP 备用通道或重写）
- 验证：单元测试 + 真实链接纯 HTTP 解析成功率对比

> **许可证评估结论**：ABogus（GPL v3，依赖 SM3/gmssl）许可证传染，**不移植**；
> 纯 HTTP 备用通道统一改用 **XBogus**（Apache 2.0，无外部依赖），
> 规避 GPL 污染且与 Python 版 XBogus 输出逐字符一致（固定样本单测固化）。
> 真实链接纯 HTTP 成功率对比随 M3 解析接入后一并验证。

### M3：解析接入（Windows 主里程碑，抖音成果可平移安卓） ✅ 已完成（2026-08-08）

- [x] **抖音**：Edge CDP 启动/连接/页面内解析流程跑通（含常驻复用策略），结果与现有 Playwright 方案一致（视频/图集/动图/Live图）
- [x] **X**：vxtwitter/fxtwitter 纯 HTTP 解析接入（tag=29、Referer 策略）
- [x] 任务队列接入真实下载（含 Live 图双文件、按域名 Referer）
- [x] 生产模式 UI 实测（安装包 → 解析 → 下载全流程，抖音 + X 真实链接成功）

> **M3 实测结论（2026-08-08）**：
> - 抖音：Edge CDP（`--remote-debugging-port` + `--user-agent` 参数）跑通；detail 接口返回完整 JSON（11 万字符），首次解析含浏览器启动约 4s，浏览器常驻复用二次解析秒回；图集/Live 图（image_url+cover）字段齐全
> - **关键坑**：CDP 的 `Network.setUserAgentOverride` 跨连接不持久，抖音会把 headless 特征识别出来并重定向到验证码页 → 必须用启动参数 `--user-agent` 固定 UA（与 Python 版 `new_context(user_agent=UA)` 等价）
> - 连续解析同一作品多次会被抖音临时限流（返回空数据），与 Python 版行为一致，冷却后自动恢复
> - X：两个真实链接（视频/封面）解析成功；媒体代理 Range 206 验证通过（bytes 0-1023/5254962）；5MB 视频下载完整
> - **打包验证（0.2.2）**：安装包 5.1MB（原 134MB，-96%）；桌面壳自动拉起 Rust 后端；生产模式抖音图集 6 张解析 + X 视频 5MB 下载全通过
> - 待收尾（非阻塞）：X 登录墙 cookie（Edge CDP 登录引导）、Python 打包目录退役清理

### M4：安卓正式接入（基于 M0 结论）

- [ ] Tauri Android 工程初始化，Rust 核心编译 .so
- [ ] 按 M0 结论实现安卓抖音解析（CDP/WebView 内解析/纯 HTTP 中择一）；**X 平台直接可用（纯 HTTP）**
- [ ] Windows 安装包体积验证 ≤ 30MB，安卓 APK ≤ 15MB
- 验证：安卓真机解析（X 必成、抖音按 M0）+ 下载；Windows 回归不坏

### M5（可选/后置）：内容水印去除

- [ ] Rust 调用 ONNX Runtime，跑 LaMa/ProPainter 类模型
- 验证：上传本地图片/视频去水印效果与 Python 版一致

---

## 7. 风险与应对

| 风险 | 等级 | 应对 |
|------|------|------|
| **安卓抖音解析不可行**（WebView 抓接口/签名被风控） | **高** | **M0 PoC 前置验证**；失败则收缩安卓范围为"X 解析 + 播放/管理壳"（X 纯 HTTP 不受影响） |
| Android WebView CDP 链路（adb 转发）不可行 | 高 | M4 备选：WebView 内解析 / 纯 HTTP + 签名；再不行收缩安卓范围 |
| X 第三方公共 API（vxtwitter/fxtwitter）限流/失效 | 中 | 保留备用解析源（syndication / 官方嵌入 / 登录后 GraphQL）；错误按层分类提示 |
| CDP 驱动 Edge 兼容性（Edge 版本差异/headless 模式行为） | 中 | **已有实战基础**（X 登录引导已用 Edge CDP）；预留 chromiumoxide 回退；保底为仍打包 Chromium（牺牲体积） |
| 抖音风控变化（签名/浏览器方案失效） | 中 | 沿用 [ANTI_RISK_PLAN.md](ANTI_RISK_PLAN.md) 对抗框架；D1-D8 诊断工具同步移植 |
| ABogus GPL 许可证污染 | 中 | 优先 XBogus/备用通道；重写非 GPL 实现 |
| 解析行为不一致（Python→Rust 移植偏差） | 中 | **换语言不换逻辑**：每里程碑用真实链接对比验证，差异点单测固化 |

---

## 8. 参考

- 现有设计：[DESIGN.md](DESIGN.md)
- 风控对抗：[ANTI_RISK_PLAN.md](ANTI_RISK_PLAN.md)
- 现状解析器：[douyin.py](backend/app/parsers/douyin.py) / [douyin_browser.py](backend/app/parsers/douyin_browser.py) / [douyin_sig.py](backend/app/parsers/douyin_sig.py) / [x.py](backend/app/parsers/x.py)
- 现状后端入口：[main.py](backend/app/main.py) / [run.py](backend/run.py) / [tasks/manager.py](backend/app/tasks/manager.py)
- 解析维护参考：`.trae/skills/web-reverse-engineering/references/platform-parsing.md`（维护与风控对抗）
- Tauri 安卓文档：https://v2.tauri.app/develop/

---

## 9. 变更记录

| 日期 | 版本 | 变更 |
|------|------|------|
| 2026-08-08 | v0.1 | 初稿：方案框架、技术选型、里程碑 |
| 2026-08-08 | v0.2 | 需求澄清（安卓侧载 APK 为核心）：优先级反转为安卓优先；新增 M0 安卓解析 PoC；Windows 保留，双平台共用 Rust 核心 |
| 2026-08-08 | v0.3 | 范围扩展至 X 平台（纯 HTTP，无浏览器依赖）；API 契约补全（/media、/feedback、/x/login/*、/shutdown）；明确三端关系（网页+桌面一套、Android 独立复用 Rust 核心、Python 退役）；Edge CDP 已有实战基础（X 登录引导）；明确总约束「换语言不换逻辑」 |
