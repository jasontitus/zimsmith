package org.zimstudio.files

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Bitmap
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.webkit.*
import androidx.webkit.ProfileStore
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import org.json.JSONObject
import java.io.ByteArrayInputStream

class PageCapture(private val activity: Activity, private val args: RenderPageArgs, private val invoke: Invoke, private val complete: (String) -> Unit) {
    private val handler = Handler(Looper.getMainLooper())
    private var browser: WebView? = null
    private var finished = false
    private var evaluating = false
    private val id = args.request.id
    private val profile = "zimsmith-capture-$id"
    private val scopes = args.request.scopes
    private val timeout = args.request.timeout_secs.coerceIn(10, 180)
    private val cap = args.request.max_html_bytes.coerceAtMost(8 * 1024 * 1024)
    private val tick = object : Runnable { override fun run() { poll(); if (!finished) handler.postDelayed(this, 500) } }

    @SuppressLint("SetJavaScriptEnabled")
    fun start() {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT) || !WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
            fail("Update Android System WebView to enable isolated JavaScript capture, or use static capture."); return
        }
        val view = WebView(activity)
        browser = view
        WebViewCompat.setProfile(view, profile)
        view.settings.apply {
            javaScriptEnabled = true; domStorageEnabled = true
            allowFileAccess = false; allowContentAccess = false
            javaScriptCanOpenWindowsAutomatically = false; setSupportMultipleWindows(false)
            mediaPlaybackRequiresUserGesture = true; mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            setGeolocationEnabled(false); cacheMode = WebSettings.LOAD_NO_CACHE
        }
        view.alpha = 0.01f; view.isEnabled = false
        view.importantForAccessibility = android.view.View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        val host = activity.findViewById<ViewGroup>(android.R.id.content)
        host.addView(view, 0, ViewGroup.LayoutParams(host.width.coerceAtLeast(360), host.height.coerceAtLeast(640)))
        val options = JSONObject().put("delay_ms", args.request.delay_ms).put("timeout_secs", timeout).put("max_html_bytes", cap)
        WebViewCompat.addDocumentStartJavaScript(view, "window.__zimsmithRenderOptions=$options;\n${args.script}", setOf("*"))
        view.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                if (!request.isForMainFrame) return true
                if (!allowed(request.url)) { fail("JavaScript navigation left the crawl scope"); return true }
                return false
            }
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
                val host = request.url.host.orEmpty().lowercase()
                val media = Regex("\\.(mp4|webm|m4a|mp3|ogg|wav|m3u8|mpd)$", RegexOption.IGNORE_CASE).containsMatchIn(request.url.path.orEmpty()) || host == "googlevideo.com" || host.endsWith(".googlevideo.com") || request.url.path.orEmpty().startsWith("/embed/") && (host.endsWith("youtube.com") || host.endsWith("youtube-nocookie.com"))
                return if (media && !request.isForMainFrame) WebResourceResponse("text/plain", "utf-8", 403, "Media waits for approval", emptyMap(), ByteArrayInputStream(ByteArray(0))) else null
            }
            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) { if (request.isForMainFrame) fail("Browser load failed: ${error.description}") }
            override fun onReceivedHttpError(view: WebView, request: WebResourceRequest, response: WebResourceResponse) {
                if (response.statusCode == 429 || response.statusCode == 503) {
                    val retry = response.responseHeaders?.entries?.firstOrNull { it.key.equals("Retry-After", true) }?.value
                    val rate = JSONObject().put("url", request.url.toString()).put("status", response.statusCode).put("retry_after", retry ?: JSONObject.NULL)
                    succeed(JSONObject().put("url", request.url.toString()).put("html", "").put("rate_limit", rate))
                } else if (request.isForMainFrame) fail("HTTP ${response.statusCode} while loading the rendered page")
            }
            override fun onReceivedHttpAuthRequest(view: WebView, auth: HttpAuthHandler, host: String, realm: String) { auth.cancel(); fail("Authenticated browser captures are not supported") }
            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean { fail("Android stopped the page browser. Resume to retry from the checkpoint."); return true }
        }
        view.webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) { request.deny() }
            override fun onGeolocationPermissionsShowPrompt(origin: String, callback: GeolocationPermissions.Callback) { callback.invoke(origin, false, false) }
            override fun onJsAlert(view: WebView, url: String, message: String, result: JsResult): Boolean { result.cancel(); return true }
            override fun onJsConfirm(view: WebView, url: String, message: String, result: JsResult): Boolean { result.cancel(); return true }
            override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean { callback.onReceiveValue(null); return true }
        }
        val source = args.request.url
        if (!allowed(Uri.parse(source))) { fail("Invalid page capture URL"); return }
        view.loadUrl(source)
        handler.postDelayed(tick, 500)
        handler.postDelayed({ fail("Browser rendering timed out. Increase its timeout or retry.") }, (timeout + 5) * 1000)
    }
    private fun allowed(url: Uri): Boolean {
        if (url.scheme !in listOf("http", "https") || url.host == null || url.userInfo != null) return false
        for (i in scopes.indices) {
            val root = Uri.parse(scopes[i])
            val rootPort = if (root.port == -1) if (root.scheme == "https") 443 else 80 else root.port
            val port = if (url.port == -1) if (url.scheme == "https") 443 else 80 else url.port
            val path = root.path.orEmpty().ifEmpty { "/" }
            if (root.scheme == url.scheme && root.host == url.host && rootPort == port && (path == "/" || url.path == path || url.path.orEmpty().startsWith(path.trimEnd('/') + "/"))) return true
        }
        return false
    }
    private fun poll() {
        if (finished || evaluating) return
        evaluating = true
        browser?.evaluateJavascript("(()=>{try{return window.__zimsmithCapture?.poll()||null}catch(e){return {error:String(e)}}})()") { value ->
            evaluating = false
            if (finished || value == null || value == "null") return@evaluateJavascript
            try {
                val result = JSONObject(value)
                if (result.has("error")) { fail(result.getString("error")); return@evaluateJavascript }
                if (!result.has("html")) return@evaluateJavascript
                if (result.getString("html").toByteArray(Charsets.UTF_8).size > cap) { fail("Rendered HTML exceeds its capture limit"); return@evaluateJavascript }
                succeed(result)
            } catch (error: Exception) { fail("Could not capture rendered HTML: ${error.message}") }
        }
    }
    fun cancel() = fail("Page capture paused")
    fun abort(message: String) = fail(message)
    private fun cleanup() {
        handler.removeCallbacksAndMessages(null)
        browser?.let { it.stopLoading(); (it.parent as? ViewGroup)?.removeView(it); it.destroy() }
        browser = null
        try { if (WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) ProfileStore.getInstance().deleteProfile(profile) } catch (_: Exception) { /* Cleaned on next launch if Chromium is still releasing the view. */ }
        complete(id)
    }
    private fun fail(message: String) { if (finished) return; finished = true; cleanup(); invoke.reject(message) }
    private fun succeed(result: JSONObject) { if (finished) return; finished = true; cleanup(); invoke.resolve(JSObject(result.toString())) }
}
