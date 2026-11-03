package org.zimstudio.files

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** Private durable inbox. Each share is an atomic file, not an Activity intent to replay. */
internal object SharedLinks {
    private fun directory(context: Context): File = File(context.filesDir, "shared-links").apply { mkdirs() }
    @Synchronized fun receive(context: Context, text: String, title: String) {
        require(text.toByteArray(Charsets.UTF_8).size <= 32768) { "Share a single website link." }
        require(Regex("https?://", RegexOption.IGNORE_CASE).containsMatchIn(text)) { "Share a URL from your browser." }
        val directory = directory(context)
        // Android may redeliver an intent while restoring an Activity. Keep one
        // pending copy, while allowing it to be shared again after use/dismissal.
        if (directory.listFiles { f -> f.extension == "json" }!!.any { file ->
            try { file.length() <= 65536 && JSONObject(file.readText()).optString("text") == text }
            catch (_: Exception) { false }
        }) return
        require(directory.listFiles { f -> f.extension == "json" }!!.size < 50) { "Open Zimsmith and use or dismiss your saved links first." }
        val id = UUID.randomUUID().toString()
        val value = JSONObject().put("id", id).put("text", text).put("title", title.take(180)).put("created_at", System.currentTimeMillis())
        val temporary = File(directory, "$id.tmp")
        temporary.outputStream().use { stream -> stream.write(value.toString().toByteArray(Charsets.UTF_8)); stream.fd.sync() }
        check(temporary.renameTo(File(directory, "$id.json"))) { "Could not save the shared link." }
    }
    @Synchronized fun list(context: Context): JSONArray {
        val result = JSONArray()
        directory(context).listFiles { f -> f.extension == "json" }!!.sortedBy { it.lastModified() }.take(50).forEach { file ->
            if (file.length() <= 65536) {
                try { result.put(JSONObject(file.readText())) }
                catch (_: Exception) { result.put(JSONObject().put("id", file.nameWithoutExtension).put("text", "").put("title", "Unusable shared link")) }
            }
        }
        return result
    }
    @Synchronized fun dismiss(context: Context, id: String) {
        require(UUID.fromString(id).toString() == id) { "Invalid shared-link identifier." }
        val file = File(directory(context), "$id.json")
        check(!file.exists() || file.delete()) { "Could not remove the shared link." }
    }
}

/** Manifest receiver also handles cold launch without modifying generated MainActivity. */
class ShareLinkActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        receiveAndOpen(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        receiveAndOpen(intent)
    }

    private fun receiveAndOpen(incoming: Intent) {
        try {
            if (incoming.action == Intent.ACTION_SEND && incoming.type in listOf("text/plain", "text/uri-list")) {
                val text = incoming.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
                    ?: incoming.clipData?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString()
                    ?: throw IllegalArgumentException("Share a URL from your browser.")
                SharedLinks.receive(this, text, incoming.getStringExtra(Intent.EXTRA_SUBJECT).orEmpty())
            }
            val launch = packageManager.getLaunchIntentForPackage(packageName)
                ?: throw IllegalStateException("Could not open Zimsmith.")
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            startActivity(launch)
        } catch (error: Exception) {
            Toast.makeText(this, error.message ?: "Could not receive the shared link.", Toast.LENGTH_LONG).show()
        }
        finish()
    }
}
