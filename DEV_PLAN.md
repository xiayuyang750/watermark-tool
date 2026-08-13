# 三端落地与核心稳固开发计划（DEV_PLAN）

> 版本：v1.0 · 2026-08-11
> 状态：规划稿，待评审后实施
> 适用范围：Watermark Tool 三端（Web/Windows/Android）统一落地 + 核心风险降低
> 前置文档：[README.md](README.md) · [REFACTOR_PLAN.md](REFACTOR_PLAN.md) · [ANTI_RISK_PLAN.md](ANTI_RISK_PLAN.md) · [DEVELOPMENT_GUIDE.md](DEVELOPMENT_GUIDE.md)

---

## 1. 背景与目标

### 1.1 现状回顾

当前产品（v0.3.5）已具备：

- **Windows 桌面端**：Tauri 2 + Vue3 + Go 后端 sidecar，5MB 安装包，抖音/X 解析下载完整可用
- **Web 端**：开发预览形态（本机起 Go 后端 + 浏览器访问 `localhost:5173`），未作为独立产品形态
- **Android 端**：未开始（[docs/G5-安卓端打包实施文档.md](docs/G5-安卓端打包实施文档.md) 仅有规划）

### 1.2 核心矛盾

| 矛盾 | 表现 |
|---|---|
| **Android 无系统 Edge** | chromedp 不可用，抖音 CDP 链路无法直接平移 |
| **抖音纯 HTTP 签名被风控** | a_bogus 静默拒绝（200 空响应），无法作为安卓兜底 |
| **个人开发者无公网部署能力** | 无法采用"远端后端统一"架构，三端必须本地化 |
| **核心强依赖浏览器** | 抖音解析单点依赖 Edge CDP，失效即整体宕机 |

### 1.3 目标

1. **三端可用**：Web、Windows、Android 三端用户均可独立使用（不依赖公网服务器）
2. **核心稳固**：降低抖音解析的单点风险，建立多通道并行 + 诊断闭环
3. **后端语言**：保持 Go（现役，零迁移成本），不迁回 Rust
4. **API 契约一致**：三端共用同一份前端代码 + 同一套后端 API 契约
5. **Android 策略**：PoC 驱动——先验证 WebView CDP 桥（方案C）与移动端纯 HTTP（方案D），失败则回退家庭 PC 模式（方案B）

### 1.4 非目标

- ❌ 公网部署/云端后端（用户明确无法部署上线）
- ❌ 应用商店上架（APK 仅侧载分发）
- ❌ 内容水印去除（M5 可选，本计划不覆盖）
- ❌ 重写前端 UI

---

## 2. 总体架构

### 2.1 三端关系

```
┌─────────────────────────────────────────────────────────────┐
│  Vue3 前端（一套代码，三端共用）                                │
│  · 解析区 / 媒体预览 / 历史 / 设置 / 后端地址配置                │
└────────────────────┬────────────────────────────────────────┘
                     │ HTTP /api/v1（契约不变）
        ┌────────────┼────────────┐
        ▼            ▼            ▼
   ┌─────────┐  ┌─────────┐  ┌─────────────┐
   │ Web 端  │  │Windows端│  │ Android 端  │
   │浏览器访问│  │Tauri壳  │  │ Tauri壳     │
   │本机后端 │  │本地sidecar│ │ PoC决定方案 │
   └────┬────┘  └────┬────┘  └──────┬──────┘
        │            │              │
        ▼            ▼              ▼
   ┌─────────────────────────────────────────┐
   │  Go 后端（同一份代码，三种部署形态）        │
   │  ├─ Web/Windows：本地 sidecar exe        │
   │  └─ Android：gomobile .so 内嵌 或 局域网  │
   └─────────────────────────────────────────┘
        │                       │
        ▼                       ▼
   系统 Edge CDP（抖音）    纯 HTTP（X，三端通用）
```

### 2.2 后端部署形态

| 端 | 后端形态 | 抖音解析 | X 解析 |
|---|---|---|---|
| **Web** | 用户本机 Go exe（开发模式） | 本机 Edge CDP | 纯 HTTP |
| **Windows** | Tauri sidecar（wm-backend.exe） | 系统 Edge CDP | 纯 HTTP |
| **Android** | PoC 决定（见 §4.4） | PoC 验证 | 纯 HTTP（本机） |

### 2.3 后端语言决策

**保持 Go**，理由：

- 现役后端就是 Go（v0.3.x），零迁移成本
- 远端后端模式下 Go 部署简单（单 exe + chromedp）
- gomobile 可编译 Android .so（若 PoC 走方案 C/D 内嵌路线）
- Rust 迁回工作量大且重复（已从 Rust 迁到 Go），仅当确定走纯本机内嵌路线才值得

### 2.4 Android 抖音解析策略（PoC 驱动）

并行验证两条路线，**任一成功则采用，全部失败则回退家庭 PC 模式**：

| 方案 | 路线 | 验证成本 | 成功后收益 | 失败风险 |
|---|---|---|---|---|
| **C** | 自研 Android WebView CDP 桥 | 中（1-2 周 PoC） | Android 完全独立 | WebView 调试端口暴露方案不成熟 |
| **D** | 移动端 UA + 纯 HTTP 签名 | 低（3-5 天 PoC） | Android 完全独立 | a_bogus 已被风控，移动端可能同样被拒 |
| **B（回退）** | 家庭 PC 局域网模式 | 零（仅前端配置项） | 功能完整但依赖 PC | 非"纯本地"，但无需公网 |

> **方案 B 澄清**：家庭 PC 模式 ≠ 部署上线。用户家里 PC 开着 Go 后端，手机连同一 WiFi 下的 PC（如 `192.168.1.5:17890`），无需公网 IP、无需域名、无需部署知识。本质是"局域网内自己用"。

---

## 3. 方向1：核心稳固与降低风险

### 3.1 解析器接口抽象

**目标**：将抖音/X 解析器抽象为统一接口，支持多通道并行尝试与自动选优。

**现状问题**：[backend-go/douyin.go](backend-go/douyin.go) 与 [backend-go/x.go](backend-go/x.go) 是独立函数，无统一接口；抖音单通道（仅 Edge CDP），无降级。

**实施方案**：

```go
// backend-go/parser.go（新增）
type Parser interface {
    Platform() string          // "douyin" | "x"
    Channels() []string        // ["edge_cdp", "x_bogus_http", "mobile_http"]
    Parse(ctx context.Context, url string, removeWm bool) (ParseResult, error)
}

// 多通道并行解析器
type MultiChannelParser struct {
    channels []Channel
    timeout  time.Duration
}

type Channel interface {
    Name() string
    Available() bool            // 环境探测（如 Edge 是否存在）
    Parse(ctx context.Context, url string, removeWm bool) (ParseResult, error)
}
```

**抖音通道清单**（按优先级）：

1. `edge_cdp`（主）：现有 chromedp + 系统 Edge，Windows 可用
2. `mobile_http`（备）：移动端 UA + 纯 HTTP 签名，PoC 验证
3. `webview_cdp`（Android）：自研 WebView 桥，PoC 验证
4. `x_bogus_http`（兜底）：XBogus 签名直连，已知被风控但保留

**验收**：解析时按优先级尝试，主通道失败自动降级，前端无感知；诊断页可看到实际使用通道。

### 3.2 多通道并行解析

**目标**：抖音解析时并行启动 2-3 个通道，取首个成功结果，降低单点延迟与失败率。

**实施方案**：

```go
func (m *MultiChannelParser) Parse(ctx context.Context, url string, removeWm bool) (ParseResult, error) {
    available := filterAvailable(m.channels)
    if len(available) == 0 {
        return ParseResult{}, errors.New("无可用解析通道")
    }
    
    ctx, cancel := context.WithTimeout(ctx, m.timeout)
    defer cancel()
    
    results := make(chan parseOutcome, len(available))
    for _, ch := range available {
        go func(c Channel) {
            r, err := c.Parse(ctx, url, removeWm)
            results <- parseOutcome{channel: c.Name(), result: r, err: err}
        }(ch)
    }
    
    // 取首个成功，失败累积错误
    var errs []string
    for i := 0; i < len(available); i++ {
        select {
        case o := <-results:
            if o.err == nil {
                cancel() // 取消其他通道
                return o.result, nil
            }
            errs = append(errs, o.channel+": "+o.err.Error())
        case <-ctx.Done():
            return ParseResult{}, fmt.Errorf("解析超时：%s", strings.Join(errs, "; "))
        }
    }
    return ParseResult{}, fmt.Errorf("所有通道失败：%s", strings.Join(errs, "; "))
}
```

**约束**：

- 并行度 ≤ 3（避免对平台压力过大）
- 单通道超时 15s，总超时 20s
- 失败时返回所有通道的错误明细（前端可展示）

**验收**：主通道正常时 ≤3s 返回；主通道失败时 ≤10s 自动降级成功；全部失败时返回结构化错误。

### 3.3 Go 版诊断端点补齐

**目标**：将 Python 沙盒中的 D1-D8 八维诊断迁移到 Go 后端，前端重建"风控自检"页。

**现状问题**：[backend-go/main.go:233](backend-go/main.go#L233) `/api/v1/diagnose` 仍是 `handleNotImplemented` 占位。

**实施方案**：

新增 `backend-go/diagnose.go`，实现八维诊断：

| 编号 | 检测项 | Go 实现 |
|---|---|---|
| D1 | 平台连通性 | `http.Get("https://www.douyin.com")` 检查状态码与验证码关键词 |
| D2 | 分享链接解析 | 短链重定向链路追踪 |
| D3 | 浏览器解析（主） | 调用 `parseDouyin` 测试链接 |
| D4 | 纯 HTTP 签名（备） | XBogus 签名调 detail 接口 |
| D5 | CDN 下载 | HEAD 媒体直链 |
| D6 | 登录态（X） | 检查 config.json 中 x_cookie 关键字段 |
| D7 | IP 频率 | 连续 3 次解析请求，统计空响应率 |
| D8 | 本地链路 | 后端 health + 媒体代理自检 |

**响应格式**（与 [ANTI_RISK_PLAN.md](ANTI_RISK_PLAN.md) §3.2 一致）：

```jsonc
{
  "generated_at": "2026-08-11 15:00:00",
  "summary": { "pass": 5, "warn": 2, "fail": 1 },
  "items": [
    { "id": "D3", "name": "浏览器解析", "level": "FAIL", "evidence": {...}, "suggestion": "..." }
  ]
}
```

**前端**：设置抽屉新增"风控自检"入口，点击后展示分级报告。

**验收**：D1-D8 全部可执行；FAIL 项有明确修复建议；连续 3 次端到端解析成功时无 FAIL。

### 3.4 tag 签名自动发现

**目标**：X 视频直链 tag（14/29/12...）从硬编码改为自动提取。

**现状问题**：[backend-go/x.go](backend-go/x.go) 硬编码 `tag=29`，遇到 `tag=14` 视频 404（v0.3.4 已用 HEAD 探测回退修复，但仍非根治）。

**实施方案**：

```go
// 从 GraphQL variants 中自动提取 tag
func extractTagFromVariants(variants []VideoVariant) string {
    for _, v := range variants {
        if u, err := url.Parse(v.URL); err == nil {
            if tag := u.Query().Get("tag"); tag != "" {
                return tag
            }
        }
    }
    return "29" // 兜底
}

// 直链验证回退链（保留现有逻辑作为兜底）
func xVideoURLWithFallback(rawURL string) string {
    candidates := []string{
        rawURL,                                    // 原始（自带 tag）
        addTagParam(rawURL, "29"),                 // 兜底 1
        addTagParam(rawURL, "14"),                 // 兜底 2
        stripTagParam(rawURL),                     // 裸 URL
    }
    for _, c := range candidates {
        if headOK(c) { return c }
    }
    return rawURL
}
```

**验收**：解析时优先使用 GraphQL variants 自带 tag；6 条历史复现链接全部 HTTP 200；新增 tag 类型无需改代码。

### 3.5 任务队列持久化回 SQLite（可选）

**目标**：从 `tasks.json` 回退到 SQLite，提升并发可靠性。

**现状**：v0.3.0 为避免 SQLite 体积膨胀改用 JSON 持久化，单用户场景够用但并发风险存在。

**方案**：保留 tasks.json 作为默认，新增 `WATERMARK_TOOL_DB=sqlite` 环境变量切换。仅当 M1 完成后评估必要性。

---

## 4. 方向3：三端架构落地

### 4.1 三端架构总览

```
┌──────────────────────────────────────────────────────────────┐
│  Vue3 前端（一套代码）                                          │
│  ├─ HomeView.vue（解析/预览/历史）                              │
│  ├─ MediaViewer.vue（媒体展示）                                 │
│  ├─ stores/config.ts（新增：backendURL 运行时可配）              │
│  └─ api/client.ts（API 基址动态读取）                           │
└────────────────────────┬─────────────────────────────────────┘
                         │
        ┌────────────────┼────────────────┐
        ▼                ▼                ▼
   ┌─────────┐      ┌─────────┐      ┌─────────────┐
   │ Web 端  │      │Windows端│      │ Android 端  │
   │Vite dev │      │Tauri 2  │      │ Tauri 2     │
   │localhost│      │sidecar  │      │ Android APK │
   │ :5173   │      │本地Go   │      │             │
   └────┬────┘      └────┬────┘      └──────┬──────┘
        │                │                  │
        ▼                ▼                  ▼
   ┌────────────────────────────────────────────────────┐
   │  Go 后端（同一份代码，三种形态）                       │
   │  Web/Windows: wm-backend.exe（sidecar）              │
   │  Android:    gomobile .so 内嵌 或 局域网连 PC        │
   └────────────────────────────────────────────────────┘
```

### 4.2 Web 端

**形态**：用户本机起 Go 后端 + 浏览器访问 Vite dev server（或打包静态资源后由 Go 后端托管）。

**部署方式**：

- **开发模式**（现状）：`cd backend-go && go run .` + `cd desktop && npm run dev` → 浏览器访问 `http://localhost:5173`
- **生产模式**（新增）：`go build -o wm-backend.exe` + `npm run build` → Go 后端托管 `desktop/dist` 静态资源 → 浏览器访问 `http://localhost:17890`

**前端改动**：

```typescript
// desktop/src/api/client.ts
const API_BASE = import.meta.env.DEV 
  ? '/api/v1'  // Vite 代理
  : `${window.location.origin}/api/v1`  // 生产模式同源
```

**验收**：双击 `wm-backend.exe` → 浏览器访问 `localhost:17890` → 解析/下载/历史全流程通过。

### 4.3 Windows 端

**形态**：现状保持（Tauri 2 sidecar）。

**改动**：仅前端 `stores/config.ts` 新增 `backendURL` 字段（默认 `http://127.0.0.1:17890`），为 Android 端配置项预留。

**验收**：Tauri 打包后安装包 ≤6MB；启动自动拉起 sidecar；生产模式 CORS 通过。

### 4.4 Android 端（PoC 驱动）

#### 4.4.1 PoC 阶段（M3，关键里程碑）

**目标**：以最小成本验证 Android 抖音解析可行性，决定后续路线。

**双路并行验证**：

##### PoC-C：自研 Android WebView CDP 桥

**原理**：Android System WebView 支持 `--remote-debugging-port`，应用内 WebView 加载抖音页面，通过 CDP 捕获页面签名请求。

**验证步骤**：

1. Tauri 2 Android 最小工程初始化（`npm run tauri android init`）
2. 在 Android WebView 中启用调试模式（`WebView.setWebContentsDebuggingEnabled(true)`）
3. `adb forward tcp:9222 localabstract:webview_devtools_remote` 端口转发
4. Go 后端通过 chromedp 连接 `localhost:9222`，复用现有抖音解析逻辑
5. 加载抖音页面，捕获 detail 接口签名请求，重放获取 JSON

**PoC 验收**：真机拿到真实作品 JSON（视频/图集各一）。

**失败判定**：WebView 调试端口无法暴露 / 抖音风控识别 WebView / 签名捕获失败。

##### PoC-D：移动端 UA + 纯 HTTP 签名

**原理**：用移动端 UA 打开抖音移动版页面，尝试（a）页面内抓接口（b）JS 带签名直连 API。

**验证步骤**：

1. Windows 上模拟移动端 UA（Chrome DevTools 移动模式）
2. 访问 `https://m.douyin.com/share/video/{aweme_id}`
3. 抓包分析移动端接口签名规则
4. Go 后端实现移动端 UA + 签名直连

**PoC 验收**：移动端 UA 下纯 HTTP 取回真实 aweme JSON。

**失败判定**：移动端同样要求签名 / 返回空响应 / 跳验证码。

##### PoC 输出

输出《Android 抖音 PoC 结论报告》，结论三选一：

- ✅ **方案 C 可行** → 走纯本机 WebView 桥路线（M4-C）
- ✅ **方案 D 可行** → 走纯本机 HTTP 签名路线（M4-D）
- ❌ **全部失败** → 回退方案 B 家庭 PC 模式（M4-B）

#### 4.4.2 实施阶段（M4，基于 PoC 结论）

##### M4-C/D：Android 纯本机路线（PoC 成功）

**架构**：

```
Android App（Tauri 2 + Vue3 WebView）
        │ JNI
Go 后端（gomobile 编译 .so 内嵌）
   ├─ 抖音：WebView CDP 桥 或 移动端 HTTP
   ├─ X：纯 HTTP（本机）
   └─ 媒体代理 / 任务队列
```

**关键技术点**：

1. **gomobile 编译**：`gomobile bind -target=android -o libwmbackend.aar ./backend-go`
2. **Tauri Android 集成**：通过 Tauri 插件桥接 Go .so 与 Vue 前端
3. **WebView CDP**（仅 M4-C）：Android System WebView 调试桥封装
4. **前端适配**：触摸交互、安全区域、屏幕方向

**APK 体积目标**：≤15MB（不含 Chromium 内核）

##### M4-B：家庭 PC 模式（PoC 失败回退）

**架构**：

```
Android App（Tauri 2 + Vue3 WebView）
        │ HTTP（局域网，可配置后端地址）
家庭 PC 上的 Go 后端（wm-backend.exe）
   ├─ 抖音：Edge CDP（PC 端系统 Edge）
   └─ X：纯 HTTP
```

**前端改动**：

```typescript
// desktop/src/stores/config.ts
const backendURL = ref(localStorage.getItem('backend_url') || 'http://10.0.2.2:17890')
// 10.0.2.2 = Android 模拟器访问宿主机的特殊 IP
// 真机改为局域网 IP，如 192.168.1.5:17890
```

**设置页新增**：后端地址配置项（带连通性测试按钮）。

**使用约束**：Android 使用时家庭 PC 需开机且运行 `wm-backend.exe`。

#### 4.4.3 前端三端适配

**统一改动**：

1. **API 基址动态化**：`api/client.ts` 从 `config.ts` 读取 `backendURL`
2. **平台判断**：`navigator.userAgent` 区分 Android/Desktop，自动调整默认 `backendURL`
3. **触摸优化**：MediaViewer 滑动切换图集（移动端手势）
4. **安全区域**：Android 状态栏/导航栏适配（`safe-area-inset`）
5. **后端连通性提示**：启动时探测 backendURL，失败时引导配置

### 4.5 API 契约统一

**约束**：三端 API 契约完全一致，前端零分支判断。

**新增端点**（为三端适配）：

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/diagnose` | 八维诊断（补齐占位） |
| GET | `/api/v1/backend/info` | 后端能力探测（platforms、channels、edge_available） |

**响应示例**：

```jsonc
// /api/v1/backend/info
{
  "version": "0.4.0",
  "platforms": ["douyin", "x"],
  "channels": {
    "douyin": ["edge_cdp", "mobile_http", "webview_cdp"],
    "x": ["vxtwitter", "fxtwitter", "graphql"]
  },
  "edge_available": true,  // Windows: true, Android: false
  "mode": "local"  // local | lan_client
}
```

---

## 5. 里程碑与验收标准

### M1：核心稳固（方向1）

**预计工作量**：1-2 周

| 子任务 | 验收标准 |
|---|---|
| 3.1 解析器接口抽象 | `Parser`/`Channel` 接口落地，抖音/X 实现迁移 |
| 3.2 多通道并行解析 | 主通道失败时 ≤10s 自动降级成功；前端无感知 |
| 3.3 Go 诊断端点补齐 | D1-D8 全部可执行；设置页"风控自检"可用 |
| 3.4 tag 签名自动发现 | 6 条历史链接全部 HTTP 200；新增 tag 类型无需改代码 |
| 3.5 任务队列（可选） | 评估报告，决定是否回 SQLite |

**整体验收**：真实抖音链接连续 5 次解析成功率 ≥80%（含降级）；真实 X 链接 5 次全部成功；诊断无 FAIL。

### M2：Web/Windows 三端统一验证

**预计工作量**：3-5 天

| 子任务 | 验收标准 |
|---|---|
| Web 生产模式 | Go 后端托管静态资源，浏览器访问 `localhost:17890` 全流程通过 |
| Windows 端回归 | Tauri 打包后安装包 ≤6MB，全流程通过 |
| API 契约一致 | Web/Windows 前端零改动共用 |
| backendURL 配置 | 前端可配置后端地址，带连通性测试 |

**整体验收**：Web 端用户双击 exe + 浏览器访问即可使用；Windows 端无回归。

### M3：Android PoC（关键决策点）

**预计工作量**：1-2 周

| 子任务 | 验收标准 |
|---|---|
| Tauri Android 工程初始化 | `npm run tauri android dev` 真机能跑空壳 |
| PoC-C WebView CDP 桥 | 真机拿到抖音真实作品 JSON（视频/图集各一） |
| PoC-D 移动端 HTTP | 移动端 UA 纯 HTTP 取回真实 aweme JSON |
| PoC 结论报告 | 输出可行/部分可行/不可行 + 推荐路线 |

**整体验收**：输出 PoC 结论报告，明确 M4 路线（C/D/B 三选一）。

**失败预案**：PoC 全部失败 → 直接进入 M4-B 家庭 PC 模式，不阻塞发布。

### M4：Android 实施

**预计工作量**：2-3 周（视 PoC 结论）

| 路线 | 子任务 | 验收 |
|---|---|---|
| **M4-C/D**（PoC 成功） | gomobile 编译 .so、Tauri 集成、WebView CDP 桥/移动HTTP落地、前端触摸适配 | APK ≤15MB，真机抖音+X 解析下载全流程通过 |
| **M4-B**（PoC 失败回退） | 前端 backendURL 配置、局域网连通性测试、家庭 PC 部署文档 | Android 真机连家庭 PC 后全流程通过 |

**整体验收**：Android 真机可安装、可解析、可下载、可回看历史。

### M5：三端发布与运维

**预计工作量**：3-5 天

| 子任务 | 验收标准 |
|---|---|
| 三端版本号同步 | package.json / tauri.conf.json / Cargo.toml / main.go 一致 |
| 发布产物 | Windows setup.exe + Android APK + Web 静态资源包 |
| GitHub Releases | 三端产物上传，更新日志完整 |
| 用户文档 | README 更新三端使用说明；部署运维指南上线 |

**整体验收**：用户按文档可在三端独立安装使用。

---

## 6. 部署运维指南

### 6.1 Windows 端部署

**用户侧**（零配置）：

1. 下载 `Watermark Tool_x.y.x_x64-setup.exe`
2. 双击安装（NSIS 安装包）
3. 开始菜单启动应用

**数据目录**：`C:\Users\<用户>\.watermark-tool\`（配置、任务、输出）

**卸载**：控制面板卸载，数据目录保留（用户手动清理）。

### 6.2 Web 端部署

**用户侧**（本机自托管）：

1. 下载 `wm-backend.exe` + `web-static.zip`
2. 解压 `web-static.zip` 到 `wm-backend.exe` 同目录的 `dist/` 文件夹
3. 双击 `wm-backend.exe`
4. 浏览器访问 `http://localhost:17890`

**Go 后端静态资源托管**（新增）：

```go
// backend-go/main.go 新增
mux.Handle("/", http.FileServer(http.Dir("dist")))
```

**约束**：仅本机访问（绑定 127.0.0.1），不暴露公网。

### 6.3 Android 端部署

#### 6.3.1 M4-C/D 纯本机模式

**用户侧**：

1. 下载 `WatermarkTool_x.y.x.apk`
2. 手机设置 → 安全 → 允许未知来源安装
3. 点击 APK 安装
4. 打开应用，直接使用

**约束**：抖音解析依赖 PoC 验证方案（WebView 桥或移动 HTTP），X 平台纯本机可用。

#### 6.3.2 M4-B 家庭 PC 模式

**用户侧**：

1. PC 端：双击 `wm-backend.exe` 启动后端（保持运行）
2. 手机端：安装 APK
3. 手机与 PC 连同一 WiFi
4. APK 设置 → 后端地址 → 填入 PC 局域网 IP（如 `http://192.168.1.5:17890`）→ 点击测试
5. 测试通过后即可使用

**PC 端 IP 查询**：`ipconfig` → 无线局域网适配器 → IPv4 地址

**约束**：使用时 PC 必须开机且后端运行；手机与 PC 同一局域网。

### 6.4 监控与日志

**日志位置**：

- Windows sidecar：`%TEMP%\watermark-tool\logs\backend.out.log`
- Web/家庭PC模式：`wm-backend.exe` 同目录 `logs\`
- Android：`/data/data/<package>/files/logs/`

**日志内容**：

- 启动/关闭事件
- 解析请求（含通道、耗时、成功/失败）
- 风控诊断结果
- 错误堆栈

**问题反馈**：设置 → 反馈 → SMTP 邮件（带日志附件）

### 6.5 升级机制

**Windows**：设置 → 检查更新 → GitHub Releases 对比版本 → 跳转下载页

**Android**：设置 → 检查更新 → GitHub Releases 对比版本 → 下载 APK → 提示安装

**Web**：重新下载 `wm-backend.exe` + `web-static.zip` 覆盖

**后端兼容性**：API 契约只增不改（向后兼容），旧前端可连新后端。

---

## 7. 风险与应对

| 风险 | 等级 | 应对 |
|---|---|---|
| **PoC 全部失败**（Android 抖音无解） | 高 | 回退方案 B 家庭 PC 模式，不阻塞 Android 发布 |
| **抖音风控升级**（Edge CDP 失效） | 高 | 多通道并行 + 诊断闭环；社区方案跟踪 |
| **gomobile 编译复杂** | 中 | 优先 PoC 验证；备选 Tauri Rust 桥接 |
| **Android WebView 调试桥不稳定** | 中 | PoC 阶段充分验证；失败回退方案 B |
| **APK 体积超标** | 低 | 不打包 Chromium；gomobile 产物精简 |
| **家庭 PC 模式用户体验差** | 中 | 文档明确说明约束；提供 PC 端一键启动脚本 |
| **三端 API 契约漂移** | 低 | 契约测试（M2 起建立） |
| **移动端签名逆向工作量超预期** | 中 | PoC 时间盒（5 天），超时即判失败 |

---

## 8. 附录

### 8.1 关键决策记录

| 日期 | 决策 | 理由 |
|---|---|---|
| 2026-08-11 | 后端保持 Go | 现役零迁移成本；gomobile 支持 Android |
| 2026-08-11 | Android 走 PoC 驱动 | 抖音 Android 解析无现成方案，需先验证 |
| 2026-08-11 | 不采用公网部署 | 用户为个人开发者，无部署能力 |
| 2026-08-11 | 家庭 PC 模式作为兜底 | 局域网内自用，无需公网 IP/域名 |
| 2026-08-11 | 三端共用 Vue 前端 | API 契约一致，零分支判断 |

### 8.2 参考资料

- [README.md](README.md) - 产品说明
- [DESIGN.md](DESIGN.md) - 详细设计（v0.1 草稿，待更新）
- [REFACTOR_PLAN.md](REFACTOR_PLAN.md) - 原生重构计划
- [ANTI_RISK_PLAN.md](ANTI_RISK_PLAN.md) - 风控对抗计划
- [DEVELOPMENT_GUIDE.md](DEVELOPMENT_GUIDE.md) - 开发规范
- [docs/G5-安卓端打包实施文档.md](docs/G5-安卓端打包实施文档.md) - 安卓打包规划
- Tauri 2 Android 文档：https://v2.tauri.app/develop/
- gomobile 文档：https://pkg.go.dev/golang.org/x/mobile/cmd/gomobile

### 8.3 变更记录

| 日期 | 版本 | 变更 |
|---|---|---|
| 2026-08-11 | v1.0 | 初稿：三端统一目标、PoC 驱动 Android 策略、5 个里程碑、部署运维指南 |
