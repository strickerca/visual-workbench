@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.app.Instrumentation
import android.content.ComponentCallbacks2
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.BrushPreference
import com.visualworkbench.android.editor.CanvasSurface
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.editor.EditorPreferences
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.android.editor.PictureComparison
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.SampleBatch
import com.visualworkbench.shared.Shape
import com.visualworkbench.shared.StrokeOptions
import com.visualworkbench.shared.workbenchCore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.abs
import kotlin.math.sin

/** The canvas draws its frames from a kept picture of the saved page, and a
 * finished stroke is drawn into that picture once. These tests hold it to the
 * whole-scene drawing it replaces, on the real surface of the real app with a
 * real native project.
 *
 * A comparison asks the canvas, on its drawing thread and for the very scene
 * it has just put on screen, to draw the frame twice off screen with the
 * renderer that draws the canvas: once as it was drawn, from the kept picture
 * the session holds at that moment, and once whole from every object. The two
 * are compared pixel by pixel. A frame that follows a picture made from every
 * object must be the whole scene exactly. Once strokes have been added to the
 * picture, the edge pixels of those strokes may differ by rounding: a whole
 * scene blends a stroke into its layer and the layer into the page, the
 * picture blends the stroke into the page at once. [ADDED_BOUND] is the most
 * any colour channel of any pixel may differ; the differences found are
 * written to the log and into the instrumentation's output.
 *
 * The tests also read how often a picture was made from every object and how
 * often a stroke was added to one, so that "drawn into it once" and "made
 * again when it would be wrong" are counted, not assumed; and they look at
 * the composited screen after a pen-up for the stroke that was just drawn.
 *
 * Synthetic pen events only: nothing here judges a pen's feel. */
@RunWith(AndroidJUnit4::class)
class KeptPictureInstrumentedTest {
    private val instrumentation: Instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val context get() = instrumentation.targetContext
    private val core by lazy { workbenchCore() }
    private val notes = StringBuilder()

    private fun note(line: String) { Log.i(TAG, line); notes.append(line).append('\n') }

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
                "screen=${editor.screen} items=${editor.document?.render?.items?.size} canvas=${canvas(activity.window.decorView)?.lifeState()}"
        }
        fail("Timed out in $phase: $detail\n$notes")
    }
    private fun canvas(view: View): CanvasSurface? {
        if (view is CanvasSurface) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) canvas(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun settled(activity: MainActivity): Boolean {
        val editor = activity.editor
        return !editor.busy && editor.pending == 0 && !editor.instructionEditor.state.value.busy && !editor.semanticEditor.state.value.busy &&
            editor.scene?.background != null && canvas(activity.window.decorView)?.holder?.surface?.isValid == true
    }
    private fun projectPath(id: String): String {
        require(id.matches(Regex("[0-9a-fA-F-]{36}")))
        return File(File(context.filesDir, "projects"), id).canonicalPath
    }

    /** Puts a varied page into the closed project through the core: pen and
     * marker strokes in several colours (one of them translucent) and widths,
     * laid so that they cross one another, some of them over the page's edge.
     * Returns how many. */
    private fun fillPage(id: String): Int = runBlocking {
        val project = core.open(projectPath(id))
        try {
            var info = project.info()
            val documentId = info.documentIds.first()
            val document = project.document(documentId)
            val layer = document.layers.first { it.visible && !it.locked && it.blend == "normal" }
            val w = document.width.toDouble(); val h = document.height.toDouble()
            val colours = listOf(0x1b2a3affu, 0xc0392bffu, 0x1f7a4dffu, 0x2456c8ffu, 0x8e44adb0u, 0xe08a00ffu)
            val widths = listOf(2.0, 5.0, 11.0, 18.0)
            val count = 36
            repeat(count) { index ->
                val now = System.currentTimeMillis()
                val family = if (index % 3 == 2) "marker" else "pen"
                val stroke = project.beginStroke(StrokeOptions(core.newId(now.toULong()), core.newId(now.toULong()), core.newId(now.toULong()),
                    documentId, layer.id, info.deviceId, info.nextLamport, now, family, widths[index % widths.size], colours[index % colours.size]))
                try {
                    // Every ninth starts left of the page, every ninth runs out on the right.
                    val x0 = if (index % 9 == 0) -0.06 * w else w * (0.08 + (index % 6) * 0.13)
                    val y0 = h * (0.12 + (index / 6) * 0.12)
                    val reach = if (index % 9 == 4) 0.5 * w else 0.3 * w
                    val samples = 24
                    stroke.append(SampleBatch(1uL, DoubleArray(samples) { x0 + reach * it / (samples - 1) },
                        DoubleArray(samples) { y0 + 0.07 * h * sin(it * 0.55 + index) + 0.04 * h * it / (samples - 1) },
                        UIntArray(samples) { (it * 8).toUInt() }, FloatArray(samples) { .3f + .5f * abs(sin(it * 0.4f)) }))
                    info = stroke.commit()
                } finally { stroke.dispose() }
            }
            count
        } finally { project.close() }
    }

    /** What a comparison found. [inked]: pixels of the whole scene that are
     * neither the page nor the surround, so that two empty frames are never
     * taken for a match. */
    private class Difference(val width: Int, val height: Int, val differing: Int, val overOne: Int, val worst: Int, val inked: Int) {
        override fun toString() = "size=${width}x$height differing=$differing over_one=$overOne worst=$worst inked=$inked"
    }

    private fun difference(compared: PictureComparison, surround: Int): Difference {
        val kept = compared.kept; val whole = compared.whole
        try {
            assertEquals(whole.width, kept.width); assertEquals(whole.height, kept.height)
            val width = whole.width; val height = whole.height
            val rows = 64
            val a = IntArray(width * rows); val b = IntArray(width * rows)
            var differing = 0; var overOne = 0; var worst = 0; var inked = 0
            var y = 0
            while (y < height) {
                val take = minOf(rows, height - y)
                kept.getPixels(a, 0, width, 0, y, width, take); whole.getPixels(b, 0, width, 0, y, width, take)
                for (index in 0 until width * take) {
                    val p = a[index]; val q = b[index]
                    if (q != surround && q != Color.WHITE) inked++
                    if (p == q) continue
                    val d = maxOf(maxOf(abs(Color.alpha(p) - Color.alpha(q)), abs(Color.red(p) - Color.red(q))),
                        maxOf(abs(Color.green(p) - Color.green(q)), abs(Color.blue(p) - Color.blue(q))))
                    differing++; if (d > 1) overOne++; if (d > worst) worst = d
                }
                y += take
            }
            return Difference(width, height, differing, overOne, worst, inked)
        } finally { kept.recycle(); whole.recycle() }
    }

    private class Counts(val rebuilt: Long, val added: Long, val frames: Long, val whole: Long, val failed: Long) {
        override fun toString() = "rebuilt=$rebuilt added=$added frames=$frames whole=$whole failed=$failed"
    }
    private fun counts(scenario: ActivityScenario<MainActivity>): Counts {
        var read: Counts? = null
        scenario.onActivity { activity ->
            val counted = requireNotNull(canvas(activity.window.decorView)).pictureCounts
            read = Counts(counted.rebuilt.get(), counted.added.get(), counted.frames.get(), counted.whole.get(), counted.failed.get())
        }
        return requireNotNull(read)
    }

    /** Waits for the editor to settle, then has the canvas draw its next saved
     * frame both ways and compares them. The frame must have been drawn from
     * the kept picture, and must not differ from the whole scene by more than
     * [bound] in any channel of any pixel. */
    private fun compare(scenario: ActivityScenario<MainActivity>, phase: String, bound: Int, settle: Boolean = true): Difference {
        if (settle) await(scenario, "settled-before-$phase", predicate = ::settled)
        val deadline = SystemClock.uptimeMillis() + 20_000
        var state = "never asked"
        while (SystemClock.uptimeMillis() < deadline) {
            val answered = CountDownLatch(1)
            val result = AtomicReference<PictureComparison?>(null)
            var asked = false; var surround = 0
            scenario.onActivity { activity ->
                val surface = canvas(activity.window.decorView)
                surround = EditorDrawing.surroundArgb(activity.editor.preferences.darkTheme)
                asked = surface?.comparePicture { compared -> result.set(compared); answered.countDown() } == true
                state = "asked=$asked canvas=${surface?.lifeState()} pictures=${surface?.pictureCounts}"
            }
            if (asked && answered.await(1_000, TimeUnit.MILLISECONDS)) {
                val compared = result.get()
                if (compared != null) {
                    val found = difference(compared, surround)
                    note("phase=$phase $found ${counts(scenario)}")
                    assertTrue("$phase: nothing but page and surround was drawn. $found\n$notes", found.inked > 500)
                    assertTrue("$phase: the frame from the kept picture is not the whole scene: a pixel differs by ${found.worst} " +
                        "(allowed $bound). $found\n$notes", found.worst <= bound)
                    return found
                }
                state += " answered=not_from_the_picture"
            }
            SystemClock.sleep(60)
        }
        throw AssertionError("$phase: no frame was drawn from the kept picture within 20 s. $state\n$notes")
    }

    /** One pen contact through the canvas view's own touch dispatch, in one
     * main-thread turn. [points] are in document space. Without [up] the pen
     * stays down. */
    private fun contact(scenario: ActivityScenario<MainActivity>, points: List<Point>, down: Boolean = true, up: Boolean = true, finger: Boolean = false) {
        scenario.onActivity { activity ->
            val surface = requireNotNull(canvas(activity.window.decorView))
            val matrix = requireNotNull(activity.editor.scene).matrix
            val start = SystemClock.uptimeMillis()
            points.forEachIndexed { index, point ->
                val action = when {
                    index == 0 && down -> MotionEvent.ACTION_DOWN
                    index == points.lastIndex && up -> MotionEvent.ACTION_UP
                    else -> MotionEvent.ACTION_MOVE
                }
                val screen = EditorDrawing.map(matrix, point)
                val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = if (finger) MotionEvent.TOOL_TYPE_FINGER else MotionEvent.TOOL_TYPE_STYLUS })
                val coordinates = arrayOf(MotionEvent.PointerCoords().apply {
                    x = screen.x.toFloat(); y = screen.y.toFloat(); pressure = .55f + .35f * index / maxOf(1, points.lastIndex); size = .01f })
                val event = MotionEvent.obtain(start, start + index * 8L, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0,
                    if (finger) InputDevice.SOURCE_TOUCHSCREEN else InputDevice.SOURCE_STYLUS, 0)
                try { assertTrue(surface.dispatchTouchEvent(event)) } finally { event.recycle() }
            }
        }
    }
    private fun line(from: Point, to: Point, steps: Int = 12): List<Point> =
        (0..steps).map { Point(from.x + (to.x - from.x) * it / steps, from.y + (to.y - from.y) * it / steps) }

    private fun items(scenario: ActivityScenario<MainActivity>): Int {
        var size = -1
        scenario.onActivity { size = it.editor.document?.render?.items?.size ?: -1 }
        return size
    }

    /** One whole stroke with the chosen tool, and the wait until it is saved and shown as saved. */
    private fun stroke(scenario: ActivityScenario<MainActivity>, phase: String, from: Point, to: Point) {
        await(scenario, "ready-for-$phase", predicate = ::settled)
        val before = items(scenario)
        contact(scenario, line(from, to))
        await(scenario, "saved-$phase") { settled(it) && it.editor.document?.render?.items?.size == before + 1 }
    }

    /** Looks at the composited screen: the middle of the stroke from [from]
     * to [to] must show [argb], and must go on showing it. The first sample
     * that shows it starts a watch of [watchMs]; a later sample without it is
     * a frame in which the stroke was gone. */
    private fun assertStrokeOnScreen(scenario: ActivityScenario<MainActivity>, phase: String, from: Point, to: Point, argb: Int, watchMs: Long = 500) {
        val automation = instrumentation.uiAutomation
        val start = SystemClock.uptimeMillis()
        var seenAt = -1L; var samples = 0; var last = "no sample"
        while (true) {
            var x = -1; var y = -1
            scenario.onActivity { activity ->
                val view = canvas(activity.window.decorView); val scene = activity.editor.scene
                if (view != null && scene != null) {
                    val location = IntArray(2); view.getLocationOnScreen(location)
                    val middle = EditorDrawing.map(scene.matrix, Point((from.x + to.x) / 2, (from.y + to.y) / 2))
                    x = (middle.x + location[0]).toInt(); y = (middle.y + location[1]).toInt()
                }
            }
            val raw = if (x < 0) null else automation.takeScreenshot()
            val elapsed = SystemClock.uptimeMillis() - start
            if (raw != null) {
                val shot = if (raw.config == Bitmap.Config.HARDWARE) raw.copy(Bitmap.Config.ARGB_8888, false).also { raw.recycle() } else raw
                try {
                    samples++
                    var shown = false
                    for (dy in -2..2) for (dx in -2..2) {
                        val px = x + dx; val py = y + dy
                        if (px !in 0 until shot.width || py !in 0 until shot.height) continue
                        val pixel = shot.getPixel(px, py)
                        if (abs(Color.red(pixel) - Color.red(argb)) <= 56 && abs(Color.green(pixel) - Color.green(argb)) <= 56 &&
                            abs(Color.blue(pixel) - Color.blue(argb)) <= 56) shown = true
                    }
                    last = "at=${elapsed}ms pixel=${String.format("%08x", if (x in 0 until shot.width && y in 0 until shot.height) shot.getPixel(x, y) else 0)} shown=$shown"
                    if (shown && seenAt < 0) seenAt = elapsed
                    if (!shown && seenAt >= 0) fail("$phase: the stroke was on screen at $seenAt ms and gone again at $elapsed ms ($last)\n$notes")
                } finally { shot.recycle() }
            }
            if (seenAt >= 0 && elapsed - seenAt >= watchMs) break
            if (seenAt < 0 && elapsed > 3_000) fail("$phase: the stroke never appeared on screen ($last, $samples samples)\n$notes")
        }
        note("phase=$phase on_screen_after_ms=$seenAt samples=$samples")
    }

    /** A new page in the real app, the varied page put into it through the
     * core, then [body] with the page open. Afterwards the page is closed,
     * the preferences are put back and the project is deleted. */
    private fun withPage(body: (ActivityScenario<MainActivity>, Int) -> Unit) {
        var owned: String? = null
        var storageClosed = false
        var preferences: EditorPreferences? = null
        try {
            ActivityScenario.launch(MainActivity::class.java).use { scenario ->
                await(scenario, "startup") { !it.editor.busy }
                // Held upright whichever way the phone lies: the comparisons are for one known shape of canvas.
                scenario.onActivity { preferences = it.editor.preferences; it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
                SystemClock.sleep(500)
                await(scenario, "upright") { !it.editor.busy && it.resources.configuration.orientation == Configuration.ORIENTATION_PORTRAIT }
                scenario.onActivity { it.editor.newCanvas() }
                await(scenario, "new-page", predicate = ::settled)
                scenario.onActivity { owned = requireNotNull(it.editor.info).projectId; assertNull(it.editor.blockingMessage) }
                val id = requireNotNull(owned)
                try {
                    scenario.onActivity { it.editor.navigate(WorkbenchScreen.Projects) }
                    await(scenario, "closed-to-fill") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                    val filled = fillPage(id)
                    scenario.onActivity { it.editor.open(id) }
                    await(scenario, "filled-page", timeoutMs = 120_000, predicate = ::settled)
                    assertEquals(filled, items(scenario))
                    body(scenario, filled)
                } finally {
                    scenario.onActivity { activity ->
                        activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
                        activity.editor.cancelTool(); preferences?.let { activity.editor.settings(it) }
                    }
                    await(scenario, "edits-saved-to-close", timeoutMs = 120_000) { it.editor.pending == 0 && !it.editor.busy }
                    scenario.onActivity { if (it.editor.screen != WorkbenchScreen.Projects) it.editor.navigate(WorkbenchScreen.Projects) }
                    await(scenario, "page-closed", timeoutMs = 120_000) { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                    storageClosed = true
                }
            }
        } finally {
            owned?.let { id ->
                val target = File(projectPath(id))
                if (storageClosed) assertTrue(!target.exists() || target.deleteRecursively())
            }
            // The differences found, where `am instrument` prints them.
            instrumentation.sendStatus(0, Bundle().apply { putString(Instrumentation.REPORT_KEY_STREAMRESULT, "\n$notes") })
        }
    }

    private fun choose(scenario: ActivityScenario<MainActivity>, tool: EditorTool, width: Double, argb: Long) {
        scenario.onActivity { it.editor.chooseTool(tool); it.editor.brush(BrushPreference(width = width, colorArgb = argb)) }
        // Choosing a tool can show other controls and so resize the canvas.
        SystemClock.sleep(400)
        await(scenario, "tool-$tool", predicate = ::settled)
    }

    @Test fun strokesDrawnWithThePenAreAddedToThePictureOnceAndTheFrameStaysTheWholeScene() = withPage { scenario, filled ->
        var page = Point(0.0, 0.0)
        scenario.onActivity { val doc = requireNotNull(it.editor.document); page = Point(doc.width.toDouble(), doc.height.toDouble()) }
        // The picture as first made from every object: the whole scene, exactly.
        compare(scenario, "opened", REBUILT_BOUND)

        choose(scenario, EditorTool.Pen, 9.0, 0xff14326e)
        compare(scenario, "pen-chosen", REBUILT_BOUND)
        var before = counts(scenario)
        // Pen strokes across the strokes already on the page, each compared when saved.
        val pen = 6
        repeat(pen) { index ->
            val from = Point(page.x * (0.12 + 0.1 * index), page.y * 0.1)
            val to = Point(page.x * (0.3 + 0.1 * index), page.y * (0.55 + 0.05 * index))
            stroke(scenario, "pen-$index", from, to)
            compare(scenario, "pen-$index", ADDED_BOUND)
            if (index == pen - 1) assertStrokeOnScreen(scenario, "pen-$index", from, to, 0xff14326e.toInt())
        }
        var after = counts(scenario)
        assertEquals("Each pen stroke is drawn into the picture once. before: $before after: $after\n$notes", before.added + pen, after.added)
        assertEquals("No pen stroke made the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        assertEquals("No picture failed to draw. $after\n$notes", 0L, after.failed)
        assertEquals(filled + pen, items(scenario))

        // The marker brush: wide strokes in another colour over all of it, one out over the page's edge.
        choose(scenario, EditorTool.Marker, 22.0, 0xff0a6b3c)
        compare(scenario, "marker-chosen", ADDED_BOUND)
        before = counts(scenario)
        val marker = 4
        repeat(marker) { index ->
            val from = Point(page.x * (if (index == 3) -0.05 else 0.1), page.y * (0.2 + 0.15 * index))
            val to = Point(page.x * (if (index == 2) 1.05 else 0.85), page.y * (0.28 + 0.12 * index))
            stroke(scenario, "marker-$index", from, to)
            compare(scenario, "marker-$index", ADDED_BOUND)
        }
        after = counts(scenario)
        assertEquals("Each marker stroke is drawn into the picture once. before: $before after: $after\n$notes", before.added + marker, after.added)
        assertEquals("No marker stroke made the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)

        // Ink in progress: the pen is down and has moved, and stays down while the frame is compared.
        choose(scenario, EditorTool.Pen, 9.0, 0xff7a1420)
        await(scenario, "ready-for-wet", predicate = ::settled)
        before = counts(scenario)
        val wetFrom = Point(page.x * 0.15, page.y * 0.8); val wetTo = Point(page.x * 0.8, page.y * 0.3)
        contact(scenario, line(wetFrom, wetTo), up = false)
        await(scenario, "ink-in-progress") { it.editor.scene?.provisional?.isNotEmpty() == true }
        compare(scenario, "ink-in-progress", ADDED_BOUND, settle = false)
        scenario.onActivity { assertTrue("The pen was still down when the frame was compared", it.editor.scene?.provisional?.isNotEmpty() == true) }
        val held = items(scenario)
        contact(scenario, listOf(wetTo, wetTo), down = false)
        await(scenario, "wet-stroke-saved") { settled(it) && it.editor.document?.render?.items?.size == held + 1 }
        compare(scenario, "ink-saved", ADDED_BOUND)
        assertStrokeOnScreen(scenario, "ink-saved", wetFrom, wetTo, 0xff7a1420.toInt())
        after = counts(scenario)
        assertEquals("The stroke was drawn into the picture once, when it was saved. before: $before after: $after\n$notes", before.added + 1, after.added)
        assertEquals(before.rebuilt, after.rebuilt)

        // Strokes faster than they are saved: each is still added, none makes the picture again.
        before = counts(scenario)
        val rapid = 10
        val base = items(scenario)
        scenario.onActivity { activity ->
            val surface = requireNotNull(canvas(activity.window.decorView))
            val matrix = requireNotNull(activity.editor.scene).matrix
            val main = Handler(Looper.getMainLooper())
            val first = SystemClock.uptimeMillis() + 300
            for (index in 0 until rapid) {
                val points = line(Point(page.x * (0.1 + 0.07 * index), page.y * 0.62), Point(page.x * (0.16 + 0.07 * index), page.y * 0.92), steps = 5)
                val down = first + index * 90L
                points.forEachIndexed { step, point ->
                    val screen = EditorDrawing.map(matrix, point)
                    val action = when (step) { 0 -> MotionEvent.ACTION_DOWN; points.lastIndex -> MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                    main.postAtTime({
                        val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_STYLUS })
                        val coordinates = arrayOf(MotionEvent.PointerCoords().apply { x = screen.x.toFloat(); y = screen.y.toFloat(); pressure = .6f; size = .01f })
                        val event = MotionEvent.obtain(down, down + step * 8L, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
                        try { surface.dispatchTouchEvent(event) } finally { event.recycle() }
                    }, down + step * 8L)
                }
            }
        }
        await(scenario, "rapid-strokes-saved", timeoutMs = 60_000) { settled(it) && it.editor.document?.render?.items?.size == base + rapid }
        compare(scenario, "rapid-strokes", ADDED_BOUND)
        after = counts(scenario)
        assertTrue("Strokes saved in a burst are added to the picture, together or one by one. before: $before after: $after\n$notes",
            after.added > before.added && after.added <= before.added + rapid)
        assertEquals("No stroke of the burst made the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        assertEquals("No picture failed to draw. $after\n$notes", 0L, after.failed)
    }

    @Test fun whateverChangesThePageOrTheViewMakesThePictureAgain() = withPage { scenario, filled ->
        var page = Point(0.0, 0.0)
        scenario.onActivity { val doc = requireNotNull(it.editor.document); page = Point(doc.width.toDouble(), doc.height.toDouble()) }
        choose(scenario, EditorTool.Pen, 8.0, 0xff14326e)
        compare(scenario, "opened", REBUILT_BOUND)

        /** [change], then a comparison; the picture must have been made again. */
        fun remade(phase: String, bound: Int = REBUILT_BOUND, change: () -> Unit) {
            val before = counts(scenario)
            change()
            compare(scenario, phase, bound)
            val after = counts(scenario)
            assertTrue("$phase must make the picture again. before: $before after: $after\n$notes", after.rebuilt > before.rebuilt)
        }

        // A stroke, then undo and redo.
        stroke(scenario, "stroke-to-undo", Point(page.x * 0.2, page.y * 0.2), Point(page.x * 0.7, page.y * 0.6))
        compare(scenario, "stroke-to-undo", ADDED_BOUND)
        remade("undo") {
            scenario.onActivity { it.editor.undo() }
            await(scenario, "undone") { settled(it) && it.editor.document?.render?.items?.size == filled }
        }
        run {
            val before = counts(scenario)
            scenario.onActivity { it.editor.redo() }
            await(scenario, "redone") { settled(it) && it.editor.document?.render?.items?.size == filled + 1 }
            compare(scenario, "redo", ADDED_BOUND)
            val after = counts(scenario)
            assertTrue("Redo must change the picture. before: $before after: $after\n$notes", after.rebuilt + after.added > before.rebuilt + before.added)
        }

        // An edit: a rectangle is made, moved, and deleted.
        scenario.onActivity { it.editor.chooseTool(EditorTool.Rectangle) }
        SystemClock.sleep(400); await(scenario, "rectangle-tool", predicate = ::settled)
        val corner = Point(page.x * 0.3, page.y * 0.3); val opposite = Point(page.x * 0.6, page.y * 0.55)
        contact(scenario, listOf(corner, Point(page.x * 0.45, page.y * 0.4), opposite))
        await(scenario, "rectangle-saved") { settled(it) && it.editor.document?.render?.items?.any { item -> item.shape is Shape.Rectangle } == true }
        compare(scenario, "rectangle", ADDED_BOUND)
        scenario.onActivity { it.editor.chooseTool(EditorTool.Select) }
        SystemClock.sleep(400); await(scenario, "select-tool", predicate = ::settled)
        remade("moved", ADDED_BOUND) {
            // Its outline is taken hold of on the left side and dragged.
            val grip = Point(corner.x, (corner.y + opposite.y) / 2)
            contact(scenario, listOf(grip, Point(grip.x + page.x * 0.04, grip.y + page.y * 0.03), Point(grip.x + page.x * 0.08, grip.y + page.y * 0.06)))
            await(scenario, "move-saved") { activity ->
                settled(activity) && activity.editor.document?.render?.items?.any { it.shape is Shape.Rectangle && (it.transform.e != 0.0 || it.transform.f != 0.0) } == true
            }
        }
        remade("deleted", ADDED_BOUND) {
            scenario.onActivity { it.editor.deleteSelected() }
            await(scenario, "delete-saved") { settled(it) && it.editor.document?.render?.items?.none { item -> item.shape is Shape.Rectangle } == true }
        }

        // The view: pan, zoom, a turn that is not a quarter turn, and back.
        scenario.onActivity { it.editor.chooseTool(EditorTool.Pen) }
        SystemClock.sleep(400); await(scenario, "pen-tool", predicate = ::settled)
        compare(scenario, "before-view-changes", ADDED_BOUND)
        var middle = Point(0.0, 0.0)
        scenario.onActivity { val camera = requireNotNull(it.editor.scene).camera; middle = Point(camera.viewportWidth / 2, camera.viewportHeight / 2) }
        remade("panned") { scenario.onActivity { it.editor.pan(37.0, -21.0) } }
        remade("zoomed") { scenario.onActivity { it.editor.pinch(middle.x, middle.y, middle.x, middle.y, 1.7, 0.0) } }
        remade("turned") { scenario.onActivity { it.editor.pinch(middle.x, middle.y, middle.x, middle.y, 0.5, 0.4) } }
        // In the turned view the page's edge runs across the pixels: a stroke out over it is added to the picture.
        run {
            val before = counts(scenario)
            stroke(scenario, "over-the-turned-edge", Point(page.x * 0.6, page.y * 0.5), Point(page.x * 1.08, page.y * 0.45))
            compare(scenario, "over-the-turned-edge", ADDED_BOUND)
            val after = counts(scenario)
            assertEquals("The stroke is added to the picture of the turned view. before: $before after: $after\n$notes", before.added + 1, after.added)
            assertEquals(before.rebuilt, after.rebuilt)
        }
        remade("fitted", ADDED_BOUND) { scenario.onActivity { it.editor.fit() } }

        // The theme is the surround, drawn for every frame: the picture stands and the frame is still the whole scene.
        run {
            val before = counts(scenario)
            scenario.onActivity { it.editor.settings(it.editor.preferences.copy(darkTheme = !it.editor.preferences.darkTheme)) }
            compare(scenario, "other-theme", ADDED_BOUND)
            val after = counts(scenario)
            assertEquals("The theme is not part of the picture. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
            scenario.onActivity { it.editor.settings(it.editor.preferences.copy(darkTheme = !it.editor.preferences.darkTheme)) }
            compare(scenario, "theme-back", ADDED_BOUND)
        }
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    @Test fun thePictureIsMadeAgainAfterRecreationRotationLeavingAndAMemoryWarning() = withPage { scenario, filled ->
        var page = Point(0.0, 0.0)
        scenario.onActivity { val doc = requireNotNull(it.editor.document); page = Point(doc.width.toDouble(), doc.height.toDouble()) }
        choose(scenario, EditorTool.Pen, 9.0, 0xff14326e)
        compare(scenario, "opened", REBUILT_BOUND)
        var drawn = 0
        /** A stroke through the pen path, added to whatever picture the canvas now holds. */
        fun strokeAdded(phase: String) {
            val before = counts(scenario)
            val from = Point(page.x * (0.15 + 0.08 * drawn), page.y * 0.15); val to = Point(page.x * (0.3 + 0.08 * drawn), page.y * 0.85)
            stroke(scenario, phase, from, to); drawn++
            compare(scenario, phase, ADDED_BOUND)
            assertStrokeOnScreen(scenario, phase, from, to, 0xff14326e.toInt())
            val after = counts(scenario)
            assertEquals("$phase: the stroke is added to the picture. before: $before after: $after\n$notes", before.added + 1, after.added)
            assertEquals("$phase: the stroke did not make the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        }
        strokeAdded("first-stroke")

        // The activity is made again: a new canvas, a new render session, a new picture.
        scenario.recreate()
        await(scenario, "recreated", predicate = ::settled)
        compare(scenario, "after-recreation", REBUILT_BOUND)
        strokeAdded("stroke-after-recreation")

        // The device is turned: the canvas has another size and the surface another generation.
        fun turn(orientation: Int, phase: String, landscape: Boolean) {
            scenario.onActivity { it.requestedOrientation = orientation }
            await(scenario, phase) { activity ->
                val view = canvas(activity.window.decorView)
                view != null && (view.width > view.height) == landscape && settled(activity) &&
                    activity.editor.scene?.camera?.let { it.viewportWidth.toInt() == view.width && it.viewportHeight.toInt() == view.height } == true
            }
            compare(scenario, phase, REBUILT_BOUND)
        }
        turn(ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE, "landscape", landscape = true)
        strokeAdded("stroke-in-landscape")
        turn(ActivityInfo.SCREEN_ORIENTATION_PORTRAIT, "portrait-again", landscape = false)
        strokeAdded("stroke-in-portrait-again")

        // The app is left and returned to, as with Home: the surface goes and comes back.
        scenario.moveToState(Lifecycle.State.CREATED)
        SystemClock.sleep(600)
        scenario.moveToState(Lifecycle.State.RESUMED)
        await(scenario, "returned", predicate = ::settled)
        compare(scenario, "after-returning", REBUILT_BOUND)
        strokeAdded("stroke-after-returning")

        // The system asks for memory: the picture is given up, and made again for the next frame.
        run {
            val before = counts(scenario)
            scenario.onActivity { it.application.onTrimMemory(ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL) }
            compare(scenario, "after-memory-warning", REBUILT_BOUND)
            val after = counts(scenario)
            assertTrue("A memory warning gives the picture up; the next frame makes it again. before: $before after: $after\n$notes",
                after.rebuilt > before.rebuilt)
        }
        strokeAdded("stroke-after-memory-warning")
        assertEquals(filled + drawn, items(scenario))
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    private companion object {
        const val TAG = "VW_KEPT_PICTURE"
        /** A frame from a picture made from every object is the whole scene. */
        const val REBUILT_BOUND = 0
        /** After strokes have been added to the picture: rounding at their edges. */
        const val ADDED_BOUND = 2
    }
}
