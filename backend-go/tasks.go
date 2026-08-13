// 任务管理器：worker goroutine + 队列 + JSON 文件持久化。
// 状态机逻辑与 backend-rust/src/tasks/manager.rs 一致；
// 持久化用 tasks.json（数据量小，避免引入纯 Go SQLite 使体积膨胀 3 倍+）。

package main

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// TaskInner 内存任务态。
type TaskInner struct {
	ID        string                 `json:"id"`
	TaskType  string                 `json:"type"`
	Status    string                 `json:"status"`
	Progress  int                    `json:"progress"`
	Output    string                 `json:"output"`
	Error     string                 `json:"error"`
	CreatedAt string                 `json:"created_at"`
	URL       string                 `json:"url"`
	Options   map[string]interface{} `json:"options"`
	Cancelled bool                   `json:"-"`
}

// TaskItem API 响应（对齐 Rust TaskRead：id/type/status/progress/output/error/created_at）。
type TaskItem struct {
	ID        string  `json:"id"`
	Type      string  `json:"type"`
	Status    string  `json:"status"`
	Progress  int     `json:"progress"`
	Output    *string `json:"output"`
	Error     *string `json:"error"`
	CreatedAt string  `json:"created_at"`
}

type TaskManager struct {
	mu    sync.Mutex
	tasks map[string]*TaskInner
	queue chan string
}

func newTaskManager() *TaskManager {
	m := &TaskManager{
		tasks: make(map[string]*TaskInner),
		queue: make(chan string, 1024),
	}
	m.loadFromDisk()
	go m.run()
	return m
}

func newTaskID() string {
	b := make([]byte, 6)
	_, _ = rand.Read(b)
	return hex.EncodeToString(b)
}

// ---- 外部接口 ----

func (m *TaskManager) create(taskType, url string, options map[string]interface{}) TaskItem {
	tid := newTaskID()
	created := time.Now().Format("2006-01-02 15:04:05")
	info := &TaskInner{
		ID: tid, TaskType: taskType, Status: "pending", Progress: 0,
		CreatedAt: created, URL: url, Options: options,
	}
	m.mu.Lock()
	m.tasks[tid] = info
	m.mu.Unlock()
	m.persist()
	m.queue <- tid
	return m.publicView(tid)
}

func (m *TaskManager) get(tid string) (TaskItem, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	t, ok := m.tasks[tid]
	if !ok {
		return TaskItem{}, false
	}
	return taskItem(t), true
}

func (m *TaskManager) list() []TaskItem {
	m.mu.Lock()
	defer m.mu.Unlock()
	items := make([]*TaskInner, 0, len(m.tasks))
	for _, t := range m.tasks {
		items = append(items, t)
	}
	// 按创建时间倒序
	for i := 0; i < len(items); i++ {
		for j := i + 1; j < len(items); j++ {
			if items[j].CreatedAt > items[i].CreatedAt {
				items[i], items[j] = items[j], items[i]
			}
		}
	}
	out := make([]TaskItem, 0, len(items))
	for _, t := range items {
		out = append(out, taskItem(t))
	}
	return out
}

func (m *TaskManager) cancel(tid string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	t, ok := m.tasks[tid]
	if !ok {
		return false
	}
	if t.Status == "pending" || t.Status == "running" {
		t.Cancelled = true
		return true
	}
	return false
}

func (m *TaskManager) publicView(tid string) TaskItem {
	m.mu.Lock()
	defer m.mu.Unlock()
	if t, ok := m.tasks[tid]; ok {
		return taskItem(t)
	}
	return TaskItem{ID: tid}
}

func taskItem(t *TaskInner) TaskItem {
	var out, err *string
	if t.Output != "" {
		o := t.Output
		out = &o
	}
	if t.Error != "" {
		e := t.Error
		err = &e
	}
	return TaskItem{
		ID: t.ID, Type: t.TaskType, Status: t.Status, Progress: t.Progress,
		Output: out, Error: err, CreatedAt: t.CreatedAt,
	}
}

func (m *TaskManager) update(tid string, status *string, progress *int, output, errMsg *string) {
	m.mu.Lock()
	if t, ok := m.tasks[tid]; ok {
		if status != nil {
			t.Status = *status
		}
		if progress != nil {
			t.Progress = *progress
		}
		if output != nil {
			t.Output = *output
		}
		if errMsg != nil {
			t.Error = *errMsg
		}
	}
	m.mu.Unlock()
	m.persist()
}

func (m *TaskManager) isCancelled(tid string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	if t, ok := m.tasks[tid]; ok {
		return t.Cancelled
	}
	return false
}

// ---- 持久化（tasks.json） ----

func tasksPath() string {
	return filepath.Join(dataDir(), "tasks.json")
}

func (m *TaskManager) persist() {
	m.mu.Lock()
	rows := make([]*TaskInner, 0, len(m.tasks))
	for _, t := range m.tasks {
		rows = append(rows, t)
	}
	m.mu.Unlock()
	_ = os.MkdirAll(dataDir(), 0o755)
	data, _ := json.MarshalIndent(rows, "", "  ")
	_ = os.WriteFile(tasksPath(), data, 0o644)
}

func (m *TaskManager) loadFromDisk() {
	data, err := os.ReadFile(tasksPath())
	if err != nil {
		return
	}
	var rows []*TaskInner
	if json.Unmarshal(data, &rows) != nil {
		return
	}
	for _, t := range rows {
		if t.Status == "pending" || t.Status == "running" {
			t.Status = "failed"
			t.Error = "应用重启，任务中断"
		}
		t.Cancelled = false
		m.tasks[t.ID] = t
	}
}

// ---- worker ----

func (m *TaskManager) run() {
	for tid := range m.queue {
		m.mu.Lock()
		cancelled := false
		if t, ok := m.tasks[tid]; ok {
			cancelled = t.Cancelled
		}
		m.mu.Unlock()
		if !cancelled {
			m.process(tid)
		}
	}
}

func (m *TaskManager) process(tid string) {
	running, progress := "running", 5
	m.update(tid, &running, &progress, nil, nil)
	if err := m.processInner(tid); err != nil {
		cancelled := m.isCancelled(tid)
		if cancelled {
			status, msg := "cancelled", "用户取消"
			m.update(tid, &status, nil, nil, &msg)
		} else {
			status := "failed"
			msg := err.Error()
			m.update(tid, &status, nil, nil, &msg)
		}
	}
}

func (m *TaskManager) processInner(tid string) error {
	m.mu.Lock()
	t := m.tasks[tid]
	url, taskType := t.URL, t.TaskType
	options := t.Options
	m.mu.Unlock()

	cfg := loadConfig()
	outDir := cfg.OutputDir
	if v, ok := options["output_dir"].(string); ok && strings.TrimSpace(v) != "" {
		outDir = v
	}
	if err := os.MkdirAll(outDir, 0o755); err != nil {
		return err
	}

	if taskType == "direct" {
		// 直链下载：url 为素材直链，options 携带 kind/title/image_url
		kind := "video"
		if v, ok := options["kind"].(string); ok {
			kind = v
		}
		title := "untitled"
		if v, ok := options["title"].(string); ok {
			title = v
		}
		imageURL := ""
		if v, ok := options["image_url"].(string); ok {
			imageURL = v
		}
		paths, err := m.download(url, outDir, title, kind, tid, 15, 99, imageURL)
		if err != nil {
			return err
		}
		status, pct := "done", 100
		out := strings.Join(paths, " | ")
		m.update(tid, &status, &pct, &out, nil)
		return nil
	}

	// link 任务：解析后下载全部素材
	platform := detectPlatform(url)
	if platform == "" {
		return fmt.Errorf("无法识别的平台链接")
	}
	removeWM := true
	if v, ok := options["remove_platform_wm"].(bool); ok {
		removeWM = v
	}
	var result *ParseResult
	var err error
	switch platform {
	case "x":
		result, err = parseX(url, removeWM)
	case "douyin":
		result, err = parseDouyin(url, removeWM)
	}
	if err != nil {
		return err
	}
	p15 := 15
	m.update(tid, nil, &p15, nil, nil)

	if len(result.Files) == 0 {
		return fmt.Errorf("解析结果为空")
	}
	total := len(result.Files)
	paths := make([]string, 0)
	for i, f := range result.Files {
		startPct := 15 + (i * 80) / total
		endPct := 15 + ((i + 1) * 80) / total
		name := result.Title
		if total > 1 {
			name = fmt.Sprintf("%s_%d", result.Title, i+1)
		}
		out, err := m.download(f.URL, outDir, name, f.Kind, tid, startPct, endPct, strPtrOrEmpty(f.ImageURL))
		if err != nil {
			return err
		}
		paths = append(paths, out...)
	}
	status, pct := "done", 100
	out := strings.Join(paths, " | ")
	m.update(tid, &status, &pct, &out, nil)
	return nil
}

func strPtrOrEmpty(p *string) string {
	if p == nil {
		return ""
	}
	return *p
}

// download 下载单个素材（Live 图双文件）。进度映射到 [startPct, endPct]。
func (m *TaskManager) download(url, outDir, title, kind, tid string, startPct, endPct int, imageURL string) ([]string, error) {
	if kind == "livephoto" && imageURL != "" {
		// Live 图 = 视频 + 静态照片，缺一不可
		mid := startPct + (endPct-startPct)*2/3
		video, err := m.download(url, outDir, title, "video", tid, startPct, mid, "")
		if err != nil {
			return nil, err
		}
		photo, err := m.download(imageURL, outDir, title, "image", tid, mid, endPct, "")
		if err != nil {
			return nil, err
		}
		return append(video, photo...), nil
	}

	var ext string
	switch kind {
	case "video", "livephoto":
		ext = ".mp4"
	case "gif":
		ext = ".gif"
	default:
		ext = extFromURL(url)
	}
	filename := fmt.Sprintf("%s_%s%s", sanitizeFilename(title), time.Now().Format("20060102150405"), ext)
	outPath := filepath.Join(outDir, filename)

	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", UA)
	req.Header.Set("Referer", refererFor(url))
	client := &http.Client{Timeout: 120 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("下载请求失败：%v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		if resp.StatusCode == http.StatusNotFound {
			return nil, fmt.Errorf("下载失败：素材链接已失效（HTTP 404），请重新解析获取最新链接")
		}
		return nil, fmt.Errorf("下载失败：HTTP %d", resp.StatusCode)
	}
	total := resp.ContentLength

	f, err := os.Create(outPath)
	if err != nil {
		return nil, fmt.Errorf("创建文件失败：%v", err)
	}
	defer f.Close()

	buf := make([]byte, 32*1024)
	var written int64
	for {
		n, rerr := resp.Body.Read(buf)
		if n > 0 {
			if m.isCancelled(tid) {
				return nil, fmt.Errorf("cancelled")
			}
			if _, werr := f.Write(buf[:n]); werr != nil {
				return nil, fmt.Errorf("写入文件失败：%v", werr)
			}
			written += int64(n)
			if total > 0 {
				pct := startPct + int(float64(written)/float64(total)*float64(endPct-startPct))
				if pct < startPct {
					pct = startPct
				}
				if pct > endPct {
					pct = endPct
				}
				m.update(tid, nil, &pct, nil, nil)
			}
		}
		if rerr == io.EOF {
			break
		}
		if rerr != nil {
			return nil, fmt.Errorf("读取数据失败：%v", rerr)
		}
	}
	return []string{outPath}, nil
}

func sanitizeFilename(name string) string {
	replacer := strings.NewReplacer("\\", "_", "/", "_", ":", "_", "*", "_", "?", "_", "\"", "_", "<", "_", ">", "_", "|", "_", "\r", "_", "\n", "_")
	cleaned := strings.TrimSpace(replacer.Replace(name))
	if cleaned == "" {
		cleaned = "untitled"
	}
	// 按字符（rune）截断，避免多字节 UTF-8 被切断产生乱码
	r := []rune(cleaned)
	if len(r) > 60 {
		cleaned = string(r[:60])
	}
	return cleaned
}

func extFromURL(url string) string {
	path := strings.ToLower(strings.SplitN(url, "?", 2)[0])
	for _, e := range []string{".webp", ".png", ".gif", ".jpeg", ".jpg"} {
		if strings.HasSuffix(path, e) {
			return e
		}
	}
	return ".jpg"
}
