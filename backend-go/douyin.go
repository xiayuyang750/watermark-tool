// 抖音解析器（对齐 backend-rust/src/parsers/douyin.rs 的纯 HTTP 部分）。
//
// 链路：分享文本 → 提取 URL → 短链重定向得作品 ID → （G3）CDP 浏览器页面内 XHR 调
// `aweme/v1/web/aweme/detail` 接口 → 按内容类型返回原生素材：视频 / 图集 / 动图 / Live 图。
//
// 平台水印去除：视频播放地址 playwm→play；Live 图剥离 watermark/logo_name 参数。

package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"regexp"
	"strings"
	"time"
)

var (
	dyVideoRe  = regexp.MustCompile(`/(video|note)/(\d+)`)
	dyDetailRe = regexp.MustCompile(`/aweme/detail/(\d+)`)
	dyRawIDRe  = regexp.MustCompile(`\b(\d{15,21})\b`)
	dyURLRe    = regexp.MustCompile(`https?://\S+`)
)

// parseDouyin 抖音解析入口：短链解析 → 常驻浏览器页面内 XHR detail → 内容分类。
func parseDouyin(shareText string, removePlatformWm bool) (*ParseResult, error) {
	awemeID, _, err := resolveAwemeID(shareText)
	if err != nil {
		return nil, err
	}
	if err := dyEnsurePage(); err != nil {
		return nil, err
	}
	text, err := dyFetchDetailRetry(buildAPIURL(awemeID))
	if err != nil {
		return nil, err
	}
	var detail map[string]interface{}
	if err := json.Unmarshal([]byte(text), &detail); err != nil {
		return nil, fmt.Errorf("抖音接口返回非 JSON 数据（可能触发风控）")
	}
	aweme, _ := detail["aweme_detail"].(map[string]interface{})
	if aweme == nil {
		return nil, fmt.Errorf("抖音接口未返回作品信息：可能未登录或作品已删除")
	}
	return parseAweme(aweme, removePlatformWm)
}

// extractURL 从分享文本中提取第一条链接（对齐 Rust extract_url）。
func extractURL(text string) (string, error) {
	m := dyURLRe.FindString(text)
	if m == "" {
		return "", fmt.Errorf("未在输入中找到可用的链接")
	}
	return strings.TrimRight(m, "，。；;."), nil
}

// resolveAwemeID 短链重定向解析作品 ID，返回 (aweme_id, 是否图文笔记 note)。
func resolveAwemeID(shareText string) (string, bool, error) {
	url, err := extractURL(shareText)
	if err != nil {
		return "", false, err
	}
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return "", false, fmt.Errorf("短链请求失败：%v", err)
	}
	req.Header.Set("User-Agent", UA)
	req.Header.Set("Referer", "https://www.douyin.com/")
	client := &http.Client{
		Timeout:   20 * time.Second,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return nil // 跟随重定向
		},
	}
	resp, err := client.Do(req)
	if err != nil {
		return "", false, fmt.Errorf("短链请求失败：%v", err)
	}
	defer resp.Body.Close()
	finalURL := resp.Request.URL.String()

	if m := dyVideoRe.FindStringSubmatch(finalURL); m != nil {
		return m[2], m[1] == "note", nil
	}
	if m := dyDetailRe.FindStringSubmatch(finalURL); m != nil {
		return m[1], false, nil
	}
	if m := dyRawIDRe.FindStringSubmatch(finalURL); m != nil {
		return m[1], false, nil
	}
	return "", false, fmt.Errorf("无法从链接中提取作品 ID，请确认是抖音分享链接")
}

// BASE_PARAMS 与 Python/Rust 版保持一致的 Web 接口基础参数。
var baseParams = [][2]string{
	{"device_platform", "webapp"},
	{"aid", "6383"},
	{"channel", "channel_pc_web"},
	{"pc_client_type", "1"},
	{"version_code", "190500"},
	{"version_name", "19.5.0"},
	{"cookie_enabled", "true"},
	{"browser_language", "zh-CN"},
	{"browser_platform", "Win32"},
	{"browser_name", "Edge"},
	{"browser_online", "true"},
	{"engine_name", "Blink"},
	{"os_name", "Windows"},
	{"os_version", "10"},
	{"platform", "PC"},
	{"screen_width", "1920"},
	{"screen_height", "1080"},
}

const apiDetail = "https://www.douyin.com/aweme/v1/web/aweme/detail/"

// buildAPIURL 构造 detail 接口 URL（G3 供 CDP 页面内 XHR 使用）。
func buildAPIURL(awemeID string) string {
	var sb strings.Builder
	for i, p := range baseParams {
		if i > 0 {
			sb.WriteString("&")
		}
		sb.WriteString(p[0])
		sb.WriteString("=")
		sb.WriteString(p[1])
	}
	sb.WriteString("&aweme_id=")
	sb.WriteString(awemeID)
	return apiDetail + "?" + sb.String()
}

// parseAweme 内容分类：图集 > 视频（对齐 Rust parse_aweme）。
func parseAweme(aweme map[string]interface{}, removePlatformWm bool) (*ParseResult, error) {
	title, _ := aweme["desc"].(string)
	title = strings.TrimSpace(title)
	if title == "" {
		title = "未命名作品"
	}

	if imgs, ok := aweme["images"].([]interface{}); ok && len(imgs) > 0 {
		files := make([]MediaFile, 0, len(imgs))
		for _, img := range imgs {
			if m, ok := img.(map[string]interface{}); ok {
				if f := imageFile(m, removePlatformWm); f != nil {
					files = append(files, *f)
				}
			}
		}
		if len(files) == 0 {
			return nil, fmt.Errorf("图集作品未包含图片地址")
		}
		return &ParseResult{Platform: "douyin", Title: title, MediaType: "image", Files: files}, nil
	}

	video, _ := aweme["video"].(map[string]interface{})
	playAddr, _ := video["play_addr"].(map[string]interface{})
	urls, _ := playAddr["url_list"].([]interface{})
	if len(urls) == 0 {
		return nil, fmt.Errorf("视频作品未包含播放地址")
	}
	url, _ := urls[0].(string)
	url = strings.ReplaceAll(url, "http://", "https://")
	if removePlatformWm {
		url = strings.ReplaceAll(url, "playwm", "play")
	}
	var cover *string
	if c, ok := video["cover"].(map[string]interface{}); ok {
		if cl, ok := c["url_list"].([]interface{}); ok && len(cl) > 0 {
			if s, ok := cl[0].(string); ok && s != "" {
				s = strings.ReplaceAll(s, "http://", "https://")
				cover = &s
			}
		}
	}
	return &ParseResult{
		Platform:  "douyin",
		Title:     title,
		MediaType: "video",
		Files:     []MediaFile{{Kind: "video", URL: url, Cover: cover}},
	}, nil
}

// imageFile 单张图片：Live 图 > 动图 > 静态图（对齐 Rust image_file）。
func imageFile(img map[string]interface{}, removePlatformWm bool) *MediaFile {
	// Live 图：带 video 字段（图片会动）→ 静态照片 + 3 秒视频双文件
	if video, ok := img["video"].(map[string]interface{}); ok {
		if playAddr, ok := video["play_addr"].(map[string]interface{}); ok {
			if urls, ok := playAddr["url_list"].([]interface{}); ok && len(urls) > 0 {
				if url, ok := urls[0].(string); ok {
					url = strings.ReplaceAll(url, "http://", "https://")
					if removePlatformWm {
						url = strings.ReplaceAll(url, "playwm", "play")
					}
					var cover *string
					if c, ok := video["cover"].(map[string]interface{}); ok {
						if cl, ok := c["url_list"].([]interface{}); ok && len(cl) > 0 {
							if s, ok := cl[0].(string); ok && s != "" {
								s = strings.ReplaceAll(s, "http://", "https://")
								cover = &s
							}
						}
					}
					// 静态照片直链：Live 图由「照片 + 视频」组成，缺一不可
					var photo *string
					if pl, ok := img["url_list"].([]interface{}); ok && len(pl) > 0 {
						if s, ok := pl[0].(string); ok && s != "" {
							s = strings.ReplaceAll(s, "http://", "https://")
							photo = &s
						}
					}
					if photo != nil {
						return &MediaFile{Kind: "livephoto", URL: url, Cover: cover, ImageURL: photo}
					}
				}
			}
		}
	}
	// 动图：animated/gif 字段
	for _, field := range []string{"animated_url_list", "gif_url_list", "animated_url", "gif_url"} {
		if v, ok := img[field]; ok {
			var u string
			if s, ok := v.(string); ok {
				u = s
			} else if arr, ok := v.([]interface{}); ok && len(arr) > 0 {
				u, _ = arr[0].(string)
			}
			if u != "" {
				return &MediaFile{Kind: "gif", URL: u}
			}
		}
	}
	// 静态图
	if urls, ok := img["url_list"].([]interface{}); ok && len(urls) > 0 {
		if url, ok := urls[0].(string); ok {
			return &MediaFile{Kind: "image", URL: strings.ReplaceAll(url, "http://", "https://")}
		}
	}
	return nil
}
