package org.zimstudio.files

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.FrameLayout
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.tauri.plugin.Invoke
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

class CaptureTestActivity : Activity() {
    override fun onCreate(state: Bundle?) { super.onCreate(state); setContentView(FrameLayout(this)) }
}

@RunWith(AndroidJUnit4::class)
class PageCaptureTest {
    private fun capture(path: String): Pair<Long, JSONObject> {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val origin = InstrumentationRegistry.getArguments().getString("fixtureUrl") ?: "http://10.0.2.2:9786"
        val script = instrumentation.context.assets.open("render.js").bufferedReader().use { it.readText() }
        val completed = CountDownLatch(1)
        val output = AtomicReference<Pair<Long, String>>()
        val mapper = Class.forName("com.fasterxml.jackson.databind.ObjectMapper").getDeclaredConstructor().newInstance()
        val callback: (Long, String) -> Unit = { code, value -> output.set(Pair(code, value)); completed.countDown() }
        val invoke = Invoke::class.java.constructors.first().newInstance(1L, "renderPage", 1L, 2L, callback, "{}", mapper) as Invoke
        val intent = Intent(instrumentation.context, CaptureTestActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        ActivityScenario.launch<CaptureTestActivity>(intent).use { scenario ->
            scenario.onActivity { activity ->
                val args = RenderPageArgs().apply {
                    this.script = script
                    request = BrowserRenderRequest().apply { id = UUID.randomUUID().toString(); url = origin + path; delay_ms = 1000; timeout_secs = 20; max_html_bytes = 8388608; scopes = listOf(origin + "/") }
                }
                PageCapture(activity, args, invoke) {}.start()
            }
            assertTrue("Native browser callback did not finish", completed.await(30, TimeUnit.SECONDS))
        }
        val value = output.get(); return Pair(value.first, JSONObject(value.second))
    }
    @Test fun rendersARealReactAppAndAggregatesAccessiblePagination() {
        val (code, result) = capture("/pages")
        assertEquals(result.toString(), 1L, code)
        val html = result.getString("html")
        for (name in listOf("Orchid", "Fern", "Moss", "Oak", "Pine", "Cedar")) assertTrue(name, html.contains(name))
        assertFalse(html.contains("<script"))
        assertTrue(result.getBoolean("settled"))
    }
    @Test fun reportsMainPageRateLimitsWithRetryAfter() {
        val (code, result) = capture("/limited")
        assertEquals(result.toString(), 1L, code)
        assertEquals(429, result.getJSONObject("rate_limit").getInt("status"))
        assertEquals("7", result.getJSONObject("rate_limit").getString("retry_after"))
    }
}
