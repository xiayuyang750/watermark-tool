// X（Twitter）解析器（对齐 backend-rust/src/parsers/x.rs 的纯 HTTP 部分）。
//
// 链路：推文链接 → 提取推文 ID → vxtwitter 优先 / fxtwitter 兜底（公共 API，无需登录）
// → 登录墙兜底（G3 接入 CDP + cookie）。
//
// 媒体：X 的视频 / 动图（GIF）都是 mp4（无平台水印），图片在 pbs.twimg.com；
// 直链无防盗链、支持 Range，可直接播放与下载，无需本地代理。

package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/chromedp"
)

const (
	vxAPI = "https://api.vxtwitter.com/i/status/{tid}"
	fxAPI = "https://api.fxtwitter.com/status/{tid}"
)

var (
	tweetIDRe = regexp.MustCompile(`(?:x|twitter)\.com/(?:[^/?#]+/status/|i/status/)(\d{15,20})`)
	rawIDRe   = regexp.MustCompile(`\b(\d{15,20})\b`)
)

// xURLAvailable 视频直链可用性探测（HEAD，带 UA/Referer）。
func xURLAvailable(u string) bool {
	req, err := http.NewRequest(http.MethodHead, u, nil)
	if err != nil {
		return false
	}
	req.Header.Set("User-Agent", UA)
	req.Header.Set("Referer", "https://x.com/")
	client := &http.Client{Timeout: 15 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode >= 200 && resp.StatusCode < 400
}

// xVideoURL 返回可用的视频直链：video.twimg.com 直链必须带正确的 tag 签名值
//（amplify_video 用 tag=14，ext_tw_video 用 tag=29，传错会返回 404），
// 解析到裸 URL 或 tag 值不对时自动验证并回退重试，避免返回失效直链。
func xVideoURL(u string) string {
	candidates := []string{u}
	if !strings.Contains(u, "tag=") {
		sep := "?"
		if strings.Contains(u, "?") {
			sep = "&"
		}
		candidates = []string{u + sep + "tag=29", u + sep + "tag=14", u}
	} else {
		candidates = []string{u, strings.Replace(u, "tag=29", "tag=14", 1), strings.Replace(u, "tag=14", "tag=29", 1)}
	}
	for _, c := range candidates {
		if xURLAvailable(c) {
			return c
		}
	}
	return u
}

func extractTweetID(text string) (string, error) {
	if m := tweetIDRe.FindStringSubmatch(text); m != nil {
		return m[1], nil
	}
	if m := rawIDRe.FindStringSubmatch(text); m != nil {
		return m[1], nil
	}
	return "", fmt.Errorf("无法从链接中提取推文 ID，请确认是 X/Twitter 的推文链接")
}

// fetchXPublic 公共解析：fxtwitter 优先，vxtwitter 兜底。
// 实测（2026-08）：fxtwitter 返回的媒体 URL 自带正确 tag 签名（amplify_video 有 tag=29/tag=14 之分），
// vxtwitter 会丢失 tag（返回裸 URL 导致部分视频 404）。优先 fxtwitter 可免 tag 猜测；
// vxtwitter 兜底时由 xVideoURL 验证回退补 tag。
func fetchXPublic(tid string) (map[string]interface{}, error) {
	client := &http.Client{Timeout: 20 * time.Second}

	// fxtwitter 优先（URL 自带正确 tag）
	url := strings.ReplaceAll(fxAPI, "{tid}", tid)
	if data, err := fetchJSON(client, url); err == nil {
		if tweet, ok := data["tweet"].(map[string]interface{}); ok {
			if media, ok := tweet["media"].(map[string]interface{}); ok {
				if media["all"] != nil {
					return map[string]interface{}{"source": "fxtwitter", "data": tweet}, nil
				}
			}
		}
	}
	// vxtwitter 兜底（URL 无 tag，交 xVideoURL 验证回退）
	url = strings.ReplaceAll(vxAPI, "{tid}", tid)
	if data, err := fetchJSON(client, url); err == nil {
		if data["media_extended"] != nil || data["mediaURLs"] != nil {
			return map[string]interface{}{"source": "vxtwitter", "data": data}, nil
		}
	}
	return nil, fmt.Errorf("第三方解析服务暂不可用")
}

func fetchJSON(client *http.Client, url string) (map[string]interface{}, error) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", UA)
	req.Header.Set("Accept", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 10<<20))
	if err != nil {
		return nil, err
	}
	var m map[string]interface{}
	if err := json.Unmarshal(body, &m); err != nil {
		return nil, err
	}
	return m, nil
}

// parseX 解析入口：公共 API 优先，失败走登录墙兜底。
func parseX(url string, _ bool) (*ParseResult, error) {
	tid, err := extractTweetID(url)
	if err != nil {
		return nil, err
	}
	data, err := fetchXPublic(tid)
	if err != nil {
		// 登录墙兜底（浏览器 + x_cookie）
		data, err = fetchXWithCookie(tid)
		if err != nil {
			return nil, err
		}
	}
	return xToResult(data, tid)
}

// xToResult 转 ParseResult（source: vxtwitter / fxtwitter / graphql）。
func xToResult(data map[string]interface{}, tid string) (*ParseResult, error) {
	source, _ := data["source"].(string)
	d, _ := data["data"].(map[string]interface{})

	var items []interface{}
	var text string
	switch source {
	case "vxtwitter":
		items, _ = d["media_extended"].([]interface{})
		text, _ = d["text"].(string)
	case "fxtwitter":
		if media, ok := d["media"].(map[string]interface{}); ok {
			items, _ = media["all"].([]interface{})
		}
		text, _ = d["text"].(string)
	case "graphql":
		// graphql 已转成 vxtwitter 兼容结构（media_extended + text）
		items, _ = d["media_extended"].([]interface{})
		text, _ = d["text"].(string)
	default:
		return nil, fmt.Errorf("X 解析失败：第三方解析服务返回异常")
	}
	text = strings.TrimSpace(text)
	title := text
	if title == "" {
		title = fmt.Sprintf("推文 %s", tid)
	}

	files := make([]MediaFile, 0, len(items))
	for _, it := range items {
		m, ok := it.(map[string]interface{})
		if !ok {
			continue
		}
		mtype, _ := m["type"].(string)
		mtype = strings.ToLower(mtype)
		mediaURL, _ := m["url"].(string)
		mediaURL = strings.TrimSpace(mediaURL)
		if mediaURL == "" {
			continue
		}
		if mtype == "video" || mtype == "gif" {
			// X 的动图（GIF）是无声 mp4，统一按视频处理；直链需带正确 tag 签名（自动验证回退）
			var cover *string
			if c, ok := m["thumbnail_url"].(string); ok && c != "" {
				cover = &c
			}
			files = append(files, MediaFile{Kind: "video", URL: xVideoURL(mediaURL), Cover: cover})
		} else {
			// photo / 图集
			files = append(files, MediaFile{Kind: "image", URL: mediaURL})
		}
	}
	if len(files) == 0 {
		return nil, fmt.Errorf("该推文没有可下载的视频/图片内容")
	}
	mediaType := "image"
	if files[0].Kind == "video" {
		mediaType = "video"
	}
	return &ParseResult{Platform: "x", Title: title, MediaType: mediaType, Files: files}, nil
}

// ---- 登录墙兜底（浏览器 + x_cookie，登录态由 xlogin.go 引导流程保存） ----

// fetchXWithCookie 打开推文页并监听 TweetResultByRestId GraphQL 响应（对齐 Rust fetch_with_cookie）。
func fetchXWithCookie(tid string) (map[string]interface{}, error) {
	cookieStr := strings.TrimSpace(loadConfig().XCookie)
	if cookieStr == "" {
		return nil, fmt.Errorf("X 解析失败：该推文可能已删除、仅登录可见，或第三方解析服务暂时不可用")
	}

	edge, err := edgePath()
	if err != nil {
		return nil, err
	}
	profile := filepath.Join(dataDir(), "x_login_profile")
	allocCtx, cancelAlloc := edgeAllocator(edge, profile, true) // headless
	ctx, cancel := chromedp.NewContext(allocCtx)
	defer func() { cancel(); cancelAlloc() }()

	// 注入登录 cookie（双保险：档案意外丢失登录态时恢复）。过滤私有字段避免整体被拒。
	var saved []*network.Cookie
	if err := json.Unmarshal([]byte(cookieStr), &saved); err != nil {
		return nil, fmt.Errorf("登录 Cookie 数据损坏，请重新打开 X 登录")
	}
	params := make([]*network.CookieParam, 0, len(saved))
	for _, c := range saved {
		p := &network.CookieParam{
			Name:     c.Name,
			Value:    c.Value,
			Domain:   c.Domain,
			Path:     c.Path,
			HTTPOnly: c.HTTPOnly,
			Secure:   c.Secure,
			SameSite: c.SameSite,
		}
		if c.Expires > 0 {
			exp := cdp.TimeSinceEpoch(time.Unix(int64(c.Expires), 0))
			p.Expires = &exp
		}
		params = append(params, p)
	}
	if err := chromedp.Run(ctx, chromedp.ActionFunc(func(ctx context.Context) error {
		return network.SetCookies(params).Do(ctx)
	})); err != nil {
		return nil, fmt.Errorf("注入登录 Cookie 失败：%v", err)
	}

	// 监听 TweetResultByRestId 响应
	gqlCh := make(chan string, 1)
	chromedp.ListenTarget(ctx, func(ev interface{}) {
		if e, ok := ev.(*network.EventResponseReceived); ok {
			if strings.Contains(e.Response.URL, "TweetResultByRestId") {
				var body []byte
				if err := chromedp.Run(ctx, chromedp.ActionFunc(func(ctx context.Context) error {
					var err error
					body, err = network.GetResponseBody(e.RequestID).Do(ctx)
					return err
				})); err == nil {
					select {
					case gqlCh <- string(body):
					default:
					}
				}
			}
		}
	})

	// 打开推文页（GraphQL 请求由页面自行发出）
	if err := chromedp.Run(ctx, chromedp.Navigate(fmt.Sprintf("https://x.com/i/status/%s", tid))); err != nil {
		return nil, fmt.Errorf("打开推文页失败：%v", err)
	}

	select {
	case body := <-gqlCh:
		var gql map[string]interface{}
		if err := json.Unmarshal([]byte(body), &gql); err != nil {
			return nil, fmt.Errorf("GraphQL 响应解析失败")
		}
		data := xExtractGraphQL(gql)
		if data == nil {
			return nil, fmt.Errorf("该推文没有可下载的视频/图片内容（或登录态已失效，请重新打开 X 登录）")
		}
		return map[string]interface{}{"source": "graphql", "data": data}, nil
	case <-time.After(45 * time.Second):
		return nil, fmt.Errorf("推文页面加载超时（登录态可能已失效，请重新打开 X 登录）")
	}
}

// xExtractGraphQL 从 GraphQL TweetResultByRestId 响应提取媒体（转成 vxtwitter 兼容结构）。
func xExtractGraphQL(gql map[string]interface{}) map[string]interface{} {
	res := gql["data"].(map[string]interface{})["tweetResult"].(map[string]interface{})["result"].(map[string]interface{})
	media := make([]interface{}, 0)

	if details, ok := res["media_details"].([]interface{}); ok && len(details) > 0 {
		// 新结构：media_details
		for _, it := range details {
			m, _ := it.(map[string]interface{})
			if m == nil {
				continue
			}
			item := map[string]interface{}{
				"id_str": str(m["id"]),
				"type":   strDefault(m["type"], "photo"),
			}
			thumb := ""
			if s, ok := m["media_url_https"].(string); ok {
				thumb = s
			} else if s, ok := m["thumbnail_url"].(string); ok {
				thumb = s
			}
			if item["type"] == "photo" {
				item["url"] = str(m["media_url_https"])
			} else {
				item["url"] = videoVariantURL(m["video"])
			}
			item["thumbnail_url"] = thumb
			media = append(media, item)
		}
	} else if legacy, ok := res["legacy"].(map[string]interface{}); ok {
		if ext, ok := legacy["extended_entities"].(map[string]interface{}); ok {
			if arr, ok := ext["media"].([]interface{}); ok {
				// 旧结构：legacy.extended_entities.media
				for _, it := range arr {
					m, _ := it.(map[string]interface{})
					if m == nil {
						continue
					}
					mtype, _ := m["type"].(string)
					thumb, _ := m["media_url_https"].(string)
					thumb = strings.ReplaceAll(thumb, "http://", "https://")
					item := map[string]interface{}{
						"id_str":         str(m["id_str"]),
						"type":           mtype,
						"thumbnail_url":  thumb,
					}
					if mtype == "photo" {
						item["url"] = thumb
					} else {
						item["url"] = videoVariantURL(m["video_info"])
					}
					media = append(media, item)
				}
			}
		}
	}
	if len(media) == 0 {
		return nil
	}
	text := ""
	if legacy, ok := res["legacy"].(map[string]interface{}); ok {
		if s, ok := legacy["full_text"].(string); ok {
			text = s
		} else if s, ok := legacy["text"].(string); ok {
			text = s
		}
	}
	return map[string]interface{}{"media_extended": media, "text": text}
}

// videoVariantURL 从 video/video_info 的 variants 中选最高码率 mp4。
func videoVariantURL(v interface{}) string {
	var variants []interface{}
	switch t := v.(type) {
	case map[string]interface{}:
		variants, _ = t["variants"].([]interface{})
	}
	bestURL := ""
	bestBitrate := -1
	for _, it := range variants {
		vv, _ := it.(map[string]interface{})
		if vv == nil {
			continue
		}
		ct, _ := vv["content_type"].(string)
		if ct != "video/mp4" {
			continue
		}
		br, _ := vv["bitrate"].(float64)
		if int(br) > bestBitrate {
			bestBitrate = int(br)
			bestURL, _ = vv["url"].(string)
		}
	}
	return bestURL
}

func str(v interface{}) string {
	s, _ := v.(string)
	return s
}

func strDefault(v interface{}, def string) string {
	if s, ok := v.(string); ok && s != "" {
		return s
	}
	return def
}
