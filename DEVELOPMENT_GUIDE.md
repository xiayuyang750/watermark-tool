# 开发规范（DEVELOPMENT_GUIDE）

> 用途：沉淀「Rust 重构后」的开发决策与踩坑经验，供后续维护、二次开发对照执行。
> 原则：**核心逻辑用 Rust，对抗分析用 Python 验证沙盒**——各用所长，把 Rust 的开发摩擦降到最低。

---

## 1. 技术栈决策（2026-08 定稿）

- **产品形态**：Vue3 前端 + Rust 后端（桌面 sidecar，未来安卓编译为 `.so`）
- **为什么保留 Rust**：体积（5MB vs 134MB）+ 跨平台（Windows/安卓共享核心）
- **为什么保留 Python 源码**（`backend/app`）：作为**对抗分析验证沙盒**，不删除

> 反向约束：不因"Rust 是最终产物"就把所有探索都直接写进 Rust——先验证、后移植。

---

## 2. 平台解析 / 反爬对抗工作流（标准流程）

```
① 分析（语言无关）
   web-reverse-engineering 方法论：抓包 → 定位接口 → 还原签名 → 反混淆
② Python 快速验证（backend/app 环境）
   写临时脚本验证新接口/新签名是否可行（分钟级）
③ Rust 落地（backend-rust）
   移植进 src/parsers，跑 cargo test + 真实链接解析验证
④ 打包发布
   cargo build --release → 复制 exe 到 backend/dist/watermark-backend/
   → desktop 下 npm run tauri build → gh release create vX.Y.Z
```

**关键**：第②步不被跳过。签名/接口类改动一律先 Python 验证，避免在 Rust 里试错一次等 10 秒编译。

---

## 3. Rust 常见坑速查（重构期踩过的）

| 坑 | 正确做法 |
|---|---|
| 阻塞调用（`reqwest::blocking`、`std::thread::sleep`）出现在 async 上下文 → panic | 用 async 版；轮询等待用 `tokio::time::sleep` |
| `std::sync::Mutex` 的 guard 跨 `await` → 不 Send | 跨 await 的共享状态用 `tokio::sync::Mutex` |
| `Cell` 跨线程 → 不 Sync | 用 `Atomic*` 或 `Mutex` |
| CDP `Network.setUserAgentOverride` 跨连接不持久，抖音识别 headless | UA 用启动参数 `--user-agent={UA}` 固定 |
| `Page.addScriptToEvaluateOnNewDocument` 会话级，连接断开失效 | 监听响应改用 `Network.enable` + `responseReceived` 事件 |
| `x_login_profile` 同档案多实例 → 新实例转发后立即退出 | 启动前 `taskkill /T /F` 进程树清理 + 重试 3 次 |
| 登录轮询误判"窗口关闭" | 双确认：CDP 端口连不上 **且** 进程已退出 |
| cookie 注入含 `partitionKey` 等私有字段 → 整体被拒 | `sanitize_cookies` 过滤为标准字段 |
| GraphQL 结构兼容 | 同时支持 `media_details` 与 `legacy.extended_entities` |

---

## 4. 环境与沙箱（本机开发）

| 问题 | 解法 |
|---|---|
| TRAE 沙箱拦截写 `C:\Users\21494\.watermark-tool` | `WATERMARK_TOOL_HOME` 指向项目内 `.data` |
| TRAE 沙箱拦截 gh CLI 写配置 | `GH_CONFIG_DIR` 指向项目内 `.data\ghcli` |
| 火绒 HIPS 拦后端写目录 | 信任区加入 `C:\Users\21494\.watermark-tool`，改后重启后端进程 |

---

## 5. 发布流程（对照清单）

1. 版本号同步改 **4 处**：`desktop/package.json`、`desktop/src-tauri/tauri.conf.json`、`desktop/src-tauri/Cargo.toml`、`backend-rust/src/main.rs`（health 里的 version）
2. 更新 `CHANGELOG.md`
3. `cargo build --release`（backend-rust）→ 复制 exe 到 `backend/dist/watermark-backend/`
4. `npm run tauri build`（desktop）
5. 发布：

```powershell
gh release create vX.Y.Z "安装包绝对路径" --repo xiayuyang750/watermark-tool --title "vX.Y.Z" --notes "更新说明"
```

6. 用户端「设置 → 关于 → 检查更新」即可发现新版本

---

## 6. 备份与退役

- `backend/dist/备份watermark-backend-py-0.2.2-bak`、`watermark-backend-x86_64-pc-windows-msvc`（各 442MB，Python 打包产物）**暂不清理**（用户要求）
- `backend/app` Python 源码**保留**（验证沙盒 + 回退参考）
- `backend/build` PyInstaller 中间产物（可清理，已无用）
