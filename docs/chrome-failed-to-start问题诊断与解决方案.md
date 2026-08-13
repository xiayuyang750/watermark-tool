# 「chrome failed to start」问题诊断与解决方案报告

> 适用产品：Watermark Tool（抖音 / X 解析下载工具）
> 报告日期：2026-08-09
> 涉及版本：v0.3.1 → v0.3.5
> 问题等级：**高**（反复出现，网页端 + 桌面端均不可用）

---

## 1. 问题概述

**现象**：抖音解析返回「打开抖音页面失败：chrome failed to start:」，网页端与桌面端均无法解析抖音内容。

**历史**：该问题在 v0.3.1、v0.3.2、v0.3.3、v0.3.4、v0.3.5 期间**反复出现**，每次修复后间隔一段又复发，属于"改完能用、用几次又坏"的顽固问题。

**影响面**：抖音全链路（解析 / 播放 / 下载）不可用；X 解析不受影响（纯 HTTP）。

---

## 2. 架构背景（理解问题的前提）

产品有两个客户端入口，**共用 17890 端口但后端完全不同**：

| 入口 | 启动方式 | 后端 | 抖音解析浏览器 |
|---|---|---|---|
| 网页端 | `scripts\start.ps1`（浏览器模式） | **Python 版**（`python run.py`，uvicorn） | Playwright + 自带 Chromium |
| 桌面端 | `watermark-tool.exe`（Tauri sidecar） | **Go 版**（`wm-backend.exe`） | chromedp + 系统 Edge |

**关键点**：两套后端抢同一个端口 17890。桌面端启动时会检测端口占用，**强制关闭旧后端再拉起自己的 Go 后端**（见 [desktop/src-tauri/src/lib.rs](desktop/src-tauri/src/lib.rs) `request_backend_shutdown`）。

---

## 3. 历史根因排查（v0.3.1 → v0.3.3）

### 3.1 残留进程占用档案（v0.3.1 定位）
- **现象**：解析偶发失败，重启后恢复
- **根因**：崩溃/强杀后 Edge 档案目录残留 `Singleton*` 锁文件与半死进程
- **修复**：启动前清理残留进程（`taskkill /T /F` 进程树）+ 删除档案锁 + 失败重试 3 次 + 换新档案目录自愈

### 3.2 安全软件按文件名拦截（v0.3.3 定位，对照实验证实）
- **现象**：`watermark-backend.exe` 拉起的 Edge 子进程全部启动即退（`Lock file Error 5`）
- **定位方法**：同一份代码，**只改文件名**（`wm-backend.exe`）后 Edge 正常启动 → 排除代码逻辑，锁定"安全软件按文件身份拦截"
- **修复**：后端 exe 改名为 `wm-backend.exe`

### 3.3 用户数据目录下 Edge 启动被拒（v0.3.4 定位）
- **现象**：v0.3.4 打包后再次全量失败
- **定位方法（对照实验逐项排除）**：

| 实验 | 结果 |
|---|---|
| 同份代码 + 项目内数据目录（`.data\wmtest`） | ✅ 成功 |
| 同份代码 + 用户数据目录（`C:\Users\21494\.watermark-tool`） | ❌ 失败（`Lock file Error 5`） |
| 系统临时目录（`%TEMP%`） | ✅ 成功 |
| 独立 Go 复现程序（相同 chromedp 参数，任意目录） | ✅ 8/8 成功 |

- **排除项**：chromedp 版本、启动参数、进程名、工作目录、目录 ACL、锁文件残留、安全软件进程、Defender 受控文件夹、点前缀目录、清空目录重建
- **结论**：用户数据目录路径下 Edge headless 启动不稳定（创建 ProcessSingleton 锁被拒），`%TEMP%` 与项目内目录稳定
- **修复（v0.3.5）**：Edge 运行时档案从数据目录迁移到 `os.TempDir()\watermark-tool-cdp`（[backend-go/browser.go](backend-go/browser.go)），数据/配置/登录态留在原目录

---

## 4. 本次复发根因分析（v0.3.5，网页端先成功 → 桌面端 → 全挂）

### 4.1 时间线（由日志确认）

| 时间 | 事件 |
|---|---|
| 17:53 | 网页端（Python 后端）解析 2 个抖音视频成功 + 下载（`.data\logs\backend.out.log` 完整 200 记录） |
| 17:55:03/08 | 用户打开桌面端（出现了两个 `watermark-tool.exe` 实例） |
| 17:55:10 | 桌面端拉起 Go 后端（PID 18420），监听 17890 |
| 17:55:06 | Python 后端被关闭，**Playwright driver 报 `EPIPE: broken pipe`**（浏览器管道断裂，Chromium 残留未完全清理） |
| 17:56-17:58 | Go 后端解析反复失败 `chrome failed to start`（`%TEMP%` 下出现 3 个换档目录：`watermark-tool-cdp-1786269378/448/511`） |
| 之后 | Go 后端退出，17890 空闲 |

### 4.2 根因结论

**「Python 后端被强制关闭 → Go 后端接管 17890」的过渡瞬间，Edge 启动被干扰。**

具体机制：
1. 桌面端检测到 17890 被 Python 后端占用 → 发送 `/api/v1/shutdown` 强制关闭（[lib.rs L40-48](desktop/src-tauri/src/lib.rs) 只等待 3 秒端口释放）
2. Python 后端关闭时 Playwright driver 报 EPIPE，**Chromium 残留未完全清理**（Python 的 `shutdown()` 用 `os._exit(0)`，0.5 秒后强杀进程，未等待浏览器完全关闭）
3. Go 后端在资源过渡态启动 Edge → chromedp 连接失败 → `chrome failed to start`（空错误）
4. 网页端此后连的是已损坏的 Go 后端 → 同样失败

**补充**：用户当时可能开了两个桌面端实例，加剧 sidecar 生命周期竞争（次要因素）。

### 4.3 已确认的环境状态（排查结束时）

- **当前环境完全正常**：同一份 Go 后端 + 用户数据目录 + 17890 → 解析成功（4 张图）
- 残留的 13 个 chrome 进程为用户日常 Chrome，非干扰源
- 无 Playwright Chromium 残留

**即：本次失败是一次性"接管时序"事件，日常单独使用桌面端（端口空闲）不会触发。**

---

## 5. 解决方案

### 5.1 已实施

| 版本 | 方案 |
|---|---|
| v0.3.1 | 残留进程清理 + 档案锁删除 + 3 次重试 + 换档案自愈 |
| v0.3.3 | 后端 exe 改名 `wm-backend.exe` 规避安全软件身份拦截 |
| v0.3.5 | Edge 运行时档案迁移至 `%TEMP%\watermark-tool-cdp` |

### 5.2 建议实施（按优先级）

**P0 — Go 后端日志文件化**（本次排查最大的痛点是"无日志"）
- 目标：`[dy]` 调试输出 + Edge stderr 落盘到 `%TEMP%\watermark-tool.log`
- 收益：再次出现 `chrome failed to start` 时，可直接看到 `Lock file Error 5` / 连接失败细节，不再抓瞎
- 实现：`browser.go` 中 `println` 改为写日志文件（含时间戳 + 轮转）

**P1 — 桌面端接管加固**
- 关闭旧后端后，除等待端口释放外，**再等待旧浏览器（Playwright Chromium）残留进程完全退出**（轮询 `chrome.exe`/`msedge.exe` 直到无匹配进程，或主动 `taskkill`）
- 等待时间从固定 3 秒改为"轮询 + 上限"

**P2 — 桌面端单实例限制**
- 引入 Tauri 单实例插件，防止用户双开导致 sidecar 竞争

**P3 — dyKillStaleEdge 增强**
- 杀进程后由固定 800ms 改为**轮询确认全部退出**（含 crashpad/renderer 子进程）+ 锁文件删除重试，避免残留 Edge 占用档案

---

## 6. 验证方法

| 场景 | 验证步骤 | 预期 |
|---|---|---|
| 抖音解析 | 解析图集 / 动图 / 视频各一次 | 全部成功 |
| 常驻复用 | 连续解析 2 条 | 第 2 条 < 2 秒（复用浏览器） |
| X 解析 | 解析视频 + 图片推文 | 成功，直链 HEAD 200 |
| 网页端 + 桌面端共存 | 网页端（Python 后端）解析成功后打开桌面端 | 桌面端接管后解析正常（P1 生效后） |
| 日志可观测 | 触发一次失败，查看 `%TEMP%\watermark-tool.log` | 包含 `[dy]` 与 Edge stderr 详情 |

---

## 7. 预防措施与经验沉淀

1. **多后端抢同一端口是架构隐患**：网页端（Python）与桌面端（Go）并存时，端口接管逻辑必须完备（等端口 + 等浏览器残留清理）
2. **"无日志"是最贵的排查成本**：凡涉及浏览器自动化（chromedp / Playwright），必须把子进程 stderr 落盘
3. **对照实验方法论有效**：本次通过"只改文件名 / 只改数据目录"等单一变量对照，多次快速定位根因（详见 `.trae/skills/web-reverse-engineering/references/cdp-browser-automation.md`）
4. **安全软件按文件身份拦截**：改名后需关注安装路径/新编译产物是否触发新的信誉评估

---

## 8. 遗留事项

- [ ] 实施 P0 日志文件化
- [ ] 实施 P1 接管加固
- [ ] 实施 P2 单实例限制（需确认用户是否双开）
- [ ] 重新打包（当前 0.3.5 安装包不含 P0-P2）
- [ ] 用户侧：删除 `C:\Users\21494\.watermark-tool_empty`（排查残留空目录）
