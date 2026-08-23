package com.douyin.downloader

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import java.io.BufferedReader
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStreamReader
import java.net.HttpURLConnection
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.URL
import java.util.regex.Pattern
import kotlin.concurrent.thread

class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private val handler = Handler(Looper.getMainLooper())
    private var pendingShareText: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        webView = findViewById(R.id.webView)
        setupWebView()
        startProxyServer()
        handleShareIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShareIntent(intent)
    }

    private fun handleShareIntent(intent: Intent?) {
        pendingShareText = intent?.getStringExtra(Intent.EXTRA_TEXT)
        webView.postDelayed({
            pendingShareText?.let { processShareText(it) }
            pendingShareText = null
        }, 1000)
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        WebView.setWebContentsDebuggingEnabled(true)
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            cacheMode = WebSettings.LOAD_DEFAULT
        }
        webView.webChromeClient = WebChromeClient()
        webView.webViewClient = object : WebViewClient() {
            // 图片走本地代理 media://proxy?url=xxx（前端 mediaSrc() 生成，带 UA/Referer 绕过 CDN 防盗链）。
            // 注意：shouldInterceptRequest 对 <video> 媒体资源不生效，故视频不走这里，
            // 而是走标准 http://127.0.0.1:port/media 本地代理（startProxyServer），WebView 媒体栈可正常消费。
            override fun shouldInterceptRequest(view: WebView?, request: WebResourceRequest?): WebResourceResponse? {
                val req = request ?: return null
                val url = req.url?.toString() ?: return null
                if (url.startsWith("media://proxy")) {
                    val target = url.substringAfter("url=", "").let { android.net.Uri.decode(it) }
                    if (target.isNotEmpty()) {
                        fetchImageBytes(target)?.let { return it }
                    }
                }
                return super.shouldInterceptRequest(view, request)
            }
        }
        webView.addJavascriptInterface(DownloadBridge(), "DownloadBridge")
        webView.loadUrl("file:///android_asset/index.html")
    }

    // ---- 分享文本处理（平台分流） ----

    fun processShareText(text: String) {
        val url = extractUrl(text) ?: run {
            sendStatus("未找到有效链接", "error")
            return
        }
        sendStatus("正在解析链接...", "")
        thread { resolveAndParse(url) }
    }

    private fun resolveAndParse(shareUrl: String) {
        try {
            // 按链接特征自动路由到对应平台的解析器（后端解析分派，不依赖前端选的平台）
            when {
                isXUrl(shareUrl) -> parseX(shareUrl)
                isDouyinUrl(shareUrl) -> parseDouyin(shareUrl)
                else -> throw Exception("暂不支持的平台链接")
            }
        } catch (e: Exception) {
            e.printStackTrace()
            handler.post { sendStatus("解析失败: ${e.message}", "error") }
        }
    }

    // ================= 抖音（feed 接口，无需签名） =================

    private fun parseDouyin(shareUrl: String) {
        // 1. 跟随短链接重定向，边跳边精确提取作品 ID
        val awemeId = resolveAwemeId(shareUrl)
            ?: throw Exception("无法从链接中提取作品 ID")

        handler.post { sendStatus("已获取作品 ID，正在请求详情...", "") }

        // 2. 调用抖音 App feed 接口（当前无需签名）
        val feedUrl = "https://aweme.snssdk.com/aweme/v1/feed/?aweme_id=$awemeId"
        val feedResponse = httpGet(feedUrl, mapOf(
            "User-Agent" to APP_UA,
            "Accept" to "application/json"
        ))

        // 3. 解析 JSON
        val json = org.json.JSONObject(feedResponse)
        val awemeList = json.optJSONArray("aweme_list")
            ?: throw Exception("接口未返回作品列表")
        if (awemeList.length() == 0) throw Exception("作品列表为空")

        // 4. 用目标 aweme_id 精确匹配，而不是取第 0 条（防 feed 返回推荐流）
        var aweme: org.json.JSONObject? = null
        for (i in 0 until awemeList.length()) {
            val item = awemeList.optJSONObject(i) ?: continue
            if (item.optString("aweme_id") == awemeId) { aweme = item; break }
        }
        if (aweme == null) aweme = awemeList.optJSONObject(0)
            ?: throw Exception("作品列表为空")

        val title = aweme.optString("desc", "未命名作品").trim()

        // 判断是否图文/动图作品：有 images 数组则按图片处理，否则按视频
        val images = aweme.optJSONArray("images")
        val video = aweme.optJSONObject("video")

        if (images != null && images.length() > 0) {
            // 图文/图集：纯 HTTP 无法拿全多图 + 动图素材（需签名/浏览器），降级取第一张图
            val firstImg = images.optJSONObject(0)
            val imgUrl = firstImg?.optJSONArray("url_list")?.optString(0)?.replace("http://", "https://")
            if (imgUrl != null) {
                handler.post { sendVideoReady(imgUrl, title, null, "image", awemeId) }
            } else {
                handler.post { sendStatus("该内容（图集/动图/实况）纯 HTTP 暂不支持完整解析", "error") }
            }
            return
        }

        val playAddr = video?.optJSONObject("play_addr")
            ?: throw Exception("未找到播放地址")
        val urlList = playAddr.optJSONArray("url_list")
            ?: throw Exception("未找到视频 URL")
        if (urlList.length() == 0) throw Exception("视频 URL 列表为空")

        val videoUrl = urlList.getString(0).replace("http://", "https://")
        val cover = extractCover(video)
        handler.post { sendVideoReady(videoUrl, title, cover, "video", awemeId) }
    }

    /** 从抖音视频对象提取封面直链。 */
    private fun extractCover(video: org.json.JSONObject): String? {
        val coverUrl = video.optJSONObject("cover")?.optJSONArray("url_list")
            ?: video.optJSONObject("origin_cover")?.optJSONArray("url_list")
            ?: return null
        return if (coverUrl.length() > 0) coverUrl.getString(0).replace("http://", "https://") else null
    }

    // ================= X（fxtwitter 优先，vxtwitter 兜底，纯 HTTP） =================

    private fun parseX(shareUrl: String) {
        val tid = extractTweetId(shareUrl) ?: throw Exception("无法从链接中提取推文 ID")
        handler.post { sendStatus("已获取推文 ID，正在请求详情...", "") }

        val media = fetchXPublic(tid)
        handler.post { sendVideoReady(media.url, media.title, media.cover, media.kind, tid) }
    }

    /** X 解析结果。kind 取 video 或 image。 */
    private data class XMedia(val url: String, val title: String, val cover: String?, val kind: String)

    /** 解析 X 推文。fxtwitter 优先，vxtwitter 兜底。 */
    private fun fetchXPublic(tid: String): XMedia {
        // fxtwitter 优先：URL 自带正确 tag 签名
        val fx = "https://api.fxtwitter.com/status/$tid"
        try {
            val body = httpGet(fx, mapOf("User-Agent" to WEB_UA, "Accept" to "application/json"))
            val root = org.json.JSONObject(body)
            val tweet = root.optJSONObject("tweet")
            val text = tweet?.optString("text", "").orEmpty().trim()
            val media = tweet?.optJSONObject("media")?.optJSONArray("all")
            val (url, kind) = pickFirstMedia(media) ?: (null to null)
            val cover = pickFirstCover(media)
            if (url != null) return XMedia(ensureVideoTag(url), text.ifEmpty { "推文 $tid" }, cover, kind ?: "video")
        } catch (e: Exception) {
            // 忽略，走 vxtwitter 兜底
        }

        // vxtwitter 兜底：可能丢 tag，交给 ensureVideoTag 补
        val vx = "https://api.vxtwitter.com/i/status/$tid"
        val body = httpGet(vx, mapOf("User-Agent" to WEB_UA, "Accept" to "application/json"))
        val root = org.json.JSONObject(body)
        val text = root.optString("text", "").trim()
        val media = root.optJSONArray("media_extended") ?: root.optJSONArray("media_info")
        val (url, kind) = pickFirstMedia(media) ?: throw Exception("该推文没有可下载的视频/图片")
        val cover = pickFirstCover(media)
        return XMedia(ensureVideoTag(url), text.ifEmpty { "推文 $tid" }, cover, kind)
    }

    /** 从媒体数组里挑一个可下载对象（优先视频，否则第一张图）。返回 (url, kind)。 */
    private fun pickFirstMedia(arr: org.json.JSONArray?): Pair<String, String>? {
        if (arr == null || arr.length() == 0) return null
        var firstUrl: String? = null
        for (i in 0 until arr.length()) {
            val obj = arr.optJSONObject(i) ?: continue
            val mtype = obj.optString("type", "photo").lowercase()
            var url = obj.optString("url").ifEmpty { obj.optString("media_url_https") }
            url = url.replace("http://", "https://")
            if (url.isEmpty()) continue
            if (firstUrl == null) firstUrl = url
            if (mtype == "video" || mtype == "gif") return url to "video"
        }
        firstUrl?.let { return it to "image" }
        return null
    }

    /** 从媒体数组里挑封面缩略图 URL。 */
    private fun pickFirstCover(arr: org.json.JSONArray?): String? {
        if (arr == null || arr.length() == 0) return null
        for (i in 0 until arr.length()) {
            val obj = arr.optJSONObject(i) ?: continue
            var thumb = obj.optString("thumbnail_url").ifEmpty { obj.optString("media_url_https") }
            if (thumb.isNotEmpty()) return thumb.replace("http://", "https://")
        }
        return null
    }

    /** X 视频直链（video.twimg.com）需带 tag 签名，缺失时补 tag=29。 */
    private fun ensureVideoTag(url: String): String {
        if (!url.contains("video.twimg.com")) return url
        if (url.contains("tag=")) return url
        val sep = if (url.contains("?")) "&" else "?"
        return url + sep + "tag=29"
    }

    // ================= 通用 HTTP 工具 =================

    /**
     * 短链重定向并提取作品 ID。
     * 关键：每一跳都尝试精确提取 /video/{id} 或 /note/{id}，一命中即返回，
     * 避免只跳一次就停在 iesdouyin.com 的长 URL（里面塞满 did/iid/mid 等数字，
     * 若用宽泛正则会把无关 id 当成本体，导致解析出别的视频）。
     */
    private fun resolveAwemeId(startUrl: String): String? {
        var cur = startUrl
        var hops = 0
        while (hops < 8) {
            // 每跳先去当前 URL 里精确找 id
            extractAwemeIdStrict(cur)?.let { return it }

            val conn = URL(cur).openConnection() as HttpURLConnection
            conn.instanceFollowRedirects = false
            conn.requestMethod = "GET"
            conn.setRequestProperty("User-Agent", APP_UA)
            conn.connectTimeout = 15000
            conn.readTimeout = 15000
            conn.connect()

            val code = conn.responseCode
            val location = conn.getHeaderField("Location")
            conn.disconnect()

            if (code in 300..399 && location != null) {
                cur = if (location.startsWith("http")) location
                      else URL(URL(cur), location).toString()
                hops++
            } else {
                // 不会再跳了，最后再用精确匹配找一次
                extractAwemeIdStrict(cur)?.let { return it }
                // 兜底：仅在 path 形如 /note/{id} 或 /video/{id} 时用宽泛匹配（避免抓 did/iid）
                return extractAwemeIdFallback(cur)
            }
        }
        return extractAwemeIdStrict(cur) ?: extractAwemeIdFallback(cur)
    }

    /** 精确匹配 /video/{id} 或 /note/{id}（只认路径里的数字）。 */
    private fun extractAwemeIdStrict(url: String): String? {
        var m = Pattern.compile("/(video|note)/(\\d+)").matcher(url)
        if (m.find()) return m.group(2)
        // 兼容 iesdouyin.com/share/note/{id}
        m = Pattern.compile("/share/(?:note|video)/(\\d+)").matcher(url)
        if (m.find()) return m.group(1)
        return null
    }

    /** 兜底：仅当 URL 路径以 /note/ 或 /video/ 结尾那段存在数字时才取，
     *  避免从 did/iid/mid/query 参数里误抓无关的 15-21 位数字。 */
    private fun extractAwemeIdFallback(url: String): String? {
        // 从 query 前的 path 段里找 //xxx/{数字}
        val pathNoQuery = url.substringBefore('?')
        val m = Pattern.compile("/(note|video)/(\\d{15,21})\\b").matcher(pathNoQuery)
        if (m.find()) return m.group(2)
        return null
    }

    private fun httpGet(url: String, headers: Map<String, String>): String {
        val conn = URL(url).openConnection() as HttpURLConnection
        conn.requestMethod = "GET"
        headers.forEach { (k, v) -> conn.setRequestProperty(k, v) }
        conn.connectTimeout = 20000
        conn.readTimeout = 30000
        conn.connect()
        return conn.inputStream.bufferedReader().use { it.readText() }.also { conn.disconnect() }
    }

    // ================= 下载 =================

    private fun startDownload(videoUrl: String, title: String) {
        sendStatus("正在下载...", "")
        thread {
            try {
                val conn = URL(videoUrl).openConnection() as HttpURLConnection
                conn.setRequestProperty("User-Agent", WEB_UA)
                // 按平台自动选 Referer
                conn.setRequestProperty("Referer", refererFor(videoUrl))
                conn.connectTimeout = 30000
                conn.readTimeout = 120000
                conn.connect()

                val isImage = videoUrl.contains("twimg.com") && !videoUrl.contains("video.")
                val ext = if (isImage) "jpg" else "mp4"
                // 文件名规则对齐 backend-go：标题_时间戳.ext；rune 截断防多字节乱码
                val clean = title.replace(Regex("[\\\\/:*?\"<>|\r\n]"), "_").trim().ifEmpty { "video" }
                val truncated = if (clean.length > 60) clean.substring(0, 60) else clean
                val ts = java.text.SimpleDateFormat("yyyyMMddHHmmss", java.util.Locale.US).format(java.util.Date())
                val fileName = "${truncated}_$ts.$ext"
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    saveToMediaStore(conn, fileName, isImage)
                } else {
                    saveToFile(conn, fileName)
                }
                conn.disconnect()

                handler.post {
                    sendStatus("下载完成！已保存到 Movies/DouyinDownloader", "success")
                    Toast.makeText(this, "下载完成: $fileName", Toast.LENGTH_LONG).show()
                }
            } catch (e: Exception) {
                e.printStackTrace()
                val msg = e.message ?: "未知错误"
                val friendly = if (msg.contains("timeout", true) || msg.contains("connect", true) ||
                    msg.contains("unreachable", true) || msg.contains("failed to connect", true)) {
                    "当前网络无法访问视频源（X 视频存于 video.twimg.com，国内网络常不可达），请检查网络或连接代理后重试"
                } else {
                    "下载失败: $msg"
                }
                handler.post {
                    sendStatus(friendly, "error")
                    Toast.makeText(this, friendly, Toast.LENGTH_SHORT).show()
                }
            }
        }
    }

    private fun refererFor(url: String): String {
        return if (url.contains("douyin") || url.contains("365yg") || url.contains("snssdk") || url.contains("douyinvod")) {
            "https://www.douyin.com/"
        } else {
            "https://x.com/"
        }
    }

    /** 图片代理：带 UA/Referer 拉取真实图片字节，返回给 WebView 渲染。失败返回 null。 */
    private fun fetchImageBytes(raw: String): WebResourceResponse? {
        return try {
            val conn = URL(raw).openConnection() as HttpURLConnection
            conn.requestMethod = "GET"
            conn.setRequestProperty("User-Agent", WEB_UA)
            conn.setRequestProperty("Referer", refererFor(raw))
            conn.connectTimeout = 15000
            conn.readTimeout = 20000
            conn.connect()
            val mime = conn.contentType?.substringBefore(';') ?: "image/jpeg"
            val bytes = conn.inputStream.use { it.readBytes() }
            conn.disconnect()
            WebResourceResponse(mime, "utf-8", java.io.ByteArrayInputStream(bytes))
        } catch (e: Exception) {
            null
        }
    }

    // ================= 本地视频代理（标准 http，解决 WebView <video> 无法播放防盗链视频） =================
    // shouldInterceptRequest 对 <video> 媒体资源不生效（实测 `<video>` 请求不会走它），因此起一个本地 HTTP 代理服务器。
    // WebView <video> 加载 http://127.0.0.1:port/media?url=xxx（标准 http 源，WebView 媒体栈可正常消费），
    // 服务器转发时注入正确 UA/Referer，绕开 CDN 对 file:// Referer 的 403 拦截。对齐 backend-go /media 思路。

    private var proxyServer: ServerSocket? = null
    private var proxyPort = 0

    private fun startProxyServer() {
        thread {
            try {
                val server = ServerSocket(0, 32, InetAddress.getByName("127.0.0.1"))
                proxyPort = server.localPort
                proxyServer = server
                while (!server.isClosed) {
                    try {
                        val sock = server.accept()
                        Thread { serveMediaRequest(sock) }.start()
                    } catch (e: Exception) {
                        if (server.isClosed) break
                    }
                }
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }
    }

    private fun serveMediaRequest(sock: Socket) {
        try {
            sock.soTimeout = 60000
            val reader = BufferedReader(InputStreamReader(sock.getInputStream(), Charsets.ISO_8859_1))
            val requestLine = reader.readLine() ?: return
            val parts = requestLine.split(" ")
            if (parts.size < 2 || parts[0] != "GET") return
            // 解析 Range 头（视频分段加载用）
            var range: String? = null
            var line: String?
            while (true) {
                line = reader.readLine() ?: break
                if (line.isEmpty()) break
                if (line.startsWith("Range:", ignoreCase = true)) range = line.substringAfter(':').trim()
            }
            val target = parts[1].substringAfter("url=")
            if (target.isEmpty()) return
            relayStream(sock, java.net.URLDecoder.decode(target, "UTF-8"), range)
        } catch (e: Exception) {
            // 忽略：连接异常直接关闭
        } finally {
            try { sock.close() } catch (_: Exception) {}
        }
    }

    private fun relayStream(sock: Socket, raw: String, range: String?) {
        val conn = URL(raw).openConnection() as HttpURLConnection
        try {
            conn.requestMethod = "GET"
            conn.setRequestProperty("User-Agent", WEB_UA)
            conn.setRequestProperty("Referer", refererFor(raw))
            if (!range.isNullOrBlank()) conn.setRequestProperty("Range", range)
            conn.connectTimeout = 15000
            conn.readTimeout = 60000
            conn.connect()
            val status = conn.responseCode
            val mime = conn.contentType?.substringBefore(';') ?: "video/mp4"
            val reason = when (status) {
                200 -> "OK"; 206 -> "Partial Content"; 302 -> "Found"; 403 -> "Forbidden"; 404 -> "Not Found"; else -> "OK"
            }
            val out = sock.getOutputStream()
            fun w(s: String) { out.write(s.toByteArray(Charsets.ISO_8859_1)) }
            w("HTTP/1.1 $status $reason\r\n")
            w("Content-Type: $mime\r\n")
            w("Accept-Ranges: bytes\r\n")
            conn.getHeaderField("Content-Length")?.let { w("Content-Length: $it\r\n") }
            conn.getHeaderField("Content-Range")?.let { w("Content-Range: $it\r\n") }
            w("\r\n")
            out.flush()
            val input = if (status in 200..399) conn.inputStream else conn.errorStream
            input.use { ins ->
                val buf = ByteArray(16 * 1024)
                var n: Int
                while (ins.read(buf).also { n = it } != -1) out.write(buf, 0, n)
                out.flush()
            }
            conn.disconnect()
        } catch (e: Exception) {
            // 忽略
        }
    }

    private fun saveToMediaStore(conn: HttpURLConnection, fileName: String, isImage: Boolean) {
        val collection = if (isImage) {
            android.provider.MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        } else {
            android.provider.MediaStore.Video.Media.EXTERNAL_CONTENT_URI
        }
        val mime = if (isImage) "image/jpeg" else "video/mp4"
        val values = android.content.ContentValues().apply {
            put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME, fileName)
            put(android.provider.MediaStore.MediaColumns.MIME_TYPE, mime)
            put(android.provider.MediaStore.MediaColumns.RELATIVE_PATH, "Movies/DouyinDownloader")
        }
        val uri = contentResolver.insert(collection, values)
            ?: throw Exception("无法创建文件")
        contentResolver.openOutputStream(uri)?.use { os ->
            val buf = ByteArray(8192)
            var n: Int
            while (conn.inputStream.read(buf).also { n = it } != -1) os.write(buf, 0, n)
        }
    }

    private fun saveToFile(conn: HttpURLConnection, fileName: String) {
        val dir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_MOVIES)
        val file = java.io.File(dir, "DouyinDownloader/$fileName")
        file.parentFile?.mkdirs()
        file.outputStream().use { os ->
            val buf = ByteArray(8192)
            var n: Int
            while (conn.inputStream.read(buf).also { n = it } != -1) os.write(buf, 0, n)
        }
        android.media.MediaScannerConnection.scanFile(this, arrayOf(file.absolutePath), null, null)
    }

    // ---- JS Bridge ----

    inner class DownloadBridge {
        @JavascriptInterface
        fun processShareText(text: String) {
            handler.post { this@MainActivity.processShareText(text) }
        }

        // 返回本地视频代理端口，前端拼接 http://127.0.0.1:port/media?url=xxx
        @JavascriptInterface
        fun getProxyPort(): Int = proxyPort

        @JavascriptInterface
        fun downloadVideo(videoUrl: String, title: String) {
            handler.post { startDownload(videoUrl, title) }
        }

        @JavascriptInterface
        fun showToast(message: String) {
            handler.post { Toast.makeText(this@MainActivity, message, Toast.LENGTH_SHORT).show() }
        }

        @JavascriptInterface
        fun copyText(text: String) {
            handler.post {
                val cm = getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                cm.setPrimaryClip(android.content.ClipData.newPlainText("douyin", text))
                Toast.makeText(this@MainActivity, "已复制", Toast.LENGTH_SHORT).show()
            }
        }

        @JavascriptInterface
        fun readClipboard(): String {
            return try {
                val cm = getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                val clip = cm.primaryClip
                if (clip != null && clip.itemCount > 0) {
                    clip.getItemAt(0).coerceToText(this@MainActivity).toString()
                } else ""
            } catch (e: Exception) { "" }
        }

        // 历史持久化：存本地文件（比 WebView localStorage 可靠，参考 backend-go history.json）
        @JavascriptInterface
        fun loadHistory(): String {
            return try {
                val f = File(filesDir, "history.json")
                if (f.exists()) f.readText() else "[]"
            } catch (e: Exception) { "[]" }
        }

        @JavascriptInterface
        fun saveHistory(json: String) {
            handler.post {
                try {
                    File(filesDir, "history.json").writeText(json)
                } catch (e: Exception) {
                    /* 忽略写入异常 */
                }
            }
        }
    }

    // ---- 工具方法 ----

    private fun extractUrl(text: String): String? {
        val m = Pattern.compile("https?://\\S+").matcher(text)
        return if (m.find()) m.group(0)?.trimEnd('.', ',', '，', '。', ';', '；') else null
    }

    private fun isXUrl(url: String): Boolean {
        val lower = url.lowercase()
        return xHosts.any { lower.contains(it) }
    }

    private fun isDouyinUrl(url: String): Boolean {
        val lower = url.lowercase()
        return lower.contains("douyin.com") || lower.contains("iesdouyin.com") || lower.contains("v.douyin")
    }

    private fun extractTweetId(url: String): String? {
        var m = Pattern.compile("/(?:[^/?#]+/status/|i/status/)(\\d{1,20})").matcher(url)
        if (m.find()) return m.group(1)
        m = Pattern.compile("\\b(\\d{1,20})\\b").matcher(url)
        if (m.find()) return m.group(1)
        return null
    }

    private fun sendVideoReady(url: String, title: String, coverUrl: String?, kind: String, itemId: String) {
        sendStatus("解析成功，点击下载", "success")
        val cover = coverUrl?.replace("'", "\\'") ?: ""
        val kindJson = if (kind == "image") "image" else "video"
        val idJson = itemId.replace("'", "\\'")
        webView.evaluateJavascript(
            "window.onVideoReady('${title.replace("'", "\\'").replace("\n", " ")}', '${url.replace("'", "\\'")}', '$cover', '$kindJson', '$idJson')",
            null
        )
        Toast.makeText(this, "解析成功: $title", Toast.LENGTH_SHORT).show()
    }

    private fun sendStatus(msg: String, type: String) {
        webView.evaluateJavascript(
            "window.onStatusUpdate('${msg.replace("'", "\\'")}', '$type')",
            null
        )
    }

    companion object {
        private const val APP_UA = "com.ss.android.ugc.aweme/260201 (Linux; U; Android 12; zh_CN; Pixel 4; Build/SP1A.210812.016; Cronet/TTNetVersion)"
        private const val WEB_UA = "Mozilla/5.0 (Linux; Android 12; Pixel 4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36"
        private val xHosts = listOf("x.com", "twitter.com", "vxtwitter.com", "fxtwitter.com", "nitter.net")
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        if (webView.canGoBack()) webView.goBack() else super.onBackPressed()
    }
}
