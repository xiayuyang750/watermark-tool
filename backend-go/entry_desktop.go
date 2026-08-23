//go:build !android

// 桌面/常规平台入口：直接启动本地 HTTP 服务。
package main

func main() {
	startServer()
}
