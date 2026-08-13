// 抖音签名算法（对齐 backend-rust/src/parsers/sign.rs / Python douyin_sig.py）。
//
// XBogus（Apache 2.0）：零依赖实现（MD5 + RC4 + 自定义编码），作纯 HTTP 备用通道签名。
// ABogus（GPL v3，依赖 SM3）：许可证传染，不移植；纯 HTTP 备用通道统一使用 XBogus。

package main

import (
	"crypto/md5"
	"encoding/base64"
	"fmt"
	"strings"
	"time"
)

// xbCharset 与 Rust CHAR_MAP 注释对应：'0'-'9'→0-9，'a'-'f'→10-15
const xbCharset = "Dkdpgh4ZKsQB80/Mfvw36XI1R25-WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe="

var xbUAKey = []byte{0x00, 0x01, 0x0c}

const xbDefaultUA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Edg/122.0.0.0"

func xbCharMap() [128]byte {
	var m [128]byte
	for i := 0; i < 10; i++ {
		m[48+i] = byte(i)
	}
	for i := 10; i < 16; i++ {
		m[97+(i-10)] = byte(i)
	}
	return m
}

// xbMD5StrToArray hex 字符串 → 字节数组（对应 Python _md5_str_to_array）
func xbMD5StrToArray(md5Str string) []byte {
	if len(md5Str) > 32 {
		return []byte(md5Str)
	}
	m := xbCharMap()
	arr := make([]byte, 0, len(md5Str)/2)
	for i := 0; i < len(md5Str); i += 2 {
		high := m[md5Str[i]]
		low := m[md5Str[i+1]]
		arr = append(arr, (high<<4)|low)
	}
	return arr
}

func xbMD5(data []byte) string {
	sum := md5.Sum(data)
	return fmt.Sprintf("%x", sum)
}

// xbMD5Encrypt url_path 经两次 md5 → 字节数组（对应 Python _md5_encrypt）
func xbMD5Encrypt(urlPath string) []byte {
	first := xbMD5([]byte(urlPath))
	second := xbMD5(xbMD5StrToArray(first))
	return xbMD5StrToArray(second)
}

// xbRC4Encrypt RC4（对应 Python _rc4_encrypt）
func xbRC4Encrypt(key, data []byte) []byte {
	s := make([]byte, 256)
	for i := range s {
		s[i] = byte(i)
	}
	j := 0
	for i := 0; i < 256; i++ {
		j = (j + int(s[i]) + int(key[i%len(key)])) % 256
		s[i], s[j] = s[j], s[i]
	}
	i := 0
	j = 0
	out := make([]byte, len(data))
	for k, b := range data {
		i = (i + 1) % 256
		j = (j + int(s[i])) % 256
		s[i], s[j] = s[j], s[i]
		t := (int(s[i]) + int(s[j])) % 256
		out[k] = b ^ s[t]
	}
	return out
}

// xbCalc 3 字节 → 4 字符 charset 编码（对应 Python _calc）
func xbCalc(a1, a2, a3 byte) [4]byte {
	x := uint32(a1)<<16 | uint32(a2)<<8 | uint32(a3)
	return [4]byte{
		xbCharset[(x>>18)&63],
		xbCharset[(x>>12)&63],
		xbCharset[(x>>6)&63],
		xbCharset[x&63],
	}
}

// xbGetAt 生成 X-Bogus。返回 (带 X-Bogus 的参数字符串, X-Bogus 值)。
// timer 可注入（测试固定时间便于与 Python/Rust 对比）。
func xbGetAt(urlPath, ua string, timer uint32) (string, string) {
	// array1 = md5_str_to_array(md5(base64(rc4(ua_key, user_agent))))
	rc4ed := xbRC4Encrypt(xbUAKey, []byte(ua))
	b64 := base64.StdEncoding.EncodeToString(rc4ed)
	array1 := xbMD5StrToArray(xbMD5([]byte(b64)))

	// array2 = md5_str_to_array(md5(md5_str_to_array("d41d8...")))（空串 md5 常量）
	array2 := xbMD5StrToArray(xbMD5(xbMD5StrToArray("d41d8cd98f00b204e9800998ecf8427e")))

	urlPathArray := xbMD5Encrypt(urlPath)
	ct := uint32(536919696)

	newArray := []uint32{
		64, 0, 1, 12,
		uint32(urlPathArray[14]), uint32(urlPathArray[15]),
		uint32(array2[14]), uint32(array2[15]),
		uint32(array1[14]), uint32(array1[15]),
		(timer >> 24) & 255, (timer >> 16) & 255, (timer >> 8) & 255, timer & 255,
		(ct >> 24) & 255, (ct >> 16) & 255, (ct >> 8) & 255, ct & 255,
	}
	xorResult := newArray[0]
	for _, b := range newArray[1:] {
		xorResult ^= b
	}
	newArray = append(newArray, xorResult)

	var array3, array4 []uint32
	for i := 0; i < len(newArray); i += 2 {
		array3 = append(array3, newArray[i])
	}
	for i := 1; i < len(newArray); i += 2 {
		array4 = append(array4, newArray[i])
	}
	merge := append(array3, array4...)

	// 索引映射对齐 Rust 注释（y 19 字节，merge[10] 为 float 0.00390625 → int 0）
	y := []byte{
		byte(merge[0]), 0, byte(merge[1]), byte(merge[11]),
		byte(merge[2]), byte(merge[12]), byte(merge[3]), byte(merge[13]),
		byte(merge[4]), byte(merge[14]), byte(merge[5]), byte(merge[15]),
		byte(merge[6]), byte(merge[16]), byte(merge[7]), byte(merge[17]),
		byte(merge[8]), byte(merge[18]), byte(merge[9]),
	}

	rc4ed2 := xbRC4Encrypt([]byte{0xff}, y)
	garbled := append([]byte{2, 255}, rc4ed2...)

	var xb strings.Builder
	for i := 0; i+2 < len(garbled); i += 3 {
		s := xbCalc(garbled[i], garbled[i+1], garbled[i+2])
		xb.Write(s[:])
	}
	return urlPath + "&X-Bogus=" + xb.String(), xb.String()
}

// xbGet 使用当前时间戳生成 X-Bogus。
func xbGet(urlPath string) (string, string) {
	return xbGetAt(urlPath, xbDefaultUA, uint32(time.Now().Unix()))
}
