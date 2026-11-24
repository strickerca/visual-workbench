@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.app.Instrumentation
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.CanvasSurface
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.SampleBatch
import com.visualworkbench.shared.StrokeOptions
import com.visualworkbench.shared.workbenchCore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.ceil
import kotlin.math.sin
import kotlin.math.sqrt

/** A stroke must cost the same on a page that holds two hundred strokes and on
 * one that holds five thousand. The same short run of pen strokes is drawn on
 * both, through the real app, its canvas view's own touch dispatch and a real
 * native project filled through the core, and what the app measured of its
 * own pen path is compared between the two:
 *
 * - the time to draw one ink frame, and from a pen sample to the frame that
 *   shows it being ready;
 * - from a stroke being saved to the editor having shown it, and the time to
 *   draw one saved frame and to have it ready (where the build measures them);
 * - measured here: from the pen lifting to the canvas having drawn the newest
 *   saved scene with nothing left in flight.
 *
 * The page of five thousand may take at most [FACTOR] times as long as the
 * page of two hundred, plus a small fixed allowance for timer resolution and
 * noise, named with each check. All numbers are in the failure message and,
 * when the test passes, in the instrumentation's output and the log.
 *
 * Synthetic pen events on one phone: this compares two pages on one build. It
 * says nothing about a pen's feel, and its absolute numbers belong to that
 * phone.
 *
 * To measure an older build, copy this one file into its tree and run it
 * there: it uses nothing but what the app has had since the pen timings were
 * introduced, and reads the timings from the text the diagnostics screen
 * copies, by name. On a build that draws every object for every frame the
 * comparison fails, and the failure message is the measurement. */
@RunWith(AndroidJUnit4::class)
class CanvasCostInstrumentedTest {
    private val instrumentation: Instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val context get() = instrumentation.targetContext
    private val core by lazy { workbenchCore() }

    private fun await(scenario: ActivityScenario<MainActivity>, phase: String, timeoutMs: Long = 45_000, predicate: (MainActivity) -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            var ready = false
            scenario.onActivity { ready = predicate(it) }
            if (ready) return
            SystemClock.sleep(25)
        }
        var detail = ""
        scenario.onActivity { activity ->
            val editor = activity.editor
            detail = "busy=${editor.busy} pending=${editor.pending} message=${editor.message} blocked=${editor.blockingMessage} " +
                "screen=${editor.screen} items=${editor.document?.render?.items?.size}"
        }
        fail("Timed out in $phase: $detail")
    }
    private fun canvas(view: View): CanvasSurface? {
        if (view is CanvasSurface) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) canvas(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun canvasReady(activity: MainActivity): Boolean {
        val editor = activity.editor
        return !editor.busy && editor.pending == 0 && !editor.instructionEditor.state.value.busy && !editor.semanticEditor.state.value.busy &&
            editor.scene?.background != null && canvas(activity.window.decorView)?.holder?.surface?.isValid == true
    }
    private fun projectPath(id: String): String {
        require(id.matches(Regex("[0-9a-fA-F-]{36}")))
        return File(File(context.filesDir, "projects"), id).canonicalPath
    }

    /** Puts [count] pen strokes into the closed project through the core,
     * spread evenly over the page and all of one shape, so that every one of
     * them is drawn when the whole page is. The band the measured strokes are
     * drawn in ([BAND_TOP] to [BAND_BOTTOM] of the page's height) is left
     * empty on both pages: the pen's loupe draws the page around the nib
     * magnified, and what it costs follows how many strokes lie there, which
     * is then the same on both pages. What is compared is what the rest of
     * the page costs. */
    private fun fillPage(id: String, count: Int): Unit = runBlocking {
        val project = core.open(projectPath(id))
        try {
            var info = project.info()
            val documentId = info.documentIds.first()
            val document = project.document(documentId)
            val layer = document.layers.first { it.visible && !it.locked && it.blend == "normal" }
            val w = document.width.toDouble(); val h = document.height.toDouble()
            val columns = ceil(sqrt(count * w / h)).toInt().coerceAtLeast(1)
            val rows = ceil(count.toDouble() / columns).toInt().coerceAtLeast(1)
            repeat(count) { index ->
                val now = System.currentTimeMillis()
                val stroke = project.beginStroke(StrokeOptions(core.newId(now.toULong()), core.newId(now.toULong()), core.newId(now.toULong()),
                    documentId, layer.id, info.deviceId, info.nextLamport, now, "pen", 3.0, 0x1b2a3affu))
                try {
                    val x = 20.0 + (w - 80.0) * (index % columns) / columns
                    // Rows over the page's height without the band, in page order.
                    val above = h * BAND_TOP - 40.0; val below = h * (1 - BAND_BOTTOM) - 60.0
                    val along = (above + below) * (index / columns) / rows
                    val y = if (along < above) 20.0 + along else h * BAND_BOTTOM + 20.0 + (along - above)
                    val samples = 12
                    stroke.append(SampleBatch(1uL, DoubleArray(samples) { x + 3.2 * it }, DoubleArray(samples) { y + 6.0 + 6.0 * sin(it * 0.9 + index) },
                        UIntArray(samples) { (it * 8).toUInt() }, FloatArray(samples) { .35f + it * .04f }))
                    info = stroke.commit()
                } finally { stroke.dispose() }
            }
        } finally { project.close() }
    }

    /** One timing as the diagnostics text gives it, in milliseconds. */
    private class Timing(val count: Int, val p50: Double, val p95: Double, val max: Double) {
        override fun toString() = "n=$count p50=$p50 p95=$p95 max=$max"
    }
    private fun timings(report: String): Map<String, Timing> {
        val row = Regex("([a-z_]+) n=([0-9]+) p50_ms=([0-9.]+) p95_ms=([0-9.]+) max_ms=([0-9.]+)")
        return report.lines().mapNotNull { row.matchEntire(it.trim()) }.associate { match ->
            match.groupValues[1] to Timing(match.groupValues[2].toInt(), match.groupValues[3].toDouble(), match.groupValues[4].toDouble(), match.groupValues[5].toDouble())
        }
    }

    /** What one page cost. [shownMs]: for each stroke, from the pen lifting to
     * the canvas having drawn the newest saved scene with nothing in flight. */
    private class Cost(val page: Int, val strokes: Int, val timings: Map<String, Timing>, val shownMs: List<Long>, val report: String) {
        val shownMedian: Double get() = shownMs.sorted().let { if (it.isEmpty()) Double.NaN else it[it.size / 2].toDouble() }
        val shownWorst: Long get() = shownMs.maxOrNull() ?: -1
        fun p50(key: String): Double? = timings[key]?.p50
        override fun toString(): String = "page of $page strokes, $strokes drawn: pen-up to saved scene drawn median=${shownMedian} ms worst=$shownWorst ms (n=${shownMs.size}); " +
            KEYS.joinToString("; ") { key -> "$key ${timings[key] ?: "not measured by this build"}" }
    }

    /** Whether the canvas has drawn the newest scene it was given as a saved
     * frame and has no frame asked for or in flight, by its own account. */
    private fun savedSceneDrawn(state: String): Boolean {
        fun number(name: String): Long? = Regex(" $name=([0-9]+)").find(state)?.groupValues?.get(1)?.toLong()
        val drawn = number("current_drawn_rev"); val newest = number("current_newest_rev"); val flying = number("current_in_flight")
        return drawn != null && drawn == newest && flying == 0L && state.contains(" current_request_pending=false")
    }

    /** Draws [count] strokes, each [strokeMs] long and [gapMs] apart, on the
     * open page and measures each. Every event is posted for its own moment
     * before the first is due; after each pen-up the main thread looks once a
     * millisecond for the stroke to be saved and the saved scene to be drawn. */
    private fun drawAndMeasure(scenario: ActivityScenario<MainActivity>, count: Int, strokeMs: Long, gapMs: Long, expectedBefore: Int): List<Long> {
        val done = CountDownLatch(count)
        val shown = java.util.Collections.synchronizedList(ArrayList<Long>())
        val steps = (strokeMs / 8).toInt().coerceAtLeast(2)
        scenario.onActivity { activity ->
            val editor = activity.editor; editor.chooseTool(EditorTool.Pen)
            val surface = requireNotNull(canvas(activity.window.decorView))
            val scene = requireNotNull(editor.scene)
            val w = scene.document.width.toDouble(); val h = scene.document.height.toDouble()
            val main = Handler(Looper.getMainLooper())
            val first = SystemClock.uptimeMillis() + 500
            for (stroke in 0 until count) {
                val from = Point(w * (0.08 + 0.03 * (stroke % 24)), h * (0.3 + 0.1 * (stroke / 24)))
                val down = first + stroke * (strokeMs + gapMs)
                for (step in 0..steps) {
                    val time = down + strokeMs * step / steps
                    val screen = EditorDrawing.map(scene.matrix, Point(from.x + w * 0.05 * step / steps, from.y + h * 0.12 * step / steps))
                    val action = when (step) { 0 -> MotionEvent.ACTION_DOWN; steps -> MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                    main.postAtTime({
                        val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_STYLUS })
                        val coordinates = arrayOf(MotionEvent.PointerCoords().apply {
                            x = screen.x.toFloat(); y = screen.y.toFloat(); pressure = .4f + .4f * step / steps; size = .01f })
                        val event = MotionEvent.obtain(down, time, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
                        try { surface.dispatchTouchEvent(event) } finally { event.recycle() }
                        if (step == steps) {
                            val lifted = SystemClock.uptimeMillis()
                            val expected = expectedBefore + stroke + 1
                            val look = object : Runnable {
                                override fun run() {
                                    val now = SystemClock.uptimeMillis()
                                    val saved = editor.pending == 0 && editor.document?.render?.items?.size == expected
                                    if (saved && savedSceneDrawn(surface.lifeState())) { shown.add(now - lifted); done.countDown() }
                                    // A stroke that was refused, or a page that never settles, ends the wait unmeasured.
                                    else if (now - lifted > gapMs - 20) done.countDown()
                                    else main.postDelayed(this, 1)
                                }
                            }
                            main.post(look)
                        }
                    }, time)
                }
            }
        }
        val allowed = count * (strokeMs + gapMs) + 60_000
        assertTrue("The strokes were not all drawn and looked for within $allowed ms", done.await(allowed, TimeUnit.MILLISECONDS))
        return shown.toList()
    }

    /** A new blank page with [filled] strokes put into it through the core,
     * then the run of strokes on it, measured. The project is deleted. */
    private fun cost(filled: Int): Cost {
        var owned: String? = null
        var storageClosed = false
        var result: Cost? = null
        try {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                await(scenario, "startup") { !it.editor.busy }
                // Held upright whichever way the phone lies, so that both pages are measured on the same canvas.
                scenario.onActivity { it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
                SystemClock.sleep(500)
                await(scenario, "upright") { !it.editor.busy && it.resources.configuration.orientation == Configuration.ORIENTATION_PORTRAIT }
                scenario.onActivity { it.editor.newCanvas() }
                await(scenario, "new-page", predicate = ::canvasReady)
                scenario.onActivity { owned = requireNotNull(it.editor.info).projectId; assertNull(it.editor.blockingMessage) }
                val id = requireNotNull(owned)
                try {
                    scenario.onActivity { it.editor.navigate(WorkbenchScreen.Projects) }
                    await(scenario, "closed-to-fill") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                    fillPage(id, filled)
                    scenario.onActivity { it.editor.open(id) }
                    await(scenario, "filled-page", timeoutMs = 240_000, predicate = ::canvasReady)
                    scenario.onActivity { assertEquals(filled, it.editor.document?.render?.items?.size); it.editor.chooseTool(EditorTool.Pen) }
                    SystemClock.sleep(1_000)
                    await(scenario, "pen-chosen", predicate = ::canvasReady)
                    // A few strokes first, unmeasured: the first of a session pays for things later ones do not.
                    drawAndMeasure(scenario, WARM_UP, STROKE_MS, GAP_MS, filled)
                    await(scenario, "warm-up-saved", timeoutMs = 120_000) { it.editor.pending == 0 && !it.editor.busy }
                    var before = 0
                    scenario.onActivity { before = it.editor.document?.render?.items?.size ?: -1; it.editor.diagnostics.pen.clear() }
                    assertEquals("Not every warm-up stroke was saved", filled + WARM_UP, before)
                    val shown = drawAndMeasure(scenario, STROKES, STROKE_MS, GAP_MS, before)
                    await(scenario, "strokes-saved", timeoutMs = 120_000) { it.editor.pending == 0 && !it.editor.busy }
                    var report = ""; var after = 0
                    scenario.onActivity { report = it.editor.diagnostics.pen.report(emptyList()); after = it.editor.document?.render?.items?.size ?: -1 }
                    result = Cost(filled, after - before, timings(report), shown, report)
                } finally {
                    scenario.onActivity { it.editor.cancelTool(); it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED }
                    await(scenario, "saved-to-close", timeoutMs = 180_000) { it.editor.pending == 0 && !it.editor.busy }
                    scenario.onActivity { if (it.editor.screen != WorkbenchScreen.Projects) it.editor.navigate(WorkbenchScreen.Projects) }
                    await(scenario, "page-closed", timeoutMs = 180_000) { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                    storageClosed = true
                }
            }
        } finally {
            owned?.let { id ->
                val target = File(projectPath(id))
                if (storageClosed) assertTrue(!target.exists() || target.deleteRecursively())
            }
        }
        return requireNotNull(result)
    }

    @Test fun aStrokeCostsTheSameOnAPageOfTwoHundredStrokesAndOnOneOfFiveThousand() {
        val light = cost(LIGHT)
        val busy = cost(BUSY)
        val numbers = "\n$light\n$busy\n"
        Log.i(TAG, numbers)
        instrumentation.sendStatus(0, Bundle().apply { putString(Instrumentation.REPORT_KEY_STREAMRESULT, numbers) })
        assertEquals("Not every stroke was saved on the page of $LIGHT.$numbers", STROKES, light.strokes)
        assertEquals("Not every stroke was saved on the page of $BUSY.$numbers", STROKES, busy.strokes)
        assertTrue("Too few strokes were seen to settle to compare the pages.$numbers", light.shownMs.size >= STROKES * 3 / 4 && busy.shownMs.size >= STROKES * 3 / 4)

        /** The page of five thousand may take [FACTOR] times the page of two hundred, plus [allowance] ms. */
        fun within(what: String, lightMs: Double?, busyMs: Double?, allowance: Double, required: Boolean = true) {
            if (lightMs == null || busyMs == null) {
                if (required) fail("This build did not measure: $what.$numbers")
                return
            }
            assertTrue("$what grows with the page: $busyMs ms with $BUSY strokes on it, $lightMs ms with $LIGHT " +
                "(allowed: $FACTOR times as long plus $allowance ms).$numbers", busyMs <= lightMs * FACTOR + allowance)
        }
        within("The time to draw one ink frame (canvas_draw_time, median)", light.p50("canvas_draw_time"), busy.p50("canvas_draw_time"), 0.3)
        within("From a pen sample to its frame being ready (canvas_frame_ready, median)", light.p50("canvas_frame_ready"), busy.p50("canvas_frame_ready"), 2.0)
        within("From a stroke being saved to the editor having shown it (canvas_refresh, median)", light.p50("canvas_refresh"), busy.p50("canvas_refresh"), 2.0)
        within("From the pen lifting to the saved scene being drawn (median)", light.shownMedian, busy.shownMedian, 5.0)
        // Timings this build added; an older build does not have them.
        within("The time to draw one saved frame (canvas_saved_draw, median)", light.p50("canvas_saved_draw"), busy.p50("canvas_saved_draw"), 0.3, required = false)
        within("From a saved frame being drawn to its being ready (canvas_saved_ready, median)", light.p50("canvas_saved_ready"), busy.p50("canvas_saved_ready"), 2.0, required = false)
    }

    private companion object {
        const val TAG = "VW_CANVAS_COST"
        const val LIGHT = 200
        const val BUSY = 5_000
        const val WARM_UP = 4
        const val STROKES = 24
        const val STROKE_MS = 64L
        const val GAP_MS = 450L
        const val FACTOR = 2.0
        /** The band of the page, as fractions of its height, that the measured strokes are drawn in. */
        const val BAND_TOP = 0.2
        const val BAND_BOTTOM = 0.7
        val KEYS = listOf("canvas_draw_time", "canvas_frame_ready", "canvas_ink_drawn", "canvas_first_ink", "canvas_save", "canvas_refresh",
            "canvas_saved_draw", "canvas_saved_ready")
    }
}
