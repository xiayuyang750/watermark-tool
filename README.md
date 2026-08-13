# 🎬 Watermark Tool 水印工坊

一个轻量、免安装运行时的**抖音 / X（Twitter）视频图片下载工具**，支持解析、在线预览、一键下载。Windows 桌面端基于 Tauri 2 构建，后端为 Rust（axum）原生实现，安装包仅 **5MB 左右**。

> ⚠️ 仅限个人合法素材使用，请尊重原创与平台规则。

---

## ✨ 功能特性

### 📱 抖音（Douyin）
- 支持内容类型：**视频 / 图片图集 / 动图（GIF）/ Live 图（动态照片）**
- 去平台水印开关（开关对 X 自动不生效——X 视频本身无水印）
- 解析后直接预览：图片原图、视频封面 + 点击播放、图集左右切换
- Live 图 = 静态照片 + 视频双文件，接近平台原生观感
- **浏览器常驻复用**：首次解析冷启动约 10-20s，此后复用页面内请求，解析 2-3 秒
- 空闲 10 分钟自动释放浏览器（省内存），应用退出时优雅关闭不残留进程

### 🐦 X（Twitter）
- 支持内容类型：**视频 / 图片 / GIF**（GIF 本质是无声 mp4，统一按视频处理）
- 链接格式：`x.com/<用户>/status/<ID>`、`twitter.com/...`、`x.com/i/status/<ID>`，兼容直接粘贴 15-20 位推文 ID
- **三级解析链路**：vxtwitter 公共 API → fxtwitter 兜底 → 登录墙兜底（Edge CDP + 登录态）
- **X 登录引导**：设置中一键弹出 Edge 登录（支持手机扫码），固定登录档案长期有效，登录一次后登录墙 / 私密推文可直接解析

### 🔧 通用能力
- **本地媒体代理**：媒体统一经本地代理（带正确 UA / Referer），视频支持 Range 拖拽播放，杜绝 403
- **解析历史按平台独立**：抖音 / X 分开存储，各保留最近 50 条，点击可回看与下载
- **检查更新**：基于 GitHub Releases 自动检查新版本，一键跳转下载页
- **问题反馈**：内置反馈入口，直达开发者邮箱

---

## 📦 下载安装

前往 [Releases](https://github.com/xiayuyang750/watermark-tool/releases) 页面下载最新安装包：

- `Watermark Tool_<版本>_x64-setup.exe`（Windows x64，约 5MB）

安装后打开即用：解析区输入链接 → 一键解析 → 预览 / 下载。

> 数据目录：`C:\Users\<你>\ .watermark-tool`（配置、任务记录、下载输出，卸载不影响该目录）

---

## 🛠️ 技术架构

```
┌────────────────────────────────────────────────┐
│ Vue 3 前端（Tauri WebView2 渲染）              │
│   · 解析区 / 媒体预览 / 历史 / 设置             │
└───────────────────────┬────────────────────────┘
                        │  HTTP（127.0.0.1:17890）
┌───────────────────────▼────────────────────────┐
│ Rust 后端（axum，随应用分发 sidecar）            │
│   · 任务队列（rusqlite） · 配置 · 反馈           │
│   · 媒体代理（UA/Referer/Range） · 风控诊断      │
└───┬───────────────────────┬────────────────────┘
    │                       │
    ▼                       ▼
Edge CDP（抖音/登录）      纯 HTTP（X）
· 浏览器常驻复用            · vxtwitter/fxtwitter
· XBogus 签名（Apache-2.0） · tag=29 签名参数
· 固定 UA 规避风控          · GraphQL 拦截（登录态）
```

- **前端**：Vue 3 + Element Plus + Pinia（网页预览 / 桌面共用一套代码）
- **桌面壳**：Tauri 2（WebView2 + NSIS 安装包）
- **后端**：Rust（axum / tokio / reqwest / rusqlite / tokio-tungstenite），API 契约与历史 Python 版完全一致
- **体积对比**：Rust 重构后安装包 **134MB → 5MB**（-96%），解压体积 440MB → 18.3MB

---

## 🚀 开发预览（网页模式）

桌面与网页共用一套代码。本地起 Rust 后端 + Vite 前端即可在浏览器中开发调试：

```bash
# 后端
cd backend-rust
cargo run --release

# 前端（另开终端）
cd desktop
npm install
npm run dev
```

浏览器访问 `http://localhost:5173`（前端经 Vite 代理转发 `/api/v1` 到本地后端）。

---

## 🗂️ 目录结构

```
├─ desktop/                 # Tauri 桌面端（Vue 前端 + 桌面壳）
│  ├─ src/                  #   Vue 3 前端（视图/组件/API/状态）
│  └─ src-tauri/            #   桌面壳（含 NSIS 打包配置）
├─ backend-rust/            # Rust 后端（axum）
│  └─ src/
│     ├─ parsers/           #   解析器（douyin / x / sign / cdp）
│     ├─ tasks/             #   下载任务队列
│     ├─ x_login.rs         #   X 登录引导（Edge CDP）
│     └─ main.rs            #   12 个 API 端点注册
├─ backend/                 # 历史 Python 后端（已退役，备份保留）
├─ CHANGELOG.md             # 更新日志
└─ REFACTOR_PLAN.md         # Rust 重构计划
```

---

## 🔄 更新方式

软件「设置 → 关于 → 检查更新」会请求本仓库的 Releases 获取最新版本：

- 有新版本 → 提示版本号并跳转下载页
- 已是最新 → 绿色提示

也可手动到 [Releases](https://github.com/xiayuyang750/watermark-tool/releases) 查看历史版本与更新说明。

---

## 📄 License / 声明

- 本项目仅用于个人学习与合法素材的整理下载
- 请勿用于任何违反平台规则或法律法规的用途
- 下载内容版权归原作者与平台所有
