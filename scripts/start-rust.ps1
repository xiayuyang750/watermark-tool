# start-rust.ps1 —— 启动 Rust(axum) 后端 + 网页前端（浏览器预览模式）
# 用法: powershell -ExecutionPolicy Bypass -File scripts\start-rust.ps1

$ErrorActionPreference = "Stop"

$Root       = Split-Path -Parent $PSScriptRoot
$BackendExe = Join-Path $Root "backend-rust\target\debug\watermark-backend-rs.exe"
$Desktop    = Join-Path $Root "desktop"
$DataDir    = Join-Path $Root ".data"
$LogsDir    = Join-Path $DataDir "logs"

# 数据目录统一使用项目内 .data（开发/自测隔离，不污染用户正式数据目录）
$env:WATERMARK_TOOL_HOME = $DataDir
New-Item -ItemType Directory -Force -Path $LogsDir | Out-Null

$BackendUrl  = "http://127.0.0.1:17890"
$FrontendUrl = "http://localhost:5173"

function Test-Port([int]$port) {
    try { return [bool](Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) }
    catch { return $false }
}
function Wait-Ready([string]$url, [string]$what, [int]$timeoutSec = 90) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
            if ($r.StatusCode -eq 200) { Write-Host "[$what] ready: $url" -ForegroundColor Green; return $true }
        } catch { }
        Start-Sleep -Seconds 2
    }
    Write-Host "[$what] NOT ready after $timeoutSec s: $url" -ForegroundColor Red
    return $false
}

Write-Host "=== Watermark Tool 启动器（Rust 后端） ===" -ForegroundColor Cyan

# 后端可执行文件检查
if (-not (Test-Path $BackendExe)) {
    Write-Error "未找到 $BackendExe，请先在 backend-rust 目录执行 cargo build"; exit 1
}

# 启动后端
if (Test-Port 17890) {
    Write-Host "[backend] 17890 已在运行，跳过" -ForegroundColor Green
} else {
    Write-Host "[backend] 启动 Rust 后端..."
    Start-Process -FilePath $BackendExe `
        -WorkingDirectory (Join-Path $Root "backend-rust") -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $LogsDir "backend-rs.out.log") `
        -RedirectStandardError  (Join-Path $LogsDir "backend-rs.err.log") | Out-Null
}

# 前端依赖
if (-not (Test-Path (Join-Path $Desktop "node_modules"))) {
    Write-Host "[setup] 安装前端依赖..." -ForegroundColor DarkYellow
    Push-Location $Desktop; & npm install; Pop-Location
}

# 启动前端
if (Test-Port 5173) {
    Write-Host "[frontend] 5173 已在运行，跳过" -ForegroundColor Green
} else {
    Write-Host "[frontend] 启动 Vite..."
    Start-Process -FilePath "npm.cmd" -ArgumentList "run", "dev" `
        -WorkingDirectory $Desktop -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $LogsDir "frontend.out.log") `
        -RedirectStandardError  (Join-Path $LogsDir "frontend.err.log") | Out-Null
}

Wait-Ready "$BackendUrl/api/v1/health" "backend"
Wait-Ready $FrontendUrl "frontend"
Start-Process $FrontendUrl | Out-Null
Write-Host "[browser] 已打开 $FrontendUrl" -ForegroundColor Green
Write-Host "=== 完成，日志目录: $LogsDir ===" -ForegroundColor Cyan
