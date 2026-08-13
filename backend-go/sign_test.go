package main

import "testing"

// 固定 UA + 固定时间戳，与 Rust/Python 版同参数输出对比（G2 验收）。
func TestXBogusMatchesPythonSample(t *testing.T) {
	signed, xb := xbGetAt(
		"https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=123456",
		xbDefaultUA,
		1700000000,
	)
	if xb != "DFSzswVYkabANxTQtmWx-e9WX7rJ" {
		t.Fatalf("X-Bogus 应与 Python 版逐字符一致，得到: %s", xb)
	}
	if signed != "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=123456&X-Bogus="+xb {
		t.Fatalf("签名 URL 拼接错误: %s", signed)
	}
}

func TestXBogusDeterministic(t *testing.T) {
	a, xa := xbGetAt("https://www.douyin.com/x?a=1", xbDefaultUA, 1700000000)
	b, xb := xbGetAt("https://www.douyin.com/x?a=1", xbDefaultUA, 1700000000)
	if a != b || xa != xb {
		t.Fatalf("同参数应输出一致")
	}
	if len(xa) != 28 {
		t.Fatalf("X-Bogus 长度应为 28，得到 %d", len(xa))
	}
}

func TestXBRC4Deterministic(t *testing.T) {
	out1 := xbRC4Encrypt([]byte{0xff}, []byte("hello"))
	out2 := xbRC4Encrypt([]byte{0xff}, []byte("hello"))
	if string(out1) != string(out2) || len(out1) != 5 {
		t.Fatalf("RC4 应确定且长度一致: %v", out1)
	}
}
