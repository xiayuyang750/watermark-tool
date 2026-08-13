// 应用配置：工作目录、配置文件读写、默认值。
// 逻辑与 backend-rust/src/config.rs 保持一致（换语言不换逻辑）。

package main

import (
	"encoding/json"
	"os"
	"path/filepath"
)

// Config 对应 config.json 的已知字段；未知字段（如 douyin_cookie）由 updateConfig 整体合并保留。
type Config struct {
	RemovePlatformWm bool   `json:"remove_platform_wm"`
	RemoveContentWm  bool   `json:"remove_content_wm"`
	OutputDir        string `json:"output_dir"`
	BackendPort      int    `json:"backend_port"`
	SMTPHost         string `json:"smtp_host"`
	SMTPPort         int    `json:"smtp_port"`
	SMTPUser         string `json:"smtp_user"`
	SMTPAuthCode     string `json:"smtp_auth_code"`
	FeedbackTo       string `json:"feedback_to"`
	// X 平台登录 Cookie（登录墙推文解析用）；敏感字段，不回传前端
	XCookie string `json:"x_cookie"`
}

// dataDir 数据目录：默认用户目录；可用 WATERMARK_TOOL_HOME 覆盖（同 Python/Rust 版回退逻辑）。
func dataDir() string {
	if h := os.Getenv("WATERMARK_TOOL_HOME"); h != "" {
		return h
	}
	base := os.Getenv("USERPROFILE")
	if base == "" {
		base = os.Getenv("HOME")
	}
	if base == "" {
		base = "."
	}
	return filepath.Join(base, ".watermark-tool")
}

func configPath() string {
	return filepath.Join(dataDir(), "config.json")
}

// ensureDirs 确保数据目录下的子目录存在（output/tmp/models，同 Python 版 ensure_dirs）。
func ensureDirs() {
	for _, d := range []string{"output", "tmp", "models"} {
		_ = os.MkdirAll(filepath.Join(dataDir(), d), 0o755)
	}
}

// defaultConfig 返回默认配置（output_dir 指向数据目录下 output）。
func defaultConfig() Config {
	return Config{
		RemovePlatformWm: true,
		OutputDir:        filepath.Join(dataDir(), "output"),
		BackendPort:      17890,
		SMTPHost:         "smtp.qq.com",
		SMTPPort:         465,
	}
}

// loadConfig 加载配置：文件存在则读取（缺失字段用默认值），否则写入默认配置。
func loadConfig() Config {
	ensureDirs()
	_ = os.MkdirAll(filepath.Dir(configPath()), 0o755)
	if _, err := os.Stat(configPath()); err == nil {
		if data, err := os.ReadFile(configPath()); err == nil {
			var cfg Config
			if json.Unmarshal(data, &cfg) == nil {
				// 数值零值回退默认（与 Rust serde default 对齐）
				if cfg.BackendPort == 0 {
					cfg.BackendPort = 17890
				}
				if cfg.SMTPPort == 0 {
					cfg.SMTPPort = 465
				}
				return cfg
			}
		}
	}
	cfg := defaultConfig()
	_ = saveConfig(&cfg)
	return cfg
}

func saveConfig(cfg *Config) error {
	_ = os.MkdirAll(filepath.Dir(configPath()), 0o755)
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(configPath(), data, 0o644)
}

// updateConfig 覆盖更新配置：整体 JSON 合并（保留未知字段，同 Python load_config() | cfg）。
// 返回合并后的完整配置 JSON。
func updateConfig(body map[string]interface{}) map[string]interface{} {
	cfg := readRawOrDefault()
	for k, v := range body {
		cfg[k] = v
	}
	data, _ := json.MarshalIndent(cfg, "", "  ")
	if err := os.WriteFile(configPath(), data, 0o644); err != nil {
		println("[config] 写入配置失败:", err.Error())
	}
	return cfg
}

// readRawOrDefault 读取 config.json 原始 JSON；不存在或损坏时返回默认配置 JSON。
func readRawOrDefault() map[string]interface{} {
	ensureDirs()
	_ = os.MkdirAll(filepath.Dir(configPath()), 0o755)
	if data, err := os.ReadFile(configPath()); err == nil {
		var m map[string]interface{}
		if json.Unmarshal(data, &m) == nil {
			return m
		}
	}
	cfg := defaultConfig()
	_ = saveConfig(&cfg)
	out, _ := json.Marshal(cfg)
	var m map[string]interface{}
	_ = json.Unmarshal(out, &m)
	return m
}

// publicConfig 回传前端的配置（过滤敏感字段：SMTP 授权码、X 登录 Cookie 等）。
func publicConfig(cfg *Config) map[string]interface{} {
	return map[string]interface{}{
		"remove_platform_wm": cfg.RemovePlatformWm,
		"remove_content_wm":  cfg.RemoveContentWm,
		"output_dir":         cfg.OutputDir,
		"backend_port":       cfg.BackendPort,
	}
}
