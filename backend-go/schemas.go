// 数据模型，对齐 backend-rust/src/schemas.rs（换语言不换逻辑）。

package main

// ParseRequest 解析请求。
type ParseRequest struct {
	URL              string `json:"url"`
	RemovePlatformWm bool   `json:"remove_platform_wm"`
}

// MediaFile 单条媒体（视频 / 图片 / 动图 / Live 图）。
type MediaFile struct {
	Kind     string  `json:"kind"`      // video / image / gif / livephoto
	URL      string  `json:"url"`
	Label    *string `json:"label"`
	Cover    *string `json:"cover"`     // 视频/封面预览图
	ImageURL *string `json:"image_url"` // Live 图静态照片直链
}

// ParseResult 解析结果。
type ParseResult struct {
	Platform  string      `json:"platform"`
	Title     string      `json:"title"`
	MediaType string      `json:"media_type"`
	Files     []MediaFile `json:"files"`
}

// FeedbackRequest 问题反馈请求。
type FeedbackRequest struct {
	Content string `json:"content"`
	Contact string `json:"contact"`
}
