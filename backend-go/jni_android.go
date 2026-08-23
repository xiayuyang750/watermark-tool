//go:build android

// 安卓端：编译为 JNI 动态库（c-shared）由 APK 的 System.loadLibrary 加载。
// 鸿蒙/Android SELinux 禁止 app exec 数据目录二进制，只有 native lib（APK 内）
// 允许加载执行，故 Go 后端以 JNI 库形态运行（Java 层触发，HTTP 服务异步启动）。
package main

/*
#cgo CFLAGS: -I C:/Users/21494/AppData/Local/Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/windows-x86_64/sysroot/usr/include
#include <jni.h>
#include <stdlib.h>

static const char* jGetUTF(JNIEnv* env, jstring s) {
    return (*env)->GetStringUTFChars(env, s, 0);
}
static void jReleaseUTF(JNIEnv* env, jstring s, const char* c) {
    (*env)->ReleaseStringUTFChars(env, s, c);
}
*/
import "C"

import (
	"log"
	"os"
)

// c-shared 构建需要 main 符号（不会被执行）。
func main() {}

//export Java_com_watermark_tool_MainActivity_wmBackendStart
func Java_com_watermark_tool_MainActivity_wmBackendStart(env *C.JNIEnv, thiz C.jobject, home C.jstring) {
	cs := C.jGetUTF(env, home)
	if cs == nil {
		return
	}
	homePath := C.GoString(cs)
	C.jReleaseUTF(env, home, cs)
	os.Setenv("WATERMARK_TOOL_HOME", homePath)
	// 异步启动 HTTP 服务，避免阻塞 Java 调用线程（触发 ANR）。
	// 必须 recover 一切 panic——c-shared 模式下未恢复的 panic 会直接崩溃整个 app 进程。
	go func() {
		defer func() {
			if r := recover(); r != nil {
				log.Printf("[wm-jni] startServer panic: %v", r)
			}
		}()
		startServer()
	}()
}
