# douyin_tool — 纯客户端安卓多平台下载器

> 本项目是在 `Video Analysis Tool` 大仓库下**新开的小文件夹**，独立于 `desktop`（Tauri 桌面端）与 `backend-*` 后端。
> 这是一套面向**安卓手机**的**纯客户端、无后端**下载方案：解析核心在安卓端用纯 HTTP 完成，不依赖桌面版 Edge/浏览器内核。

## 目录结构

```
douyin_tool/
├── 抖音下载器.apk          # 安卓安装包（带全部修复的最新版）
├── douyin-android/         # 安卓工程（Kotlin + WebView 纯客户端）
│   ├── app/src/main/       # 源码：MainActivity.kt + assets(前端)
│   ├── build.gradle        # 安卓构建配置
│   └── gradlew.bat         # 构建脚本（./gradlew assembleDebug）
└── douyin-web/             # 网页版（Python + Playwright 早期方案，供参考）
```

## 与主项目其他版本的区别

| 维度 | `douyin_tool` 安卓版 | `desktop`（Tauri 桌面版 / 其安卓 APK） |
|------|--------------------|----------------------------------------|
| 载体 | Android App（WebView UI） | Windows 桌面（Tauri 2）/ 其安卓包 |
| 解析核心 | 安卓端纯 HTTP（`HttpURLConnection`） | Go 后端 + Edge CDP（浏览器内核） |
| 依赖 | 无后端、无浏览器内核，安装包仅 **5.4MB** | 嵌入 Go 后端，桌面版约 8MB、安卓版约 150MB |
| 抖音解析 | 走 App feed 接口（免签名） | detail 接口需 a_bogus 签名（Edge 捕获） |
| X 解析 | fxtwitter / vxtwitter 公共 API（纯 HTTP） | 同公共 API + 登录墙兜底（Edge CDP） |
| 历史存储 | 本地文件 `filesDir/history.json`，按平台分桶 | 浏览器本地存储，按平台独立 |

**一句话**：`douyin_tool` 是"轻量纯客户端版"，把解析链路从桌面版的"浏览器内核"换成了"安卓端纯 HTTP"，所以安装包小、无运行时依赖；代价是部分平台（如抖音图集/动图、X 登录墙内容）能力弱于桌面版。

## 功能

- 支持平台：**抖音 / X（Twitter）**（后续可接入小红书、B站，路由已预留）
- 输入链接自动路由：短链跟随跳转提取作品 ID → 纯 HTTP 解析 → 无水印直链
- 解析历史：按平台分桶、作品 ID 去重、就地展开回看、完整链接展示
- 媒体播放：本地 HTTP 代理流式转发（带 UA/Referer，绕开 CDN 防盗链与 file:// Referer 403）
- 一键粘贴 / 一键清除输入框（规避 WebView textarea 选中删除后失灵）
- 下载到手机相册（`Movies/DouyinDownloader/`）

## 构建

```powershell
cd douyin-android
.\gradlew.bat assembleDebug
# 产物：app\build\outputs\apk\debug\app-debug.apk
```

## ⚠️ 说明

- ⚠️ 本工具仅供个人合法素材使用，请尊重原创与平台规则。
- X 视频源（`video.twimg.com`）在部分网络（如未开启代理）下不可达，此时解析可成功但播放/下载会提示网络问题，请开启代理或更换网络。
