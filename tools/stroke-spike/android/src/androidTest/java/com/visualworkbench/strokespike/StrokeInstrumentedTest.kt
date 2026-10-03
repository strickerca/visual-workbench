package com.visualworkbench.strokespike

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.os.SystemClock
import android.view.MotionEvent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.AfterClass
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class StrokeInstrumentedTest {
    companion object {
        private val results = JSONArray()
        private var callbacks = JSONObject()
        @JvmStatic @AfterClass fun writeReport() {
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            val runId = InstrumentationRegistry.getArguments().getString("runId") ?: error("Missing run binding")
            require(Regex("[0-9a-f]{32}").matches(runId))
            val report = JSONObject().put("schema", 1).put("run_id", runId)
                .put("complete", results.length() == 3).put("hardware_pen_acceptance", false)
                .put("verification_image_files", 0).put("results", results).put("callbacks", callbacks)
            instrumentation.targetContext.openFileOutput("stroke-report.json", 0).use { it.write(report.toString().toByteArray()) }
        }
        private fun passed(name: String) { results.put(JSONObject().put("name", name).put("passed", true)) }
    }

    @Test fun nativeGeometryIsStableAndPredictionIsDisposable() {
        assertEquals(0, NativeInk.liveHandles())
        assertEquals(0L, NativeInk.begin(99, 12.0, 0.0))
        val handle = NativeInk.begin(0, 12.0, 0.25)
        assertTrue(handle > 0)
        try {
            for (i in 0..31) assertTrue(NativeInk.append(handle, 20.0 + i * 6, 60.0 + (i % 5) * 4, i * 8L, 0.2 + i / 40.0) >= 0)
            val wetHash = NativeInk.hash(handle)
            val preview = NativeInk.preview(handle, 240.0, 80.0, 264, 0.9)
            assertTrue(preview > 0)
            try { assertNotEquals(wetHash, NativeInk.hash(preview)) }
            finally { assertEquals(0, NativeInk.release(preview)) }
            assertEquals(wetHash, NativeInk.hash(handle))
            val wet = Path().also { NativeInk.appendContours(it, handle) }
            assertEquals(0, NativeInk.finish(handle))
            val dry = Path().also { NativeInk.appendContours(it, handle) }
            assertEquals(wetHash, NativeInk.hash(handle))
            fun pixels(path: Path): IntArray {
                val bitmap = Bitmap.createBitmap(256, 128, Bitmap.Config.ARGB_8888)
                return try {
                    Canvas(bitmap).drawPath(path, Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.BLACK; style = Paint.Style.FILL })
                    IntArray(256 * 128).also { bitmap.getPixels(it, 0, 256, 0, 0, 256, 128) }
                } finally { bitmap.recycle() }
            }
            val wetPixels = pixels(wet)
            assertTrue(wetPixels.any { it != 0 })
            assertArrayEquals(wetPixels, pixels(dry))
            assertEquals(-1, NativeInk.append(handle, 250.0, 80.0, 272, 0.9))
        } finally { assertEquals(0, NativeInk.release(handle)) }
        assertEquals(-1, NativeInk.release(handle))
        assertTrue(NativeInk.coordinate(handle, 0, 0, 0).isNaN())
        assertEquals(0, NativeInk.liveHandles())
        passed("native_prediction_isolated_wet_dry_32768_pixels_identical")
    }

    private fun event(down: Long, action: Int, index: Int): MotionEvent = MotionEvent.obtain(
        down, SystemClock.uptimeMillis(), action, 60f + index * 8f, 100f + index * 2f, 0.6f,
        1f, 0, 1f, 1f, 0, 0)

    private fun awaitJetpackReady(scenario: ActivityScenario<StrokeActivity>) {
        val deadline = SystemClock.uptimeMillis() + 5_000
        var ready = false
        while (!ready && SystemClock.uptimeMillis() < deadline) {
            scenario.onActivity { ready = it.hasWindowFocus() && it.jetpack.readyForInput }
            if (!ready) SystemClock.sleep(10)
        }
        assertTrue("Jetpack canvas did not become drawn and focused", ready)
    }

    @Test fun bothEnginesReceiveAndFinishSyntheticInput() {
        ActivityScenario.launch(StrokeActivity::class.java).use { scenario ->
            for (rust in listOf(true, false)) {
                scenario.onActivity { it.showSurface(if (rust) it.rust else it.jetpack) }
                if (!rust) awaitJetpackReady(scenario)
                val down = SystemClock.uptimeMillis()
                for (index in 0..16) {
                    scenario.onActivity { activity ->
                        check(activity.hasWindowFocus()) { "INCONCLUSIVE: comparison app lost foreground" }
                        val action = when (index) { 0 -> MotionEvent.ACTION_DOWN; 16 -> MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                        val event = event(down, action, index)
                        try { (if (rust) activity.rust else activity.jetpack).dispatchTouchEvent(event) }
                        finally { event.recycle() }
                    }
                    SystemClock.sleep(8)
                }
                val limit = SystemClock.uptimeMillis() + 5_000
                var complete = false
                while (!complete && SystemClock.uptimeMillis() < limit) {
                    scenario.onActivity {
                        assertNull(if (rust) it.rust.lastFailure else it.jetpack.lastFailure)
                        complete = if (rust) it.rust.committedCount == 1 &&
                            it.metrics.observed("rust", "front_buffer_draw_callback") &&
                            it.metrics.observed("rust", "multi_buffer_draw_callback")
                        else it.jetpack.committedCount == 1 && it.metrics.observed("jetpack", "finished_stroke_callback")
                    }
                    if (!complete) SystemClock.sleep(25)
                }
                assertTrue("Finished-stroke callback timed out", complete)
            }
            scenario.onActivity {
                assertEquals(it.rust.lastWetHash, it.rust.lastDryHash)
                callbacks = it.metrics.report()
                val phases = callbacks.getJSONArray("records")
                val observed = (0 until phases.length()).map { i -> phases.getJSONObject(i).getString("engine") }.toSet()
                assertEquals(setOf("rust", "jetpack"), observed)
            }
        }
        assertEquals(0, NativeInk.liveHandles())
        passed("both_engines_synthetic_motion_dispatch_and_completion")
    }

    @Test fun cancellationAndActivityLifecycleReleaseNativeStroke() {
        ActivityScenario.launch(StrokeActivity::class.java).use { scenario ->
            scenario.onActivity {
                it.showSurface(it.rust)
                val down = event(SystemClock.uptimeMillis(), MotionEvent.ACTION_DOWN, 0)
                try { it.rust.dispatchTouchEvent(down) } finally { down.recycle() }
                assertEquals(1, NativeInk.liveHandles())
                val cancel = event(SystemClock.uptimeMillis(), MotionEvent.ACTION_CANCEL, 0)
                try { it.rust.dispatchTouchEvent(cancel) } finally { cancel.recycle() }
                assertEquals(0, NativeInk.liveHandles())
                assertEquals(0, it.rust.committedCount)
                val second = event(SystemClock.uptimeMillis(), MotionEvent.ACTION_DOWN, 0)
                try { it.rust.dispatchTouchEvent(second) } finally { second.recycle() }
                assertEquals(1, NativeInk.liveHandles())
            }
            scenario.onActivity {
                it.showSurface(it.jetpack)
                assertEquals(0, NativeInk.liveHandles())
            }
            awaitJetpackReady(scenario)
            scenario.onActivity {
                val time = SystemClock.uptimeMillis()
                for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
                    val event = event(time, action, if (action == MotionEvent.ACTION_DOWN) 0 else 1)
                    try { it.jetpack.dispatchTouchEvent(event) } finally { event.recycle() }
                }
                it.jetpack.clearInk()
            }
            val limit = SystemClock.uptimeMillis() + 5_000
            var pending = true
            while (pending && SystemClock.uptimeMillis() < limit) {
                scenario.onActivity {
                    assertNull(it.jetpack.lastFailure)
                    assertEquals(0, it.jetpack.committedCount)
                    assertEquals(0, it.jetpack.visibleFinishedCount)
                    pending = it.jetpack.pendingFinishedCount > 0
                }
                if (pending) SystemClock.sleep(25)
            }
            assertFalse("Cleared Jetpack stroke callback did not drain", pending)
        }
        assertEquals(0, NativeInk.liveHandles())
        passed("cancel_switch_and_late_jetpack_clear_without_commit")
    }
}
