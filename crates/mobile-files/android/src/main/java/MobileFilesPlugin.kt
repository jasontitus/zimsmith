package org.zimstudio.files

import android.os.Build
import android.net.Uri
import android.content.pm.PackageManager
import org.json.JSONArray
import java.security.MessageDigest
import android.app.Activity
import android.content.Intent
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.webkit.WebView
import androidx.core.content.FileProvider
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import app.tauri.plugin.Invoke
import java.io.File
import kotlin.concurrent.thread

@InvokeArg
class SaveArgs {
    lateinit var path: String
    lateinit var filename: String
}

@InvokeArg
class OpenArgs {
    lateinit var path: String
}

@InvokeArg
class WebUrlArgs { lateinit var url: String }

@InvokeArg
class SharedLinkArgs { lateinit var id: String }

class ZimFileProvider : FileProvider()

@TauriPlugin
class MobileFilesPlugin(private val activity: Activity): Plugin(activity) {
    @Command
    fun sharedLinks(invoke: Invoke) {
        try { invoke.resolve(JSObject().put("items", SharedLinks.list(activity))) }
        catch (_: Exception) { invoke.reject("Could not read your shared links.") }
    }

    @Command
    fun dismissSharedLink(invoke: Invoke) {
        try {
            SharedLinks.dismiss(activity, invoke.parseArgs(SharedLinkArgs::class.java).id)
            invoke.resolve(JSObject().put("dismissed", true))
        } catch (error: Exception) { invoke.reject(error.message ?: "Could not dismiss the shared link.") }
    }
    override fun load(webView: WebView) {
        webView.settings.setSupportZoom(false)
        webView.settings.builtInZoomControls = false
        webView.settings.displayZoomControls = false
    }

    @Command
    @Suppress("DEPRECATION")
    fun appInfo(invoke: Invoke) {
        try {
            val flags = if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
            val info = activity.packageManager.getPackageInfo(activity.packageName, flags)
            val signatures = if (Build.VERSION.SDK_INT >= 28) info.signingInfo?.apkContentsSigners else info.signatures
            val signature = signatures?.singleOrNull() ?: throw IllegalStateException("Expected one app signing certificate")
            val digest = MessageDigest.getInstance("SHA-256").digest(signature.toByteArray()).joinToString("") { "%02x".format(it.toInt() and 255) }
            invoke.resolve(JSObject().apply {
                put("package_id", activity.packageName)
                put("version", info.versionName)
                put("version_code", if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong())
                put("sdk", Build.VERSION.SDK_INT)
                put("abis", JSONArray(Build.SUPPORTED_ABIS.toList()))
                put("certificate_sha256", digest)
            })
        } catch (error: Exception) { invoke.reject("Could not read app identity: ${error.message}") }
    }

    @Command
    fun openWebUrl(invoke: Invoke) {
        val url = invoke.parseArgs(WebUrlArgs::class.java).url
        val uri = Uri.parse(url)
        if (uri.scheme != "https" || uri.host.isNullOrEmpty() || uri.userInfo != null || uri.fragment != null) {
            invoke.reject("Update downloads require an HTTPS address")
            return
        }
        activity.runOnUiThread {
            try {
                activity.startActivity(Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE))
                invoke.resolve(JSObject().put("opened", true))
            } catch (error: Exception) { invoke.reject("Could not open your browser: ${error.message}") }
        }
    }

    @Command
    fun open(invoke: Invoke) {
        val args = invoke.parseArgs(OpenArgs::class.java)
        val file = File(args.path).canonicalFile
        if (!file.isFile || file.extension.lowercase() != "zim" || !file.path.startsWith(File(activity.applicationInfo.dataDir).canonicalPath + File.separator)) {
            invoke.reject("Archive is outside the app's storage or no longer exists")
            return
        }
        activity.runOnUiThread {
            try {
                val uri = FileProvider.getUriForFile(activity, activity.packageName + ".zim-files", file)
                val intent = Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(uri, "application/x-zim")
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    clipData = ClipData.newRawUri(file.name, uri)
                }
                activity.startActivity(intent)
                invoke.resolve(JSObject().put("opened", true))
            } catch (error: ActivityNotFoundException) {
                invoke.reject("No ZIM viewer is installed. Install a compatible viewer, then tap Open ZIM again.")
            } catch (error: Exception) {
                invoke.reject("Could not open archive: ${error.message}")
            }
        }
    }

    @Command
    fun save(invoke: Invoke) {
        val args = invoke.parseArgs(SaveArgs::class.java)
        val file = File(args.path).canonicalFile
        if (!file.isFile || !file.path.startsWith(File(activity.applicationInfo.dataDir).canonicalPath + File.separator)) {
            invoke.reject("Archive is outside the app's storage")
            return
        }
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/x-zim"
            putExtra(Intent.EXTRA_TITLE, args.filename)
        }
        startActivityForResult(invoke, intent, "savedDocument")
    }

    @ActivityCallback
    private fun savedDocument(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject().put("saved", false))
            return
        }
        val args = invoke.parseArgs(SaveArgs::class.java)
        thread(name = "zim-export") {
            try {
                activity.contentResolver.openOutputStream(uri, "w")!!.use { output ->
                    File(args.path).inputStream().use { input -> input.copyTo(output, 65536) }
                }
                invoke.resolve(JSObject().put("saved", true))
            } catch (error: Exception) {
                invoke.reject("Could not save archive: ${error.message}")
            }
        }
    }
}
