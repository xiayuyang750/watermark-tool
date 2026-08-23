package com.watermark.tool

import android.os.Bundle

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // 注意：不要启用 edge-to-edge。安卓 WebView 不识别 CSS env(safe-area-inset-*)，
    // edge-to-edge 会让顶栏被状态栏遮挡（设置按钮点不到）；默认布局由系统在状态栏下方留白。
    super.onCreate(savedInstanceState)
  }
}
