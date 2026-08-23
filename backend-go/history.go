// 解析历史持久化（后端存储）。
// 移动端 WebView 的 localStorage 兼容性/持久化不可靠，历史统一存后端 dataDir/history.json。
package main

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"sync"
)

// HistoryItem 一条解析历史记录（与前端结构一致）。
type HistoryItem struct {
	ID     string      `json:"id"`
	Input  string      `json:"input"`
	Time   string      `json:"time"`
	Result ParseResult `json:"result"`
}

const historyMax = 50

type historyStore struct {
	mu    sync.Mutex
	items map[string][]HistoryItem // platform -> 最近 historyMax 条（新在前）
}

var historyS = &historyStore{items: map[string][]HistoryItem{}}

func historyPath() string {
	return filepath.Join(dataDir(), "history.json")
}

func (h *historyStore) loadLocked() {
	data, err := os.ReadFile(historyPath())
	if err != nil {
		return
	}
	_ = json.Unmarshal(data, &h.items)
}

func (h *historyStore) saveLocked() {
	data, _ := json.MarshalIndent(h.items, "", "  ")
	_ = os.MkdirAll(filepath.Dir(historyPath()), 0o755)
	_ = os.WriteFile(historyPath(), data, 0o644)
}

// handleReplaceHistory PUT /api/v1/history {"platform":"x","items":[...]}
// 全量替换该平台历史（清空/单条删除由前端先改数组再整体提交）。
func handleReplaceHistory(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Platform string        `json:"platform"`
		Items    []HistoryItem `json:"items"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil || req.Platform == "" {
		apiError(w, http.StatusBadRequest, "请求体不是合法 JSON")
		return
	}
	historyS.mu.Lock()
	defer historyS.mu.Unlock()
	historyS.loadLocked()
	if req.Items == nil {
		req.Items = []HistoryItem{}
	}
	if len(req.Items) > historyMax {
		req.Items = req.Items[:historyMax]
	}
	historyS.items[req.Platform] = req.Items
	historyS.saveLocked()
	writeJSON(w, http.StatusOK, map[string]interface{}{"items": req.Items})
}

// handleListHistory GET /api/v1/history?platform=x
func handleListHistory(w http.ResponseWriter, r *http.Request) {
	platform := r.URL.Query().Get("platform")
	historyS.mu.Lock()
	defer historyS.mu.Unlock()
	historyS.loadLocked()
	items := historyS.items[platform]
	if items == nil {
		items = []HistoryItem{}
	}
	writeJSON(w, http.StatusOK, map[string]interface{}{"items": items})
}

// handleAddHistory POST /api/v1/history {"platform":"x","item":{...}}
func handleAddHistory(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Platform string      `json:"platform"`
		Item     HistoryItem `json:"item"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil || req.Platform == "" {
		apiError(w, http.StatusBadRequest, "请求体不是合法 JSON")
		return
	}
	historyS.mu.Lock()
	defer historyS.mu.Unlock()
	historyS.loadLocked()
	list := historyS.items[req.Platform]
	list = append([]HistoryItem{req.Item}, list...) // 新记录插前
	if len(list) > historyMax {
		list = list[:historyMax]
	}
	historyS.items[req.Platform] = list
	historyS.saveLocked()
	writeJSON(w, http.StatusOK, map[string]interface{}{"items": list})
}
