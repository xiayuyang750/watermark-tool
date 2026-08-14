// CDP 浏览器管理器（chromedp 驱动系统 Edge）。
// 对齐 backend-rust/src/parsers/cdp.rs 的常驻复用逻辑：
// - 抖音解析用固定 profile + 常驻页面，空闲 IDLE_TTL 自动关闭
// - UA 通过启动参数固定（规避 headless 特征被平台识别）

package main

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	gort "runtime"
	"strings"
	"sync"
	"time"

	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/chromedp"
)

const idleTTL = 10 * time.Minute
const dyHome = "https://www.douyin.com/"

// edgePath 查找系统 Edge 可执行文件。
func edgePath() (string, error) {
	for _, cand := range []string{
		`C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`,
		`C:\Program Files\Microsoft\Edge\Application\msedge.exe`,
	} {
		if _, err := os.Stat(cand); err == nil {
			return cand, nil
		}
	}
	return "", fmt.Errorf("未找到 Edge 浏览器，无法执行解析")
}

// edgeAllocator 用 chromedp 创建 Edge 实例分配器。
// 注意：不引入 chromedp.DefaultExecAllocatorOptions 全量默认参数——
// 实测其完整参数组合会被安全软件（火绒 HIPS）拦截导致 Edge 启动即退出（exit 21 / Lock file Error 5），
// 改用最小参数集（含反检测 UA + 自动化特征隐藏）即可正常工作。
func edgeAllocator(edge, profile string, headless bool) (context.Context, context.CancelFunc) {
	opts := []chromedp.ExecAllocatorOption{
		chromedp.ExecPath(edge),
		chromedp.UserDataDir(profile),
		chromedp.UserAgent(UA),
		chromedp.Flag("no-first-run", true),
		chromedp.Flag("no-default-browser-check", true),
		chromedp.Flag("disable-blink-features", "AutomationControlled"),
		chromedp.Flag("disable-features", "AutomationControlled"),
		// 跳过 ProcessSingleton 锁文件（部分安全软件拦截锁文件创建）
		chromedp.Flag("no-singleton", true),
		chromedp.Flag("mute-audio", true),
	}
	if headless {
		opts = append(opts, chromedp.Flag("headless", "new"))
	}
	return chromedp.NewExecAllocator(context.Background(), opts...)
}

// ---- 抖音常驻浏览器 ----

var dyBrowser struct {
	sync.Mutex
	ctx       context.Context
	cancel    context.CancelFunc
	lastUsed  time.Time
}

// dyKillStaleEdge 结束所有命令行含 watermark-tool-cdp 的 msedge 进程（进程树 + 二次确认，
// 防止残留未完全退出占用档案导致新实例启动即失败）。
// 注意：仅 Windows 需要（安卓无 msedge / powershell，直接跳过）。
func dyKillStaleEdge() {
	if gort.GOOS != "windows" {
		return
	}
	script := `
$ps = Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'watermark-tool-cdp' }
foreach ($p in $ps) { taskkill /PID $p.ProcessId /T /F 2>$null | Out-Null }
Start-Sleep -Milliseconds 800
$ps = Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -match 'watermark-tool-cdp' }
foreach ($p in $ps) { taskkill /PID $p.ProcessId /T /F 2>$null | Out-Null }
`
	_ = exec.Command("powershell", "-NoProfile", "-Command", script).Run()
}

// dyRemoveProfileLocks 删除 Edge 档案锁文件（崩溃残留的 Singleton* 锁会让新实例拒绝启动）。
func dyRemoveProfileLocks(profile string) {
	for _, f := range []string{"SingletonLock", "SingletonCookie", "SingletonSocket"} {
		_ = os.Remove(filepath.Join(profile, f))
	}
}

// dyEnsurePage 确保抖音常驻页面就绪；失效或空闲超时自动重建。
func dyEnsurePage() error {
	dyBrowser.Lock()
	defer dyBrowser.Unlock()

	// 已有页面且存活且未空闲超时 → 复用
	if dyBrowser.ctx != nil && time.Since(dyBrowser.lastUsed) < idleTTL {
		if dyPageAlive(dyBrowser.ctx) {
			dyBrowser.lastUsed = time.Now()
			return nil
		}
		// 页面失效：重建
		dyTeardownLocked()
	} else {
		dyTeardownLocked()
	}

	// 启动前清理残留 Edge 进程 + 档案锁文件，避免档案被占用导致新实例启动即失败
	dyKillStaleEdge()
	edge, err := edgePath()
	if err != nil {
		return err
	}
	// Edge 运行时档案放系统临时目录（实测：部分机器用户数据目录下 Edge headless
	// 启动会报 Lock file Error 5 / chrome failed to start，临时目录稳定可用）
	baseProfile := filepath.Join(os.TempDir(), "watermark-tool-cdp")
	profile := baseProfile
	_ = os.MkdirAll(profile, 0o755)
	dyRemoveProfileLocks(profile)

	// 启动失败（残留未完全退出/档案被锁）时清理后重试；固定档案仍失败则换新档案目录自愈
	for attempt := 0; attempt < 3; attempt++ {
		allocCtx, cancelAlloc := edgeAllocator(edge, profile, true)
		ctx, cancel := chromedp.NewContext(allocCtx)
		dyBrowser.ctx = ctx
		dyBrowser.cancel = func() { cancel(); cancelAlloc() }

		// 打开抖音首页，等安全 SDK 注入（首次解析前置）
		// 注意：不可用带超时的子 ctx 包裹 chromedp.Run——实测取消子 ctx 会连坐浏览器
		// context（browser.LostConnection → allocator cancel），导致常驻页面后续全部失效
		err := chromedp.Run(ctx, chromedp.Navigate(dyHome))
		if err != nil {
			println(fmt.Sprintf("[dy] attempt=%d profile=%s err=%v", attempt, profile, err))
			dyTeardownLocked()
			if attempt < 2 {
				dyKillStaleEdge()
				dyRemoveProfileLocks(profile)
				if attempt >= 1 {
					// 固定档案被锁/损坏（目录删不掉、锁文件清不掉）时换全新目录绕开
					profile = filepath.Join(os.TempDir(), fmt.Sprintf("watermark-tool-cdp-%d", time.Now().Unix()))
					_ = os.MkdirAll(profile, 0o755)
				}
				time.Sleep(1200 * time.Millisecond)
				continue
			}
			return fmt.Errorf("打开抖音页面失败：%v", err)
		}
		time.Sleep(2500 * time.Millisecond)
		dyBrowser.lastUsed = time.Now()
		return nil
	}
	return fmt.Errorf("打开抖音页面失败")
}

// dyPageAlive 页面是否存活（轻量 JS 探测）。
func dyPageAlive(ctx context.Context) bool {
	var ready string
	err := chromedp.Run(ctx, chromedp.Evaluate("document.readyState", &ready, func(p *runtime.EvaluateParams) *runtime.EvaluateParams {
		return p.WithReturnByValue(true)
	}))
	return err == nil && (ready == "complete" || ready == "interactive" || ready == "loading")
}

// dyTeardownLocked 关闭常驻浏览器（须已持有 dyBrowser.Mutex）。
func dyTeardownLocked() {
	if dyBrowser.cancel != nil {
		dyBrowser.cancel()
	}
	dyBrowser.ctx = nil
	dyBrowser.cancel = nil
}

// dyTeardown 关闭常驻浏览器（应用退出时调用）。
func dyTeardown() {
	dyBrowser.Lock()
	defer dyBrowser.Unlock()
	dyTeardownLocked()
}

// dyFetchDetail 捕获页面自身发出的 detail 请求（带 a_bogus 签名），复用其签名 URL 直连拿数据。
// 抖音新版风控：detail 接口强制 a_bogus 签名（手动 XHR / 内嵌数据均不可行），
// 详情页自身渲染时会发出带签名的 detail 请求，捕获该请求 URL 后由 Go 立即重放获取响应。
func dyFetchDetail(apiURL string) (string, error) {
	awemeID := awemeIDFromAPIURL(apiURL)
	if awemeID == "" {
		return "", fmt.Errorf("detail URL 缺少 aweme_id")
	}
	pageURL := "https://www.douyin.com/video/" + awemeID

	// 捕获页面自身发出的 detail 请求（回调只记录 URL，不做 CDP 调用避免阻塞命令通道）
	sigCh := make(chan string, 1)
	chromedp.ListenTarget(dyBrowser.ctx, func(ev interface{}) {
		if e, ok := ev.(*network.EventRequestWillBeSent); ok {
			if strings.Contains(e.Request.URL, "aweme/v1/web/aweme/detail") {
				select {
				case sigCh <- e.Request.URL:
				default:
				}
			}
		}
	})

	if err := chromedp.Run(dyBrowser.ctx, chromedp.Navigate(pageURL)); err != nil {
		return "", fmt.Errorf("打开作品页失败：%v", err)
	}

	var signedURL string
	select {
	case signedURL = <-sigCh:
	case <-time.After(15 * time.Second):
		return "", fmt.Errorf("作品页未请求 detail 接口（可能触发风控或需登录）")
	}

	// 取页面 cookies（ttwid 等），直连时携带保持会话一致
	var cookies []*network.Cookie
	_ = chromedp.Run(dyBrowser.ctx, chromedp.ActionFunc(func(ctx context.Context) error {
		var err error
		cookies, err = network.GetCookies().Do(ctx)
		return err
	}))
	cookieParts := make([]string, 0, len(cookies))
	for _, c := range cookies {
		cookieParts = append(cookieParts, c.Name+"="+c.Value)
	}
	cookieHeader := strings.Join(cookieParts, "; ")

	// 用带签名的 URL 立即重放（签名有时效，越快越好）
	req, err := http.NewRequest(http.MethodGet, signedURL, nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("User-Agent", UA)
	req.Header.Set("Referer", "https://www.douyin.com/")
	if cookieHeader != "" {
		req.Header.Set("Cookie", cookieHeader)
	}
	client := &http.Client{Timeout: 20 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return "", fmt.Errorf("detail 请求失败：%v", err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 10<<20))
	if err != nil {
		return "", err
	}
	b := string(body)
	if !strings.Contains(b, "aweme_detail") {
		return "", fmt.Errorf("detail 接口返回异常（HTTP %d，可能签名已过期）", resp.StatusCode)
	}
	return b, nil
}

// awemeIDFromAPIURL 从 detail API URL 提取 aweme_id。
func awemeIDFromAPIURL(apiURL string) string {
	if m := regexp.MustCompile(`aweme_id=(\d+)`).FindStringSubmatch(apiURL); m != nil {
		return m[1]
	}
	return ""
}

// dyFetchDetailRetry detail 接口重试（详情页加载抖动/风控时重新导航再试，对齐 Rust 3 次）。
func dyFetchDetailRetry(apiURL string) (string, error) {
	for i := 0; i < 3; i++ {
		text, err := dyFetchDetail(apiURL)
		if err == nil {
			return text, nil
		}
		if i < 2 {
			time.Sleep(1500 * time.Millisecond)
		}
	}
	return "", fmt.Errorf("抖音接口返回空数据（可能需要登录抖音）")
}
