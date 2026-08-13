// X 登录引导（chromedp 有头 Edge 窗口，对齐 backend-rust/src/x_login.rs）。
//
// 流程：启动系统 Edge（固定登录档案 x_login_profile）打开 X 登录页
// → 用户在窗口扫码/账号登录 → 后台轮询捕获 auth_token cookie
// → 保存到 config.json（x_cookie，JSON 数组）并关闭浏览器。
// 固定档案持久化登录态，后续登录墙解析直接复用。

package main

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/chromedp"
)

const loginURL = "https://x.com/i/flow/login"
const loginTimeout = 5 * time.Minute

// X 登录流程全局状态：idle / running / done / error
var xLoginState struct {
	sync.Mutex
	status string
	err    string
}

func xLoginSet(status, err string) {
	xLoginState.Lock()
	defer xLoginState.Unlock()
	xLoginState.status = status
	xLoginState.err = err
}

// startXLogin 启动登录引导：弹出 Edge 窗口并后台轮询登录结果。
func startXLogin() map[string]interface{} {
	xLoginState.Lock()
	if xLoginState.status == "running" {
		xLoginState.Unlock()
		return map[string]interface{}{"ok": false, "error": "已有登录流程正在进行中"}
	}
	xLoginState.Unlock()

	// 关闭上次残留的登录实例（同一固定档案），避免档案被占用导致启动失败
	killStaleLoginEdge()
	time.Sleep(1500 * time.Millisecond)

	edge, err := edgePath()
	if err != nil {
		xLoginSet("error", err.Error())
		return map[string]interface{}{"ok": false, "error": "未找到 Edge 浏览器，请改用设置中手动粘贴 Cookie"}
	}
	profile := filepath.Join(dataDir(), "x_login_profile")
	if err := os.MkdirAll(profile, 0o755); err != nil {
		xLoginSet("error", err.Error())
		return map[string]interface{}{"ok": false, "error": "创建登录数据目录失败"}
	}

	allocCtx, cancelAlloc := edgeAllocator(edge, profile, false) // 有头模式：用户需在窗口中登录
	ctx, cancel := chromedp.NewContext(allocCtx)

	xLoginSet("running", "")
	go func() {
		defer func() { cancel(); cancelAlloc() }()
		if err := chromedp.Run(ctx, chromedp.Navigate(loginURL)); err != nil {
			xLoginSet("error", "打开登录页失败，请重试")
			return
		}
		deadline := time.Now().Add(loginTimeout)
		for time.Now().Before(deadline) {
			cookies, err := getAllCookies(ctx)
			if err == nil && hasAuthToken(cookies) {
				data, _ := json.Marshal(cookies)
				cfg := loadConfig()
				cfg.XCookie = string(data)
				if err := saveConfig(&cfg); err != nil {
					xLoginSet("error", "保存登录信息失败")
				} else {
					xLoginSet("done", "")
				}
				return
			}
			time.Sleep(2 * time.Second)
		}
		xLoginSet("error", "登录超时，请重试")
	}()
	return map[string]interface{}{"ok": true, "message": "请在 Edge 窗口中完成 X 登录（建议用手机 X App 扫码，比输入账号更稳定）"}
}

// xLoginStatus 查询登录流程状态。
func xLoginStatus() map[string]interface{} {
	xLoginState.Lock()
	defer xLoginState.Unlock()
	return map[string]interface{}{
		"ok":     xLoginState.status == "done",
		"status": xLoginState.status,
		"error":  xLoginState.err,
	}
}

// getAllCookies 获取浏览器全部 cookie（等价 Playwright ctx.cookies()）。
func getAllCookies(ctx context.Context) ([]*network.Cookie, error) {
	var cookies []*network.Cookie
	err := chromedp.Run(ctx, chromedp.ActionFunc(func(ctx context.Context) error {
		var err error
		cookies, err = network.GetCookies().Do(ctx)
		return err
	}))
	return cookies, err
}

// hasAuthToken 是否出现 auth_token（登录成功标志）。
func hasAuthToken(cookies []*network.Cookie) bool {
	for _, c := range cookies {
		if c.Name == "auth_token" && c.Value != "" {
			return true
		}
	}
	return false
}

// killStaleLoginEdge 结束所有命令行含 x_login_profile 的 msedge 进程（进程树 + 二次确认）。
func killStaleLoginEdge() {
	script := `
$ps = Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'x_login_profile' }
foreach ($p in $ps) { taskkill /PID $p.ProcessId /T /F 2>$null | Out-Null }
Start-Sleep -Milliseconds 800
$ps = Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'x_login_profile' }
foreach ($p in $ps) { taskkill /PID $p.ProcessId /T /F 2>$null | Out-Null }
`
	_ = exec.Command("powershell", "-NoProfile", "-Command", script).Run()
}
