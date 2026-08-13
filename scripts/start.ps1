# start.ps1 - One-click launcher for Watermark Tool (backend + frontend)
# Usage: powershell -ExecutionPolicy Bypass -File scripts\start.ps1 [-Mode browser|tauri]
#
# Modes:
#   browser - web interface, opens the default browser (default when -Mode omitted
#             and user picks 1 in the menu)
#   tauri   - native desktop window (requires Rust toolchain for first compile)
#
# Notes:
# - Data dir defaults to ~/.watermark-tool; falls back to <project>/.data if the
#   user profile is not writable (e.g. sandboxed dev environment).
# - Already-running services on ports 17890/5173 are detected and skipped.
# - Chromium is required once for the douyin browser parser:
#   python -m playwright install chromium

param(
    [ValidateSet("browser", "tauri", "")]
    [string]$Mode = ""
)

$ErrorActionPreference = "Stop"

$Root     = Split-Path -Parent $PSScriptRoot
$Backend  = Join-Path $Root "backend"
$Desktop  = Join-Path $Root "desktop"
$DataDir  = Join-Path $Root ".data"
$LogsDir  = Join-Path $DataDir "logs"
$BackendUrl = "http://127.0.0.1:17890"
$FrontendUrl = "http://localhost:5173"

Write-Host "=== Watermark Tool Launcher ===" -ForegroundColor Cyan

# ---------- 0. launch mode selection ----------
if (-not $Mode) {
    Write-Host ""
    Write-Host "Select launch mode:" -ForegroundColor Yellow
    Write-Host "  1) Browser mode    - web interface in your default browser"
    Write-Host "  2) Tauri mode      - native desktop window"
    $choice = Read-Host "Enter 1 or 2"
    $Mode = if ($choice -eq "2") { "tauri" } else { "browser" }
    Write-Host ""
}
Write-Host "[mode] $Mode"

# ---------- 1. data dir (fallback if profile not writable) ----------
$profileDir = Join-Path $env:USERPROFILE ".watermark-tool"
$env:WATERMARK_TOOL_HOME = $profileDir
try {
    New-Item -ItemType Directory -Force -Path $profileDir | Out-Null
    $t = Join-Path $profileDir ".write_test"
    Set-Content -Path $t -Value "t" -ErrorAction Stop
    Remove-Item -Path $t
    $dataHome = $profileDir
} catch {
    $env:WATERMARK_TOOL_HOME = $DataDir
    $dataHome = $DataDir
    Write-Host "[warn] User profile not writable, using project data dir: $DataDir" -ForegroundColor Yellow
}
New-Item -ItemType Directory -Force -Path $LogsDir | Out-Null
Write-Host "[data] $dataHome"

# ---------- 2. dependency check ----------
function Test-Cmd([string]$name) { return [bool](Get-Command $name -ErrorAction SilentlyContinue) }

if (-not (Test-Cmd python)) { Write-Error "Python not found. Install Python 3.11+ first."; exit 1 }
if (-not (Test-Cmd node))    { Write-Error "Node.js not found. Install Node 18+ first."; exit 1 }

& python -c "import fastapi, uvicorn, httpx, playwright" 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Host "[setup] Installing backend dependencies..." -ForegroundColor DarkYellow
    & python -m pip install -r (Join-Path $Backend "requirements.txt")
    if ($LASTEXITCODE -ne 0) { Write-Error "Backend dependency install failed."; exit 1 }
}

if (-not (Test-Path (Join-Path $Desktop "node_modules"))) {
    Write-Host "[setup] Installing frontend dependencies..." -ForegroundColor DarkYellow
    Push-Location $Desktop
    & npm install
    Pop-Location
    if ($LASTEXITCODE -ne 0) { Write-Error "Frontend dependency install failed."; exit 1 }
}

# ---------- 3. port check ----------
function Test-Port([int]$port) {
    try {
        return [bool](Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)
    } catch { return $false }
}

# ---------- 4. start backend ----------
if (Test-Port 17890) {
    Write-Host "[backend] already running on 17890, skip" -ForegroundColor Green
} else {
    Write-Host "[backend] starting..."
    Start-Process -FilePath "python" -ArgumentList "run.py" `
        -WorkingDirectory $Backend `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $LogsDir "backend.out.log") `
        -RedirectStandardError  (Join-Path $LogsDir "backend.err.log") | Out-Null
}

# ---------- 5. start frontend ----------
# Browser mode: start vite directly. Tauri mode: vite is started by `tauri dev`
# itself (beforeDevCommand), so only skip when the port is already in use.
if ($Mode -eq "browser") {
    if (Test-Port 5173) {
        Write-Host "[frontend] already running on 5173, skip" -ForegroundColor Green
    } else {
        Write-Host "[frontend] starting..."
        Start-Process -FilePath "npm.cmd" -ArgumentList "run", "dev" `
            -WorkingDirectory $Desktop `
            -WindowStyle Hidden `
            -RedirectStandardOutput (Join-Path $LogsDir "frontend.out.log") `
            -RedirectStandardError  (Join-Path $LogsDir "frontend.err.log") | Out-Null
    }
}

# ---------- 6. wait until ready ----------
function Wait-Ready([string]$url, [string]$what, [int]$timeoutSec = 90) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 3 -ErrorAction Stop
            if ($r.StatusCode -eq 200) {
                Write-Host "[$what] ready: $url" -ForegroundColor Green
                return $true
            }
        } catch { }
        Start-Sleep -Seconds 2
    }
    Write-Host "[$what] NOT ready after $timeoutSec s: $url" -ForegroundColor Red
    return $false
}

Wait-Ready "$BackendUrl/api/v1/health" "backend"

# ---------- 7. launch frontend entry per mode ----------
if ($Mode -eq "browser") {
    Wait-Ready $FrontendUrl "frontend"
    Start-Process $FrontendUrl | Out-Null
    Write-Host "[browser] opened $FrontendUrl" -ForegroundColor Green
} else {
    Write-Host "[tauri] starting desktop app (first compile may take a few minutes)..."
    Start-Process -FilePath "npm.cmd" -ArgumentList "run", "tauri", "dev" `
        -WorkingDirectory $Desktop `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $LogsDir "tauri.out.log") `
        -RedirectStandardError  (Join-Path $LogsDir "tauri.err.log") | Out-Null
    $deadline = (Get-Date).AddSeconds(600)
    $seen = $false
    while ((Get-Date) -lt $deadline) {
        if (Get-Process app -ErrorAction SilentlyContinue) {
            $seen = $true
            break
        }
        Start-Sleep -Seconds 3
    }
    if ($seen) {
        Write-Host "[tauri] desktop window started" -ForegroundColor Green
    } else {
        Write-Host "[tauri] window not detected within 600s, check logs: tauri.err.log" -ForegroundColor Red
    }
}

Write-Host "=== Done. Logs: $LogsDir ===" -ForegroundColor Cyan
