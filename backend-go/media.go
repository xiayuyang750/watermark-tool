// 媒体代理：绕过 CDN 防盗链，带 UA/Referer 流式转发，供前端直接播放/显示。
// 透传浏览器 Range 头并转发上游 206 分段响应，保证视频可拖拽、时长正常显示。

package main

import (
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

// refererFor 按 CDN 域名选择合法 Referer（对齐 backend-rust _referer_for）。
func refererFor(url string) string {
	if strings.Contains(url, "twimg.com") {
		return "https://x.com/"
	}
	return "https://www.douyin.com/"
}

func handleMedia(w http.ResponseWriter, r *http.Request) {
	raw := r.URL.Query().Get("url")
	if raw == "" {
		apiError(w, http.StatusBadRequest, "缺少 url")
		return
	}
	if !strings.HasPrefix(raw, "http://") && !strings.HasPrefix(raw, "https://") {
		apiError(w, http.StatusBadRequest, "非法媒体地址")
		return
	}

	req, err := http.NewRequest(http.MethodGet, raw, nil)
	if err != nil {
		apiError(w, http.StatusBadRequest, "非法媒体地址")
		return
	}
	req.Header.Set("User-Agent", UA)
	req.Header.Set("Referer", refererFor(raw))
	if rng := r.Header.Get("Range"); rng != "" {
		req.Header.Set("Range", rng)
	}

	client := &http.Client{Timeout: 120 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		apiError(w, http.StatusBadGateway, fmt.Sprintf("媒体拉取失败：%v", err))
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		apiError(w, http.StatusBadGateway, fmt.Sprintf("媒体拉取失败：HTTP %d", resp.StatusCode))
		return
	}

	w.Header().Set("Cache-Control", "public, max-age=86400")
	// 透传关键响应头（content-type / length / range / accept-ranges），保证播放与拖拽
	for _, h := range []string{"Content-Type", "Content-Length", "Content-Range", "Accept-Ranges"} {
		if v := resp.Header.Get(h); v != "" {
			w.Header().Set(h, v)
		}
	}
	w.WriteHeader(resp.StatusCode)
	_, _ = io.Copy(w, resp.Body)
}
