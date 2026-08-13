// Watermark Tool Go 后端（G1 骨架，替代 Rust/Python sidecar）。
// 仅绑定 localhost（本地桌面工具后端）。逻辑与 backend-rust/src/main.rs 对齐。

package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"
)

const version = "0.3.6"

// UA 与 Python/Rust 版保持一致（避免平台指纹变化）
const UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"

func apiError(w http.ResponseWriter, status int, detail string) {
	writeJSON(w, status, map[string]interface{}{"detail": detail})
}

func writeJSON(w http.ResponseWriter, status int, v interface{}) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

// corsMiddleware 放行 Tauri 生产模式 WebView2 来源（http://tauri.localhost）。
func corsMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		origin := r.Header.Get("Origin")
		if origin == "http://tauri.localhost" || origin == "tauri://localhost" {
			w.Header().Set("Access-Control-Allow-Origin", origin)
			w.Header().Set("Vary", "Origin")
		}
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "*")
		if r.Method == http.MethodOptions {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func handleHealth(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, map[string]interface{}{
		"status":    "ok",
		"version":   version,
		"platforms": []string{"douyin", "x"},
	})
}

// detectPlatform 按链接特征识别平台（对齐 Rust detect_platform）。
func detectPlatform(url string) string {
	u := strings.ToLower(url)
	if strings.Contains(u, "douyin.com") || strings.Contains(u, "iesdouyin") || strings.Contains(u, "v.douyin") {
		return "douyin"
	}
	if strings.Contains(u, "x.com") || strings.Contains(u, "twitter.com") {
		return "x"
	}
	return ""
}

func handleParse(w http.ResponseWriter, r *http.Request) {
	var req ParseRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		apiError(w, http.StatusBadRequest, "请求体不是合法 JSON")
		return
	}
	platform := detectPlatform(req.URL)
	if platform == "" {
		apiError(w, http.StatusBadRequest, "无法识别的平台链接")
		return
	}
	switch platform {
	case "x":
		result, err := parseX(req.URL, req.RemovePlatformWm)
		if err != nil {
			apiError(w, http.StatusUnprocessableEntity, err.Error())
			return
		}
		writeJSON(w, http.StatusOK, result)
	case "douyin":
		result, err := parseDouyin(req.URL, req.RemovePlatformWm)
		if err != nil {
			apiError(w, http.StatusUnprocessableEntity, err.Error())
			return
		}
		writeJSON(w, http.StatusOK, result)
	}
}

func handleGetConfig(w http.ResponseWriter, r *http.Request) {
	// SMTP 授权码、X 登录 Cookie 等敏感字段不回传前端
	cfg := loadConfig()
	writeJSON(w, http.StatusOK, publicConfig(&cfg))
}

func handlePutConfig(w http.ResponseWriter, r *http.Request) {
	var body map[string]interface{}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		apiError(w, http.StatusBadRequest, "请求体不是合法 JSON")
		return
	}
	writeJSON(w, http.StatusOK, updateConfig(body))
}

func handleFeedback(w http.ResponseWriter, r *http.Request) {
	var req FeedbackRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		apiError(w, http.StatusBadRequest, "请求体不是合法 JSON")
		return
	}
	if strings.TrimSpace(req.Content) == "" {
		apiError(w, http.StatusBadRequest, "反馈内容不能为空")
		return
	}
	cfg := loadConfig()
	if err := sendFeedbackEmail(&cfg, req.Content, req.Contact); err != nil {
		apiError(w, http.StatusBadRequest, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]interface{}{"ok": true})
}

// handleNotImplemented 尚未迁移的端点占位（G2 起逐步替换）。
func handleNotImplemented(feature string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		apiError(w, http.StatusNotImplemented, feature+"（Go 迁移阶段实现中）")
	}
}

// handleShutdown 优雅关闭后端。
func handleShutdown(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, map[string]interface{}{"ok": true})
	go func() {
		time.Sleep(500 * time.Millisecond)
		dyTeardown() // 关闭常驻浏览器，避免残留 Chromium 进程
		os.Exit(0)
	}()
}

func handleXLoginStart(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, startXLogin())
}

func handleXLoginStatus(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, xLoginStatus())
}

// ---- 任务端点 ----

type taskCreateReq struct {
	Type    string                 `json:"type"`
	URL     *string                `json:"url"`
	Options map[string]interface{} `json:"options"`
}

var taskManager = newTaskManager()

func handleCreateTask(w http.ResponseWriter, r *http.Request) {
	var req taskCreateReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		apiError(w, http.StatusBadRequest, "请求体不是合法 JSON")
		return
	}
	if req.Type != "link" && req.Type != "direct" {
		apiError(w, http.StatusBadRequest, fmt.Sprintf("任务类型 %s 尚未支持", req.Type))
		return
	}
	url := ""
	if req.URL != nil {
		url = *req.URL
	}
	if url == "" {
		apiError(w, http.StatusBadRequest, "缺少 url")
		return
	}
	if req.Options == nil {
		req.Options = map[string]interface{}{}
	}
	writeJSON(w, http.StatusOK, taskManager.create(req.Type, url, req.Options))
}

func handleListTasks(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, taskManager.list())
}

func handleGetTask(w http.ResponseWriter, r *http.Request) {
	tid := r.PathValue("id")
	item, ok := taskManager.get(tid)
	if !ok {
		apiError(w, http.StatusNotFound, "任务不存在")
		return
	}
	writeJSON(w, http.StatusOK, item)
}

func handleCancelTask(w http.ResponseWriter, r *http.Request) {
	tid := r.PathValue("id")
	if taskManager.cancel(tid) {
		writeJSON(w, http.StatusOK, map[string]interface{}{"ok": true})
		return
	}
	apiError(w, http.StatusBadRequest, "任务不存在或不可取消")
}

func main() {
	cfg := loadConfig()
	port := cfg.BackendPort

	// 启动即清理历史残留的 CDP Edge 进程（异常退出/升级残留会占用档案，导致新实例启动即失败）
	dyKillStaleEdge()

	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/v1/health", handleHealth)
	mux.HandleFunc("GET /api/v1/media", handleMedia)
	mux.HandleFunc("GET /api/v1/config", handleGetConfig)
	mux.HandleFunc("PUT /api/v1/config", handlePutConfig)
	mux.HandleFunc("POST /api/v1/feedback", handleFeedback)
	mux.HandleFunc("POST /api/v1/shutdown", handleShutdown)
	mux.HandleFunc("POST /api/v1/parse", handleParse)
	mux.HandleFunc("POST /api/v1/tasks", handleCreateTask)
	mux.HandleFunc("GET /api/v1/tasks", handleListTasks)
	mux.HandleFunc("GET /api/v1/tasks/{id}", handleGetTask)
	mux.HandleFunc("POST /api/v1/tasks/{id}/cancel", handleCancelTask)
	mux.HandleFunc("GET /api/v1/diagnose", handleNotImplemented("诊断"))
	mux.HandleFunc("POST /api/v1/x/login/start", handleXLoginStart)
	mux.HandleFunc("GET /api/v1/x/login/status", handleXLoginStatus)

	addr := "127.0.0.1:" + strconv.Itoa(port)
	log.Printf("Watermark Tool Go 后端已启动: http://%s", addr)
	if err := http.ListenAndServe(addr, corsMiddleware(mux)); err != nil {
		log.Fatalf("服务异常退出: %v", err)
	}
}
