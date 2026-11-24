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
import android.os.HandlerThread
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
import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.KeptPicture
import com.visualworkbench.android.precision.PrecisionDrawing
import com.visualworkbench.shared.DocumentControlCommand
import com.visualworkbench.shared.EditCommand
import com.visualworkbench.shared.EditOptions
import com.visualworkbench.shared.EraserChoice
import com.visualworkbench.shared.InstructionCommand
import com.visualworkbench.shared.InstructionEntryMethod
import com.visualworkbench.shared.InstructionRole
import com.visualworkbench.shared.instructionWorkflows
import com.visualworkbench.shared.LayerControlUpdate
import com.visualworkbench.shared.ObjectStyle
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.Transform
import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.WorkflowBinding
import com.visualworkbench.shared.WorkflowMetadata
import com.visualworkbench.shared.documentControlWorkflows
import com.visualworkbench.android.editor.CanvasSurface
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.editor.EditorPreferences
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.android.editor.PictureComparison
import com.visualworkbench.android.editor.PictureSurface
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

/** The canvas draws its frames from kept pictures of the saved page, one for
 * each layer that draws, composited with each layer's opacity and blend, and a
 * finished stroke is drawn into its layer's picture once. These tests hold it
 * to the whole-scene drawing it replaces, on the real surface of the real app
 * with a real native project.
 *
 * A comparison asks the canvas, on its drawing thread and for the very scene
 * it has just put on screen, to draw the frame twice off screen with the
 * renderer that draws the canvas: once as it was drawn, from the kept
 * pictures the session holds at that moment, and once whole from every
 * object. The two are compared pixel by pixel. A frame of a one-layer page
 * that follows pictures made from every object must be the whole scene
 * exactly ([REBUILT]). Once strokes have been added to a picture ([ADDED]),
 * the edge pixels of those strokes may differ by rounding: a whole scene
 * draws a stroke with everything else, the picture draws it alone, so a
 * colour channel may be one step off. Measured on the S23 (Android 17): at
 * most two pixels of a page differed by more, up to 59 steps, at the
 * anti-aliased edge of an added pen stroke; the renderer does not always
 * smooth the same outline the same way when it is drawn alone and when it is
 * drawn with everything else. [ADDED] allows [COARSE_PIXELS] such pixels, each
 * off by at most [COARSE_STEPS] (a quarter of full coverage). On a page of
 * several layers each layer's picture is drawn alone, and the same bound holds
 * for every frame ([LAYERED]). The differences found are written to the log
 * and into the instrumentation's output.
 *
 * A change to the page draws again only the part of the pictures it touched,
 * grown until it cuts no stroke, and while an object is moved or turned each
 * frame draws that part over the pictures ([PATCHED]: the same bound as
 * [ADDED]). These are checked on a page of three layers (one translucent, one
 * a highlighter's multiply layer) after undo, redo, erasing a whole stroke and
 * part of one, a move, a turn, a deletion, a restyle and edits the paired
 * computer would make, and while the pen is down moving, turning and erasing.
 * Strokes in the lowest, the highlighter's and the translucent layer, and ink
 * in progress in each, are drawn into or over their own layer's picture; a
 * layer hidden, shown, made fainter, given another blend or put elsewhere
 * only composites the pictures otherwise (a layer shown draws its own picture
 * once). After a pan, a pinch, a turn, a fit and a zoom far out and far in,
 * once the view rests, the frame is the whole scene again; a pan with
 * everything on the screen whole only moves the pictures, and every frame of
 * it is the whole scene. Scenes the editor does not make here (objects hidden
 * while they are erased) are drawn by kept pictures of the test's own, as the
 * canvas draws them, and compared the same way.
 *
 * The tests also read how often pictures were made from every object, how
 * often a stroke was added to one, a part of one drawn again or one layer's
 * picture drawn whole, so that "drawn into it once", "only what changed" and
 * "made again when it would be wrong" are counted, not assumed; and they look
 * at the composited screen after a pen-up for the stroke that was just drawn.
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
                "screen=${editor.screen} items=${editor.document?.render?.items?.size} selected=${editor.selected} " +
                "transformed=${editor.scene?.transformed?.keys} tool=${editor.tool} pictures=${canvas(activity.window.decorView)?.pictureCounts} " +
                "canvas=${canvas(activity.window.decorView)?.lifeState()} pen=${editor.diagnostics.pen.report(emptyList()).lines().filter { it.contains("refus") || it.contains("cancel") }}"
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

    /** What a comparison found. [overOne]: pixels with a channel more than one
     * step off, the first few of them in [samples] (place: kept/whole).
     * [inked]: pixels of the whole scene that are neither the page nor the
     * surround, so that two empty frames are never taken for a match.
     * [edge]: differing pixels within [EDGE_PX] of the page's edge, where its
     * anti-aliased outline cuts through them; [worstInside] the most any
     * channel differs anywhere else. */
    private class Difference(val width: Int, val height: Int, val differing: Int, val overOne: Int, val worst: Int, val inked: Int,
                             val edge: Int, val worstInside: Int, val samples: String) {
        override fun toString() = "size=${width}x$height differing=$differing over_one=$overOne worst=$worst " +
            "at_page_edge=$edge worst_inside=$worstInside inked=$inked" + (if (samples.isEmpty()) "" else " over_one_at=$samples")
    }

    /** How far a frame from the kept picture may be from the whole scene:
     * every channel of every pixel within [steps], except at most [coarse]
     * pixels, which may be off by up to [coarseSteps]. */
    private class Bound(val steps: Int, val coarse: Int, val coarseSteps: Int) {
        fun allows(found: Difference) = found.worst <= coarseSteps && (found.worst <= steps || found.overOne <= coarse) &&
            (steps >= 1 || found.differing <= coarse)
        override fun toString() = "every channel within $steps, at most $coarse pixels within $coarseSteps"
    }

    private fun difference(compared: PictureComparison, surround: Int): Difference {
        val kept = compared.kept; val whole = compared.whole
        try {
            assertEquals(whole.width, kept.width); assertEquals(whole.height, kept.height)
            val width = whole.width; val height = whole.height
            // The page's four edges on screen, as lines: distance = |a x + b y + c|.
            val m = compared.matrix
            val corners = listOf(Point(0.0, 0.0), Point(compared.pageWidth, 0.0), Point(compared.pageWidth, compared.pageHeight), Point(0.0, compared.pageHeight))
                .map { EditorDrawing.map(m, it) }
            val edges = corners.indices.map { i ->
                val p = corners[i]; val q = corners[(i + 1) % 4]
                val dx = q.x - p.x; val dy = q.y - p.y; val length = kotlin.math.hypot(dx, dy)
                doubleArrayOf(-dy / length, dx / length, (dy * p.x - dx * p.y) / length, p.x, p.y, q.x, q.y)
            }
            fun nearEdge(x: Int, y: Int): Boolean {
                val px = x + .5; val py = y + .5
                for (e in edges) {
                    if (abs(e[0] * px + e[1] * py + e[2]) > EDGE_PX) continue
                    // Within the segment's extent (grown by the margin).
                    if (px >= minOf(e[3], e[5]) - EDGE_PX && px <= maxOf(e[3], e[5]) + EDGE_PX &&
                        py >= minOf(e[4], e[6]) - EDGE_PX && py <= maxOf(e[4], e[6]) + EDGE_PX) return true
                }
                return false
            }
            val rows = 64
            val a = IntArray(width * rows); val b = IntArray(width * rows)
            var differing = 0; var overOne = 0; var worst = 0; var inked = 0; var edge = 0; var worstInside = 0
            val samples = StringBuilder()
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
                    if (d > 1 && overOne <= 4) samples.append(" ${index % width},${y + index / width}:${String.format("%08x", p)}/${String.format("%08x", q)}")
                    if (nearEdge(index % width, y + index / width)) edge++ else if (d > worstInside) worstInside = d
                }
                y += take
            }
            return Difference(width, height, differing, overOne, worst, inked, edge, worstInside, samples.toString())
        } finally { kept.recycle(); whole.recycle() }
    }

    private class Counts(val rebuilt: Long, val added: Long, val patched: Long, val frames: Long, val over: Long, val whole: Long, val failed: Long, val area: Long) {
        override fun toString() = "rebuilt=$rebuilt added=$added patched=$patched frames=$frames over=$over whole=$whole failed=$failed area=$area"
    }
    private fun counts(scenario: ActivityScenario<MainActivity>): Counts {
        var read: Counts? = null
        scenario.onActivity { activity ->
            val counted = requireNotNull(canvas(activity.window.decorView)).pictureCounts
            read = Counts(counted.rebuilt.get(), counted.added.get(), counted.patched.get(), counted.frames.get(), counted.over.get(),
                counted.whole.get(), counted.failed.get(), counted.area.get())
        }
        return requireNotNull(read)
    }

    /** Waits for the editor to settle, then has the canvas draw its next saved
     * frame both ways and compares them. The frame must have been drawn from
     * the kept picture, and must be as close to the whole scene as [bound]
     * allows. */
    private fun compare(scenario: ActivityScenario<MainActivity>, phase: String, bound: Bound, settle: Boolean = true): Difference {
        if (settle) await(scenario, "settled-before-$phase", predicate = ::settled)
        // After a pan, a pinch or a turn the pictures are drawn at the view once it rests; till then frames are softer.
        await(scenario, "exact-before-$phase") { canvas(it.window.decorView)?.pictureCounts?.soft?.get() != true }
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
                    assertTrue("$phase: the frame from the kept picture is not the whole scene (allowed: $bound). $found\n$notes", bound.allows(found))
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
    private fun withPage(body: (ActivityScenario<MainActivity>, Int) -> Unit) = withPage(::fillPage, { it }, body)

    /** As above, with the page put in by [fill], which returns what [body]
     * is given; [count] is how many objects it put in. */
    private fun <T> withPage(fill: (String) -> T, count: (T) -> Int, body: (ActivityScenario<MainActivity>, T) -> Unit) {
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
                    val filled = fill(id)
                    scenario.onActivity { it.editor.open(id) }
                    await(scenario, "filled-page", timeoutMs = 120_000, predicate = ::settled)
                    assertEquals(count(filled), items(scenario))
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
                    // Once the canvas is gone, no picture, buffer or renderer of it is left.
                    val deadline = SystemClock.uptimeMillis() + 10_000
                    while ((PictureSurface.liveSurfaces.get() != 0 || PictureSurface.livePictures.get() != 0) && SystemClock.uptimeMillis() < deadline)
                        SystemClock.sleep(50)
                    val left = "surfaces=${PictureSurface.liveSurfaces.get()} pictures=${PictureSurface.livePictures.get()}"
                    note("left_after_closing $left")
                    assertTrue("The closed canvas left kept pictures behind: $left\n$notes",
                        PictureSurface.liveSurfaces.get() == 0 && PictureSurface.livePictures.get() == 0)
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
        compare(scenario, "opened", REBUILT)

        choose(scenario, EditorTool.Pen, 9.0, 0xff14326e)
        compare(scenario, "pen-chosen", REBUILT)
        var before = counts(scenario)
        // Pen strokes across the strokes already on the page, each compared when saved.
        val pen = 6
        repeat(pen) { index ->
            val from = Point(page.x * (0.12 + 0.1 * index), page.y * 0.1)
            val to = Point(page.x * (0.3 + 0.1 * index), page.y * (0.55 + 0.05 * index))
            stroke(scenario, "pen-$index", from, to)
            compare(scenario, "pen-$index", ADDED)
            if (index == pen - 1) assertStrokeOnScreen(scenario, "pen-$index", from, to, 0xff14326e.toInt())
        }
        var after = counts(scenario)
        assertEquals("Each pen stroke is drawn into the picture once. before: $before after: $after\n$notes", before.added + pen, after.added)
        assertEquals("No pen stroke made the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        assertEquals("No picture failed to draw. $after\n$notes", 0L, after.failed)
        assertEquals(filled + pen, items(scenario))

        // The marker brush: wide strokes in another colour over all of it, one out over the page's edge.
        choose(scenario, EditorTool.Marker, 22.0, 0xff0a6b3c)
        compare(scenario, "marker-chosen", ADDED)
        before = counts(scenario)
        val marker = 4
        repeat(marker) { index ->
            val from = Point(page.x * (if (index == 3) -0.05 else 0.1), page.y * (0.2 + 0.15 * index))
            val to = Point(page.x * (if (index == 2) 1.05 else 0.85), page.y * (0.28 + 0.12 * index))
            stroke(scenario, "marker-$index", from, to)
            compare(scenario, "marker-$index", ADDED)
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
        compare(scenario, "ink-in-progress", ADDED, settle = false)
        scenario.onActivity { assertTrue("The pen was still down when the frame was compared", it.editor.scene?.provisional?.isNotEmpty() == true) }
        val held = items(scenario)
        contact(scenario, listOf(wetTo, wetTo), down = false)
        await(scenario, "wet-stroke-saved") { settled(it) && it.editor.document?.render?.items?.size == held + 1 }
        compare(scenario, "ink-saved", ADDED)
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
        compare(scenario, "rapid-strokes", ADDED)
        after = counts(scenario)
        assertTrue("Strokes saved in a burst are added to the picture, together or one by one. before: $before after: $after\n$notes",
            after.added > before.added && after.added <= before.added + rapid)
        assertEquals("No stroke of the burst made the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        assertEquals("No picture failed to draw. $after\n$notes", 0L, after.failed)
    }

    /** One change of view through the editor's own entry points, between a
     * navigation contact going down and up as fingers make them. */
    private fun viewChange(scenario: ActivityScenario<MainActivity>, change: (EditorController, Point) -> Unit) {
        scenario.onActivity { activity ->
            val camera = requireNotNull(activity.editor.scene).camera
            activity.editor.navigationContact(true)
            change(activity.editor, Point(camera.viewportWidth / 2, camera.viewportHeight / 2))
        }
        // Frames of the moving view are drawn before the fingers lift.
        SystemClock.sleep(80)
        scenario.onActivity { it.editor.navigationContact(false) }
    }

    @Test fun settledFramesAfterPanPinchTurnFitAndAZoomAcrossLevelsAreTheWholeScene() = withPage(::fillLayered, { it.count }) { scenario, _ ->
        val page = pageSize(scenario)
        choose(scenario, EditorTool.Pen, 8.0, 0xff14326e)
        compare(scenario, "opened", LAYERED)

        /** [change] to the view, then a comparison once it has rested. [made]:
         * the pictures were drawn again at the view; otherwise they were only
         * moved, and no frame was softer meanwhile. */
        fun viewed(phase: String, made: Boolean, contact: Boolean = true, change: (EditorController, Point) -> Unit) {
            val before = counts(scenario)
            var soft = 0L
            scenario.onActivity { soft = requireNotNull(canvas(it.window.decorView)).pictureCounts.softFrames.get() }
            if (contact) viewChange(scenario, change)
            else scenario.onActivity { activity -> val camera = requireNotNull(activity.editor.scene).camera
                change(activity.editor, Point(camera.viewportWidth / 2, camera.viewportHeight / 2)) }
            compare(scenario, phase, LAYERED)
            val after = counts(scenario)
            var softAfter = 0L; var shifted = 0L
            scenario.onActivity { val counted = requireNotNull(canvas(it.window.decorView)).pictureCounts; softAfter = counted.softFrames.get(); shifted = counted.shifted.get() }
            note("phase=$phase soft_frames=${softAfter - soft} shifted=$shifted")
            if (made) {
                assertTrue("$phase must draw the pictures at the view once it rests. before: $before after: $after\n$notes", after.rebuilt > before.rebuilt)
                assertTrue("$phase: frames while the view moved are drawn from the pictures moved or scaled. before: $before after: $after\n$notes", softAfter > soft)
            } else {
                assertEquals("$phase: the pictures are only moved. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
                assertEquals("$phase: every frame of the move is the whole scene. before: $before after: $after\n$notes", soft, softAfter)
            }
        }

        // A pan with the page, and everything on it, on the screen whole: the
        // pictures move by whole pixels, nothing is drawn, and every frame is
        // the whole scene exactly. (The page of three layers keeps every
        // object within the page.)
        viewed("panned-whole-page", made = false) { editor, _ -> editor.pan(19.0, -13.0); editor.pan(-4.4, 2.6); editor.pan(-0.6, 0.4) }
        // Zoomed in three times: drawn smoothed while the fingers move, at the view once they lift.
        viewed("zoomed", made = true) { editor, middle -> repeat(10) { editor.pinch(middle.x, middle.y, middle.x, middle.y, 1.117, 0.0) } }
        // A pan while the screen cuts the page: moved meanwhile, drawn at the view once it rests.
        viewed("panned-zoomed", made = true) { editor, _ -> repeat(12) { editor.pan(-11.0, 7.0) } }
        viewed("turned", made = true) { editor, middle -> repeat(8) { editor.pinch(middle.x, middle.y, middle.x, middle.y, 1.0, 0.05) } }

        // On the zoomed and turned view: a stroke is added to its layer's
        // picture, and an undo draws a part of it again.
        run {
            val before = counts(scenario)
            stroke(scenario, "zoomed-stroke", Point(page.x * 0.45, page.y * 0.45), Point(page.x * 0.55, page.y * 0.52))
            compare(scenario, "zoomed-stroke", ADDED)
            val after = counts(scenario)
            assertEquals("The stroke on the zoomed view is added to the picture. before: $before after: $after\n$notes", before.added + 1, after.added)
            assertEquals(before.rebuilt, after.rebuilt)
            // Zoomed in, the part drawn again grows until it cuts no stroke; over
            // strokes that cross one another it may cover half the screen, and
            // then the layer's picture is drawn whole instead.
            val layersBefore = layerDrawn(scenario)
            scenario.onActivity { it.editor.undo() }
            await(scenario, "undone-zoomed") { settled(it) && (requireNotNull(canvas(it.window.decorView)).pictureCounts.patched.get() > after.patched ||
                layerDrawn(it) > layersBefore) }
            compare(scenario, "zoomed-undo", PATCHED)
            assertEquals(after.rebuilt, counts(scenario).rebuilt)
        }
        // Fit, a single change with no fingers: drawn at the view at once.
        viewed("fitted", made = true, contact = false) { editor, _ -> editor.fit() }
        // Across resolution levels: far out, the page small in the middle; then far in, past where it was.
        viewed("zoomed-out-far", made = true) { editor, middle -> repeat(6) { editor.pinch(middle.x, middle.y, middle.x, middle.y, 0.72, 0.0) } }
        viewed("zoomed-in-far", made = true) { editor, middle -> repeat(16) { editor.pinch(middle.x, middle.y, middle.x, middle.y, 1.27, 0.0) } }
        viewed("fitted-again", made = true, contact = false) { editor, _ -> editor.fit() }
        // A pan with the page on the screen whole: only moved. Then a stroke out
        // past the screen's edge: the screen now cuts it, and a pan is drawn at
        // the view once it rests.
        viewed("panned-fitted", made = false) { editor, _ -> editor.pan(-9.0, 11.0) }
        run {
            val before = counts(scenario)
            stroke(scenario, "past-the-edge", Point(page.x * 0.6, page.y * 0.5), Point(page.x * 1.2, page.y * 0.45))
            compare(scenario, "past-the-edge", ADDED)
            assertEquals(before.added + 1, counts(scenario).added)
        }
        viewed("panned-past-the-edge", made = true) { editor, _ -> editor.pan(9.0, -11.0) }

        // The theme is the surround, drawn for every frame: the pictures stand and the frame is still the whole scene.
        run {
            val before = counts(scenario)
            scenario.onActivity { it.editor.settings(it.editor.preferences.copy(darkTheme = !it.editor.preferences.darkTheme)) }
            compare(scenario, "other-theme", ADDED)
            val after = counts(scenario)
            assertEquals("The theme is not part of the pictures. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
            scenario.onActivity { it.editor.settings(it.editor.preferences.copy(darkTheme = !it.editor.preferences.darkTheme)) }
            compare(scenario, "theme-back", ADDED)
        }
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    @Test fun thePictureIsMadeAgainAfterRecreationRotationLeavingAndAMemoryWarning() = withPage { scenario, filled ->
        var page = Point(0.0, 0.0)
        scenario.onActivity { val doc = requireNotNull(it.editor.document); page = Point(doc.width.toDouble(), doc.height.toDouble()) }
        choose(scenario, EditorTool.Pen, 9.0, 0xff14326e)
        compare(scenario, "opened", REBUILT)
        var drawn = 0
        /** A stroke through the pen path, added to whatever picture the canvas now holds. */
        fun strokeAdded(phase: String) {
            val before = counts(scenario)
            val from = Point(page.x * (0.15 + 0.08 * drawn), page.y * 0.15); val to = Point(page.x * (0.3 + 0.08 * drawn), page.y * 0.85)
            stroke(scenario, phase, from, to); drawn++
            compare(scenario, phase, ADDED)
            assertStrokeOnScreen(scenario, phase, from, to, 0xff14326e.toInt())
            val after = counts(scenario)
            assertEquals("$phase: the stroke is added to the picture. before: $before after: $after\n$notes", before.added + 1, after.added)
            assertEquals("$phase: the stroke did not make the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        }
        strokeAdded("first-stroke")

        // The activity is made again: a new canvas, a new render session, a new picture.
        scenario.recreate()
        await(scenario, "recreated", predicate = ::settled)
        compare(scenario, "after-recreation", REBUILT)
        strokeAdded("stroke-after-recreation")

        // The device is turned and turned back: the activity is made again
        // twice and the surface goes and comes back. (On this phone the
        // editor leaves the canvas no height in landscape, so nothing is drawn
        // there; the frames after turning back are what is checked.)
        scenario.onActivity { it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE }
        await(scenario, "landscape") { !it.editor.busy && it.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE }
        SystemClock.sleep(600)
        scenario.onActivity { it.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
        await(scenario, "portrait-again") { activity ->
            val view = canvas(activity.window.decorView)
            activity.resources.configuration.orientation == Configuration.ORIENTATION_PORTRAIT && view != null && view.height > view.width &&
                settled(activity) && activity.editor.scene?.camera?.let { it.viewportWidth.toInt() == view.width && it.viewportHeight.toInt() == view.height } == true
        }
        compare(scenario, "portrait-again", REBUILT)
        strokeAdded("stroke-after-turning")

        // The app is left and returned to, as with Home: the surface goes and comes back.
        scenario.moveToState(Lifecycle.State.CREATED)
        SystemClock.sleep(600)
        scenario.moveToState(Lifecycle.State.RESUMED)
        await(scenario, "returned", predicate = ::settled)
        compare(scenario, "after-returning", REBUILT)
        strokeAdded("stroke-after-returning")

        // The system asks for memory: the picture is given up, and made again for the next frame.
        run {
            val before = counts(scenario)
            scenario.onActivity { it.application.onTrimMemory(ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL) }
            compare(scenario, "after-memory-warning", REBUILT)
            val after = counts(scenario)
            assertTrue("A memory warning gives the picture up; the next frame makes it again. before: $before after: $after\n$notes",
                after.rebuilt > before.rebuilt)
        }
        strokeAdded("stroke-after-memory-warning")
        assertEquals(filled + drawn, items(scenario))
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    // A page of several layers, and changes that draw a part of the picture again.

    /** What [fillLayered] put in: how many objects, and the ones the tests change. */
    private class Layered(val count: Int, val faint: String, val highlight: String, val lone: String, val loneHighlight: String,
                          val box: String, val band: String, val column: String, val lowRow: String)

    /** Puts a page of three layers into the closed project through the core.
     * The layer the pen draws in, beneath: eight pen strokes, a filled
     * rectangle and a line. Above it a translucent layer (opacity 0.55) with
     * wide marker strokes across the pen strokes, and one alone on the right.
     * On top a highlighter layer (multiply) with two bands across both, and one
     * alone low on the right. Straight strokes, so that the middle of each
     * one's box lies on it. */
    private fun fillLayered(id: String): Layered = runBlocking {
        val project = core.open(projectPath(id))
        try {
            var info = project.info()
            val documentId = info.documentIds.first()
            val base = project.document(documentId).layers.first { it.visible && !it.locked && it.blend == "normal" }
            val controls = documentControlWorkflows(project)
            suspend fun control(command: DocumentControlCommand) {
                info = project.info()
                val now = System.currentTimeMillis()
                val plan = controls.prepare(WorkflowBinding(info.projectId, documentId, info.hostSeq, info.stateHash),
                    WorkflowMetadata(core.newId(now.toULong()), info.deviceId, info.nextLamport, now), command)
                try { plan.commit() } finally { plan.close() }
                info = project.info()
            }
            val faint = core.newId(System.currentTimeMillis().toULong())
            control(DocumentControlCommand.CreateAnnotationLayer(faint, "Faint"))
            control(DocumentControlCommand.UpdateLayer(LayerControlUpdate(faint, opacity = 0.55)))
            val highlight = core.newId(System.currentTimeMillis().toULong())
            control(DocumentControlCommand.CreateAnnotationLayer(highlight, "Highlight"))
            control(DocumentControlCommand.UpdateLayer(LayerControlUpdate(highlight, blend = "multiply")))
            val doc = project.document(documentId)
            val order = doc.layers.map { it.id }
            assertTrue("The pen's layer is beneath the translucent one, and that beneath the highlighter: $order",
                order.indexOf(base.id) in 0 until order.indexOf(faint) && order.indexOf(faint) < order.indexOf(highlight))
            assertEquals(0.55, doc.layers.first { it.id == faint }.opacity, 1e-6)
            assertEquals("multiply", doc.layers.first { it.id == highlight }.blend)
            val w = doc.width.toDouble(); val h = doc.height.toDouble()
            fun at(x: Double, y: Double) = Point(w * x, h * y)
            var count = 0
            suspend fun stroke(layer: String, family: String, width: Double, rgba: UInt, from: Point, to: Point): String {
                val now = System.currentTimeMillis()
                val objectId = core.newId(now.toULong())
                val run = project.beginStroke(StrokeOptions(core.newId(now.toULong()), core.newId(now.toULong()), objectId, documentId, layer,
                    info.deviceId, info.nextLamport, now, family, width, rgba))
                try {
                    val samples = 16
                    run.append(SampleBatch(1uL, DoubleArray(samples) { from.x + (to.x - from.x) * it / (samples - 1) },
                        DoubleArray(samples) { from.y + (to.y - from.y) * it / (samples - 1) }, UIntArray(samples) { (it * 8).toUInt() }, FloatArray(samples) { .6f }))
                    info = run.commit()
                } finally { run.dispose() }
                count++
                return objectId
            }
            val pens = listOf(0x1b2a3affu, 0xc0392bffu, 0x1f7a4dffu, 0x2456c8ffu)
            val rows = (0 until 8).map { k -> stroke(base.id, "pen", 6.0, pens[k % 4], at(0.1, 0.15 + 0.1 * k), at(0.6, 0.16 + 0.1 * k)) }
            val now = System.currentTimeMillis()
            val box = core.newId(now.toULong()); val line = core.newId(now.toULong() + 1uL)
            info = project.edit(EditOptions(core.newId(now.toULong()), documentId, info.deviceId, info.nextLamport, now), listOf(
                EditCommand.Create(box, base.id, Shape.Rectangle(Rect(w * 0.65, h * 0.45, w * 0.25, h * 0.15)), ObjectStyle(0x2456c8ffu, 4.0, fill = 0x8ecae6ffu)),
                EditCommand.Create(line, base.id, Shape.Line(listOf(at(0.65, 0.8), at(0.95, 0.9))), ObjectStyle(0x8e44adffu, 5.0))))
            count += 2
            val columns = listOf(0.2, 0.35, 0.5).map { x -> stroke(faint, "marker", 16.0, 0xe08a00ffu, at(x, 0.1), at(x + 0.01, 0.6)) }
            val lone = stroke(faint, "marker", 16.0, 0x8e44adffu, at(0.8, 0.1), at(0.81, 0.4))
            val bands = listOf(0.3, 0.5).map { y -> stroke(highlight, "highlighter", 20.0, 0xf4d03fffu, at(0.05, y), at(0.66, y + 0.005)) }
            val loneHighlight = stroke(highlight, "highlighter", 20.0, 0x58d68dffu, at(0.65, 0.7), at(0.95, 0.705))
            Layered(count, faint, highlight, lone, loneHighlight, box, bands[0], columns[1], rows[7])
        } finally { project.close() }
    }

    private fun pageSize(scenario: ActivityScenario<MainActivity>): Point {
        var page = Point(0.0, 0.0)
        scenario.onActivity { val doc = requireNotNull(it.editor.document); page = Point(doc.width.toDouble(), doc.height.toDouble()) }
        return page
    }
    /** The object with [id] in the scene the editor last gave the canvas. */
    private fun shown(activity: MainActivity, id: String): RenderItem? = activity.editor.scene?.objects?.firstOrNull { it.item.objectId == id }?.item
    private fun shown(scenario: ActivityScenario<MainActivity>, id: String): RenderItem? {
        var found: RenderItem? = null
        scenario.onActivity { found = shown(it, id) }
        return found
    }
    private fun centre(item: RenderItem) = Point(item.bounds.x + item.bounds.width / 2, item.bounds.y + item.bounds.height / 2)

    /** With the selection tool: a tap where nothing lies, so that nothing is
     * selected and no handle of an earlier selection lies under the next contact. */
    private fun deselect(scenario: ActivityScenario<MainActivity>, empty: Point) {
        contact(scenario, listOf(empty, empty))
        await(scenario, "nothing-selected") { settled(it) && it.editor.selected == null }
    }

    /** Edits the project the editor holds without the editor, as the paired
     * computer's session does, on a thread of its own. */
    private fun outside(scenario: ActivityScenario<MainActivity>, commands: (com.visualworkbench.shared.ProjectInfo) -> List<EditCommand>) {
        scenario.onActivity { activity ->
            val project = EditorController::class.java.getDeclaredField("project").apply { isAccessible = true }.get(activity.editor) as WorkbenchProject
            val documentId = requireNotNull(activity.editor.document).documentId
            Thread {
                runBlocking {
                    val info = project.info(); val now = System.currentTimeMillis()
                    project.edit(EditOptions(core.newId(now.toULong()), documentId, info.deviceId, info.nextLamport, now), commands(info))
                }
            }.start()
        }
    }

    /** What [fillMixed] put in: how many objects, the strokes, and the other objects. */
    private class Mixed(val count: Int, val strokes: List<String>, val box: String, val otherBox: String, val arrow: String, val text: String, val marker: String)

    /** Puts a page of one layer into the closed project through the core:
     * eight straight pen strokes on the left, two filled rectangles on the
     * right, an arrow and a text below them, and a numbered marker placed as
     * the instruction list places one. None of the other objects is a stroke:
     * each read of the document sends them whole. */
    private fun fillMixed(id: String): Mixed = runBlocking {
        val project = core.open(projectPath(id))
        try {
            var info = project.info()
            val documentId = info.documentIds.first()
            val doc = project.document(documentId)
            val layer = doc.layers.first { it.visible && !it.locked && it.blend == "normal" }
            val w = doc.width.toDouble(); val h = doc.height.toDouble()
            fun at(x: Double, y: Double) = Point(w * x, h * y)
            val strokes = (0 until 8).map { k ->
                val now = System.currentTimeMillis()
                val objectId = core.newId(now.toULong())
                val run = project.beginStroke(StrokeOptions(core.newId(now.toULong()), core.newId(now.toULong()), objectId, documentId, layer.id,
                    info.deviceId, info.nextLamport, now, "pen", 7.0, listOf(0x1b2a3affu, 0xc0392bffu, 0x1f7a4dffu, 0x2456c8ffu)[k % 4]))
                try {
                    val from = at(0.1, 0.15 + 0.05 * k); val to = at(0.45, 0.16 + 0.05 * k); val samples = 16
                    run.append(SampleBatch(1uL, DoubleArray(samples) { from.x + (to.x - from.x) * it / (samples - 1) },
                        DoubleArray(samples) { from.y + (to.y - from.y) * it / (samples - 1) }, UIntArray(samples) { (it * 8).toUInt() }, FloatArray(samples) { .6f }))
                    info = run.commit()
                } finally { run.dispose() }
                objectId
            }
            val now = System.currentTimeMillis()
            val box = core.newId(now.toULong()); val otherBox = core.newId(now.toULong() + 1uL)
            val arrow = core.newId(now.toULong() + 2uL); val text = core.newId(now.toULong() + 3uL)
            info = project.edit(EditOptions(core.newId(now.toULong()), documentId, info.deviceId, info.nextLamport, now), listOf(
                EditCommand.Create(box, layer.id, Shape.Rectangle(Rect(w * 0.55, h * 0.15, w * 0.3, h * 0.12)), ObjectStyle(0x2456c8ffu, 4.0, fill = 0x8ecae6ffu)),
                EditCommand.Create(otherBox, layer.id, Shape.Rectangle(Rect(w * 0.55, h * 0.35, w * 0.3, h * 0.1)), ObjectStyle(0xc0392bffu, 3.0, fill = 0xf5b7b1ffu)),
                EditCommand.Create(arrow, layer.id, Shape.Arrow(listOf(at(0.1, 0.62), at(0.45, 0.75))), ObjectStyle(0x8e44adffu, 5.0)),
                EditCommand.Create(text, layer.id, Shape.Text(at(0.55, 0.62), "Label", "Inter", h * 0.04), ObjectStyle(0x1f7a4dffu, 1.0))))
            val marker = core.newId(System.currentTimeMillis().toULong())
            val plan = instructionWorkflows(project).prepare(WorkflowBinding(info.projectId, documentId, info.hostSeq, info.stateHash),
                WorkflowMetadata(core.newId(System.currentTimeMillis().toULong()), info.deviceId, info.nextLamport, System.currentTimeMillis()),
                InstructionCommand.PlaceMarker(marker, core.newId(System.currentTimeMillis().toULong()), layer.id, at(0.7, 0.82), null, emptyList(),
                    ObjectStyle(0x2456c8ffu, 3.0), InstructionRole.Change, "Check this", InstructionEntryMethod.PhoneKeyboard))
            try { plan.commit() } finally { plan.close() }
            Mixed(strokes.size + 5, strokes, box, otherBox, arrow, text, marker)
        } finally { project.close() }
    }

    @Test fun changesBesideAMarkerShapesAndTextDrawAgainOnlyWhatTheyTouch() = withPage(::fillMixed, { it.count }) { scenario, mixed ->
        val page = pageSize(scenario)
        fun at(x: Double, y: Double) = Point(page.x * x, page.y * y)
        scenario.onActivity { assertTrue("The marker is on the page", shown(it, mixed.marker)?.shape is Shape.Marker) }
        compare(scenario, "opened", REBUILT)
        choose(scenario, EditorTool.Pen, 8.0, 0xff7a1420)
        compare(scenario, "pen-chosen", REBUILT)

        /** [change] (which waits until the editor shows it), then a comparison:
         * a part of the picture was drawn again, and the picture was not made
         * again, although the marker, the rectangles, the arrow and the text
         * came whole in the read that showed the change. */
        fun drawnAgain(phase: String, change: () -> Unit) {
            val before = counts(scenario)
            change()
            await(scenario, "drawn-again-$phase") { settled(it) && requireNotNull(canvas(it.window.decorView)).pictureCounts.patched.get() > before.patched }
            compare(scenario, phase, PATCHED)
            val after = counts(scenario)
            assertEquals("$phase must not make the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        }
        fun objects(): Int { var size = 0; scenario.onActivity { size = it.editor.scene?.objects?.size ?: -1 }; return size }

        stroke(scenario, "stroke", at(0.12, 0.56), at(0.44, 0.58))
        compare(scenario, "stroke", ADDED)
        val size = objects()
        drawnAgain("undo") {
            scenario.onActivity { it.editor.undo() }
            await(scenario, "undone") { settled(it) && it.editor.scene?.objects?.size == size - 1 }
        }
        // Part of a stroke erased. (The whole-object eraser cannot be used on
        // a page with a numbered marker: its search builds a path for every
        // object, and has none for a marker.)
        scenario.onActivity { it.editor.chooseEraser(EraserChoice.Stroke) }
        SystemClock.sleep(400); await(scenario, "stroke-eraser") { settled(it) && it.editor.erasers.state.value.ready }
        drawnAgain("erase-part") {
            contact(scenario, line(at(0.3, 0.13), at(0.3, 0.17), steps = 6))
            await(scenario, "erased") { settled(it) && shown(it, mixed.strokes[0]) == null }
        }
        scenario.onActivity { it.editor.chooseTool(EditorTool.Select) }
        SystemClock.sleep(400); await(scenario, "select-tool", predicate = ::settled)
        deselect(scenario, at(0.93, 0.97))
        val moved = requireNotNull(shown(scenario, mixed.strokes[3]))
        drawnAgain("move") {
            val grip = centre(moved)
            contact(scenario, line(grip, Point(grip.x + page.x * 0.03, grip.y + page.y * 0.02), steps = 8))
            await(scenario, "moved") { settled(it) && it.editor.scene?.transformed?.isEmpty() == true && shown(it, mixed.strokes[3])?.transform != moved.transform }
        }
        val remote = requireNotNull(shown(scenario, mixed.strokes[5]))
        val remoteTarget = EditorDrawing.compose(Transform(e = page.x * 0.04, f = page.y * 0.01), remote.transform)
        drawnAgain("computer-move") {
            outside(scenario) { listOf(EditCommand.SetTransform(mixed.strokes[5], remoteTarget)) }
            await(scenario, "computer-moved") { settled(it) && shown(it, mixed.strokes[5])?.transform == remoteTarget }
        }
        // A rectangle itself changed: drawn again where it lies.
        drawnAgain("rectangle-restyled") {
            outside(scenario) { listOf(EditCommand.SetStyle(mixed.box, ObjectStyle(0xe08a00ffu, 6.0, fill = 0xfad7a0ffu))) }
            await(scenario, "rectangle-restyled") { settled(it) && shown(it, mixed.box)?.style?.rgba == 0xe08a00ffu }
        }
        // The marker itself moved: its label is laid out on the phone and its
        // box does not tell where that lies, so the whole picture is made again.
        run {
            val before = counts(scenario)
            val placed = requireNotNull(shown(scenario, mixed.marker))
            val target = EditorDrawing.compose(Transform(e = -page.x * 0.1, f = -page.y * 0.03), placed.transform)
            outside(scenario) { listOf(EditCommand.SetTransform(mixed.marker, target)) }
            await(scenario, "marker-moved") { settled(it) && shown(it, mixed.marker)?.transform == target }
            await(scenario, "made-again-for-the-marker") { requireNotNull(canvas(it.window.decorView)).pictureCounts.rebuilt.get() > before.rebuilt }
            compare(scenario, "marker-moved", PATCHED)
        }
        // The marker, the rectangles, the arrow and the text read again with no change of their own.
        drawnAgain("after-the-marker") {
            outside(scenario) { listOf(EditCommand.Delete(mixed.strokes[7])) }
            await(scenario, "deleted-after-the-marker") { settled(it) && shown(it, mixed.strokes[7]) == null }
        }
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    @Test fun undoRedoErasingMovingTurningDeletingRestylingAndTheComputerDrawAgainOnlyWhatTheyTouch() = withPage(::fillLayered, { it.count }) { scenario, layered ->
        val page = pageSize(scenario)
        fun at(x: Double, y: Double) = Point(page.x * x, page.y * y)
        compare(scenario, "opened", LAYERED)
        choose(scenario, EditorTool.Pen, 8.0, 0xff7a1420)
        compare(scenario, "pen-chosen", LAYERED)

        /** [change] (which waits until the editor shows it), then a comparison:
         * a part of a layer's picture was drawn again, or a stroke added to it
         * (a stroke that comes back by a redo is added to its layer again), and
         * no picture was made again. */
        fun drawnAgain(phase: String, change: () -> Unit) {
            val before = counts(scenario)
            change()
            await(scenario, "drawn-again-$phase") { settled(it) && requireNotNull(canvas(it.window.decorView)).pictureCounts.let { counted ->
                counted.patched.get() > before.patched || counted.added.get() > before.added } }
            compare(scenario, phase, PATCHED)
            val after = counts(scenario)
            assertEquals("$phase must not make the picture again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
        }
        fun objects(): Int { var size = 0; scenario.onActivity { size = it.editor.scene?.objects?.size ?: -1 }; return size }

        // A pen stroke in the layer beneath the translucent and highlighter
        // layers: it is added to that layer's own picture.
        drawnAgain("stroke-beneath") { stroke(scenario, "stroke-beneath", at(0.08, 0.42), at(0.62, 0.47)) }
        var size = objects()
        drawnAgain("undo") {
            scenario.onActivity { it.editor.undo() }
            await(scenario, "undone") { settled(it) && it.editor.scene?.objects?.size == size - 1 }
        }
        size = objects()
        drawnAgain("redo") {
            scenario.onActivity { it.editor.redo() }
            await(scenario, "redone") { settled(it) && it.editor.scene?.objects?.size == size + 1 }
        }
        // The eraser: a whole translucent stroke, then part of a highlighter stroke.
        scenario.onActivity { it.editor.chooseEraser(EraserChoice.Object) }
        SystemClock.sleep(400); await(scenario, "object-eraser") { settled(it) && it.editor.erasers.state.value.ready }
        drawnAgain("erase-whole") {
            contact(scenario, line(at(0.74, 0.25), at(0.87, 0.25), steps = 8))
            await(scenario, "erased-whole") { settled(it) && shown(it, layered.lone) == null }
        }
        scenario.onActivity { it.editor.chooseEraser(EraserChoice.Stroke) }
        SystemClock.sleep(400); await(scenario, "stroke-eraser") { settled(it) && it.editor.erasers.state.value.ready }
        drawnAgain("erase-part") {
            contact(scenario, line(at(0.8, 0.66), at(0.8, 0.74), steps = 6))
            await(scenario, "erased-part") { settled(it) && shown(it, layered.loneHighlight) == null }
        }
        // The selection tool: the filled rectangle moved, turned and deleted.
        scenario.onActivity { it.editor.chooseTool(EditorTool.Select) }
        SystemClock.sleep(400); await(scenario, "select-tool", predicate = ::settled)
        deselect(scenario, at(0.93, 0.97))
        var box = requireNotNull(shown(scenario, layered.box))
        drawnAgain("move") {
            val grip = centre(box)
            contact(scenario, line(grip, Point(grip.x + page.x * 0.04, grip.y + page.y * 0.03), steps = 8))
            await(scenario, "moved") { settled(it) && it.editor.scene?.transformed?.isEmpty() == true && shown(it, layered.box)?.transform != box.transform }
        }
        box = requireNotNull(shown(scenario, layered.box))
        drawnAgain("turn") {
            var handle: com.visualworkbench.shared.ObjectRotationHandle? = null
            scenario.onActivity { handle = it.editor.scene?.rotationHandle }
            val turn = requireNotNull(handle) { "No rotation handle for the selected rectangle\n$notes" }
            val arm = Point(turn.point.x - turn.pivot.x, turn.point.y - turn.pivot.y)
            contact(scenario, (0..8).map { step -> val angle = 0.6 * step / 8
                Point(turn.pivot.x + arm.x * kotlin.math.cos(angle) - arm.y * sin(angle), turn.pivot.y + arm.x * sin(angle) + arm.y * kotlin.math.cos(angle)) })
            await(scenario, "turned") { settled(it) && it.editor.scene?.transformed?.isEmpty() == true && shown(it, layered.box)?.transform != box.transform }
        }
        drawnAgain("delete") {
            scenario.onActivity { it.editor.deleteSelected() }
            await(scenario, "deleted") { settled(it) && shown(it, layered.box) == null }
        }
        // A highlighter band restyled.
        val tap = at(0.63, 0.3015)
        contact(scenario, listOf(tap, tap))
        await(scenario, "band-selected") { settled(it) && it.editor.selected == layered.band }
        drawnAgain("restyle") {
            scenario.onActivity { it.editor.brush(BrushPreference(width = 24.0, colorArgb = 0xffaf7ac5)) }
            await(scenario, "restyled") { settled(it) && shown(it, layered.band)?.style?.rgba == EditorDrawing.rgba(0xffaf7ac5) }
        }
        // The paired computer moves a translucent stroke and deletes a pen stroke.
        val column = requireNotNull(shown(scenario, layered.column))
        val target = EditorDrawing.compose(Transform(e = page.x * 0.05, f = page.y * 0.01), column.transform)
        drawnAgain("computer-move") {
            outside(scenario) { listOf(EditCommand.SetTransform(layered.column, target)) }
            await(scenario, "computer-moved") { settled(it) && shown(it, layered.column)?.transform == target }
        }
        drawnAgain("computer-delete") {
            outside(scenario) { listOf(EditCommand.Delete(layered.lowRow)) }
            await(scenario, "computer-deleted") { settled(it) && shown(it, layered.lowRow) == null }
        }
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    @Test fun framesWhileAnObjectIsMovedTurnedOrErasedAreDrawnOverThePictureAsTheWholeScene() = withPage(::fillLayered, { it.count }) { scenario, layered ->
        val page = pageSize(scenario)
        fun at(x: Double, y: Double) = Point(page.x * x, page.y * y)
        scenario.onActivity { it.editor.chooseTool(EditorTool.Select) }
        SystemClock.sleep(400); await(scenario, "select-tool", predicate = ::settled)
        deselect(scenario, at(0.93, 0.97))
        compare(scenario, "opened", LAYERED)

        /** A comparison while the pen is still down: the frame was drawn over the picture, and the picture was not changed. */
        fun over(phase: String, before: Counts) {
            compare(scenario, phase, PATCHED, settle = false)
            val after = counts(scenario)
            note("phase=$phase area=${after.area}")
            assertTrue("$phase: frames are drawn over the picture. before: $before after: $after\n$notes", after.over > before.over)
            assertEquals("$phase: the picture is not made again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
            assertEquals("$phase: the picture is not changed. before: $before after: $after\n$notes", before.patched, after.patched)
        }

        // Moving: the pen is down on the rectangle and has moved, and stays down.
        val box = requireNotNull(shown(scenario, layered.box))
        val grip = centre(box)
        var before = counts(scenario)
        val step = Point(grip.x + page.x * 0.03, grip.y + page.y * 0.02)
        contact(scenario, line(grip, step, steps = 6), up = false)
        await(scenario, "moving") { it.editor.scene?.transformed?.containsKey(layered.box) == true }
        over("moving", before)
        // Further, across the pen strokes, the translucent columns and the highlighter bands.
        val far = at(0.3, 0.32)
        contact(scenario, line(step, far, steps = 10), down = false, up = false)
        await(scenario, "moving-far") { it.editor.scene?.transformed?.get(layered.box)?.let { t -> kotlin.math.abs(t.e - box.transform.e) > page.x * 0.2 } == true }
        over("moved-across-the-layers", before)
        contact(scenario, listOf(far, far), down = false)
        await(scenario, "move-saved") { settled(it) && it.editor.scene?.transformed?.isEmpty() == true && shown(it, layered.box)?.transform != box.transform }
        run {
            val saved = counts(scenario)
            await(scenario, "drawn-again-after-move") { requireNotNull(canvas(it.window.decorView)).pictureCounts.patched.get() > before.patched }
            compare(scenario, "move-saved", PATCHED)
            assertEquals("The move did not make the picture again. $saved ${counts(scenario)}\n$notes", before.rebuilt, counts(scenario).rebuilt)
        }

        // Turning: the pen is down on the rotation handle and has turned part way.
        val moved = requireNotNull(shown(scenario, layered.box))
        var handle: com.visualworkbench.shared.ObjectRotationHandle? = null
        scenario.onActivity { handle = it.editor.scene?.rotationHandle }
        val turn = requireNotNull(handle) { "No rotation handle for the selected rectangle\n$notes" }
        val arm = Point(turn.point.x - turn.pivot.x, turn.point.y - turn.pivot.y)
        val arc = (0..8).map { k -> val angle = 0.5 * k / 8
            Point(turn.pivot.x + arm.x * kotlin.math.cos(angle) - arm.y * sin(angle), turn.pivot.y + arm.x * sin(angle) + arm.y * kotlin.math.cos(angle)) }
        before = counts(scenario)
        contact(scenario, arc, up = false)
        await(scenario, "turning") { it.editor.scene?.transformed?.containsKey(layered.box) == true }
        over("turning", before)
        contact(scenario, listOf(arc.last(), arc.last()), down = false)
        await(scenario, "turn-saved") { settled(it) && it.editor.scene?.transformed?.isEmpty() == true && shown(it, layered.box)?.transform != moved.transform }
        compare(scenario, "turn-saved", PATCHED)

        // Erasing: the pen is down with the eraser over the lone translucent
        // stroke. This eraser saves at pen-up and changes nothing on the canvas
        // meanwhile (its sweep is drawn above it): the frames are the picture.
        scenario.onActivity { it.editor.chooseEraser(EraserChoice.Object) }
        SystemClock.sleep(400); await(scenario, "object-eraser") { settled(it) && it.editor.erasers.state.value.ready }
        before = counts(scenario)
        contact(scenario, line(at(0.74, 0.25), at(0.87, 0.25), steps = 8), up = false)
        await(scenario, "erasing") { it.editor.erasers.state.value.draft != null }
        compare(scenario, "erasing", PATCHED, settle = false)
        assertEquals("Nothing is made while the eraser is down. $before ${counts(scenario)}\n$notes", before.rebuilt + before.patched, counts(scenario).let { it.rebuilt + it.patched })
        contact(scenario, listOf(at(0.87, 0.25), at(0.87, 0.25)), down = false)
        await(scenario, "erased") { settled(it) && shown(it, layered.lone) == null }
        await(scenario, "drawn-again-after-erase") { requireNotNull(canvas(it.window.decorView)).pictureCounts.patched.get() > before.patched }
        compare(scenario, "erased", PATCHED)

        // Objects hidden while they are erased (as an eraser that hides what it
        // sweeps would show them), several at once, with one moved as well, and
        // then gone from the saved scene: drawn by a kept picture as the canvas
        // draws it, and compared with the same scenes drawn whole.
        probe(scenario, layered)
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    /** A layer command run on the project the editor holds, without the
     * editor, as the layers panel of the paired computer runs one. */
    private fun control(scenario: ActivityScenario<MainActivity>, command: DocumentControlCommand) {
        scenario.onActivity { activity ->
            val project = EditorController::class.java.getDeclaredField("project").apply { isAccessible = true }.get(activity.editor) as WorkbenchProject
            val documentId = requireNotNull(activity.editor.document).documentId
            Thread {
                runBlocking {
                    val info = project.info(); val now = System.currentTimeMillis()
                    val plan = documentControlWorkflows(project).prepare(WorkflowBinding(info.projectId, documentId, info.hostSeq, info.stateHash),
                        WorkflowMetadata(core.newId(now.toULong()), info.deviceId, info.nextLamport, now), command)
                    try { plan.commit() } finally { plan.close() }
                }
            }.start()
        }
    }

    private fun layerDrawn(scenario: ActivityScenario<MainActivity>): Long {
        var drawn = 0L
        scenario.onActivity { drawn = layerDrawn(it) }
        return drawn
    }
    private fun layerDrawn(activity: MainActivity): Long = requireNotNull(canvas(activity.window.decorView)).pictureCounts.layers.get()

    @Test fun strokesOnALowerAHighlighterAndATranslucentLayerAreAddedToTheirOwnLayersPicture() = withPage(::fillLayered, { it.count }) { scenario, layered ->
        val page = pageSize(scenario)
        fun at(x: Double, y: Double) = Point(page.x * x, page.y * y)
        compare(scenario, "opened", LAYERED)
        choose(scenario, EditorTool.Pen, 8.0, 0xff7a1420)
        compare(scenario, "pen-chosen", LAYERED)
        var base = ""
        scenario.onActivity { base = requireNotNull(it.editor.document).layers.first { layer -> layer.visible && !layer.locked && layer.blend == "normal" }.id }

        /** A stroke from [from] to [to] with the chosen tool lands in [layer]:
         * added to that layer's picture once, nothing drawn again or made again. */
        fun added(phase: String, layer: String, from: Point, to: Point) {
            val before = counts(scenario); val layersBefore = layerDrawn(scenario)
            stroke(scenario, phase, from, to)
            compare(scenario, phase, ADDED)
            val after = counts(scenario)
            var newest: RenderItem? = null
            scenario.onActivity { activity -> newest = activity.editor.scene?.objects?.lastOrNull { it.item.layerId == layer }?.item }
            assertNotNull("$phase: the stroke is in its layer\n$notes", newest)
            assertEquals("$phase: drawn into its layer's picture once. before: $before after: $after\n$notes", before.added + 1, after.added)
            assertEquals("$phase: no picture made again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
            assertEquals("$phase: no part drawn again. before: $before after: $after\n$notes", before.patched, after.patched)
            assertEquals("$phase: no layer's picture drawn whole\n$notes", layersBefore, layerDrawn(scenario))
        }
        /** Ink in progress in a layer beneath others or composited otherwise: drawn in its layer's place over the pictures. */
        fun inkInProgress(phase: String, from: Point, to: Point) {
            await(scenario, "ready-for-$phase", predicate = ::settled)
            val before = counts(scenario)
            contact(scenario, line(from, to), up = false)
            await(scenario, "ink-$phase") { it.editor.scene?.provisional?.isNotEmpty() == true }
            compare(scenario, phase, ADDED, settle = false)
            assertTrue("$phase: the ink frame was drawn from the pictures. before: $before ${counts(scenario)}\n$notes", counts(scenario).frames > before.frames)
            val held = items(scenario)
            // The pen-down's events are dated up to 96 ms ahead; the pen lifts after the last of them, as time only goes forward.
            SystemClock.sleep(150)
            contact(scenario, listOf(to, to), down = false)
            await(scenario, "saved-$phase") { settled(it) && it.editor.document?.render?.items?.size == held + 1 }
        }

        // The pen's layer lies beneath the translucent and the highlighter layers.
        added("pen-beneath", base, at(0.08, 0.42), at(0.62, 0.47))
        inkInProgress("ink-beneath", at(0.1, 0.85), at(0.55, 0.62))
        // The highlighter draws in the multiply layer.
        choose(scenario, EditorTool.Highlighter, 18.0, 0xfff4d03f)
        added("highlighter", layered.highlight, at(0.06, 0.38), at(0.64, 0.4))
        inkInProgress("ink-highlighter", at(0.08, 0.2), at(0.6, 0.25))
        // With the pen's layer locked, the pen draws in the translucent layer.
        control(scenario, DocumentControlCommand.UpdateLayer(LayerControlUpdate(base, locked = true)))
        await(scenario, "base-locked") { settled(it) && it.editor.document?.layers?.firstOrNull { layer -> layer.id == base }?.locked == true }
        choose(scenario, EditorTool.Pen, 12.0, 0xff1f7a4d)
        added("pen-translucent", layered.faint, at(0.15, 0.12), at(0.4, 0.55))
        inkInProgress("ink-translucent", at(0.3, 0.1), at(0.45, 0.58))
        // Back to the lowest layer and to the highlighter: each stroke still goes into its own layer's picture.
        control(scenario, DocumentControlCommand.UpdateLayer(LayerControlUpdate(base, locked = false)))
        await(scenario, "base-unlocked") { settled(it) && it.editor.document?.layers?.firstOrNull { layer -> layer.id == base }?.locked == false }
        added("pen-beneath-again", base, at(0.12, 0.72), at(0.58, 0.76))
        choose(scenario, EditorTool.Highlighter, 18.0, 0xff58d68d)
        added("highlighter-again", layered.highlight, at(0.06, 0.58), at(0.64, 0.6))
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    @Test fun hidingShowingAndChangingALayersOpacityBlendOrPlaceOnlyCompositesItsPicture() = withPage(::fillLayered, { it.count }) { scenario, layered ->
        compare(scenario, "opened", LAYERED)
        choose(scenario, EditorTool.Pen, 8.0, 0xff7a1420)
        compare(scenario, "pen-chosen", LAYERED)
        fun shownIn(activity: MainActivity, layer: String) = activity.editor.scene?.objects?.count { it.item.layerId == layer } ?: -1

        /** [command], and the wait until the editor shows it: the pictures are
         * composited otherwise, and none is drawn again, in part or whole,
         * except [drawn] layers' pictures drawn whole. */
        fun composited(phase: String, command: DocumentControlCommand, drawn: Int = 0, done: (MainActivity) -> Boolean) {
            val before = counts(scenario); val layersBefore = layerDrawn(scenario)
            control(scenario, command)
            await(scenario, "shown-$phase") { settled(it) && done(it) }
            compare(scenario, phase, LAYERED)
            val after = counts(scenario)
            assertEquals("$phase: no picture made again. before: $before after: $after\n$notes", before.rebuilt, after.rebuilt)
            assertEquals("$phase: no part drawn again. before: $before after: $after\n$notes", before.patched, after.patched)
            assertEquals("$phase: no stroke added. before: $before after: $after\n$notes", before.added, after.added)
            assertEquals("$phase: layers' pictures drawn whole\n$notes", layersBefore + drawn, layerDrawn(scenario))
        }
        composited("highlight-hidden", DocumentControlCommand.UpdateLayer(LayerControlUpdate(layered.highlight, visible = false))) { shownIn(it, layered.highlight) == 0 }
        // A layer shown again: its objects come back from the core as new ones, and only its picture is drawn.
        composited("highlight-shown", DocumentControlCommand.UpdateLayer(LayerControlUpdate(layered.highlight, visible = true)), drawn = 1) { shownIn(it, layered.highlight) > 0 }
        composited("faint-fainter", DocumentControlCommand.UpdateLayer(LayerControlUpdate(layered.faint, opacity = 0.3))) { activity ->
            activity.editor.scene?.objects?.firstOrNull { it.item.layerId == layered.faint }?.item?.layerOpacity?.let { kotlin.math.abs(it - 0.3) < 1e-3 } == true }
        composited("faint-multiply", DocumentControlCommand.UpdateLayer(LayerControlUpdate(layered.faint, blend = "multiply"))) { activity ->
            activity.editor.scene?.objects?.firstOrNull { it.item.layerId == layered.faint }?.item?.layerBlend == "multiply" }
        composited("highlight-beneath-faint", DocumentControlCommand.PlaceLayer(layered.highlight, layered.faint)) { activity ->
            activity.editor.document?.layers?.map { it.id }?.let { ids -> ids.indexOf(layered.highlight) in 0 until ids.indexOf(layered.faint) } == true }
        composited("faint-plain-again", DocumentControlCommand.UpdateLayer(LayerControlUpdate(layered.faint, opacity = 1.0, blend = "normal"))) { activity ->
            activity.editor.scene?.objects?.firstOrNull { it.item.layerId == layered.faint }?.item?.layerOpacity?.let { kotlin.math.abs(it - 1.0) < 1e-3 } == true }
        // Strokes after all of it still go into their layer's picture.
        val page = pageSize(scenario)
        val before = counts(scenario)
        stroke(scenario, "after-composites", Point(page.x * 0.08, page.y * 0.42), Point(page.x * 0.62, page.y * 0.47))
        compare(scenario, "after-composites", ADDED)
        assertEquals(before.added + 1, counts(scenario).added)
        assertEquals(before.rebuilt, counts(scenario).rebuilt)
        assertEquals("No picture failed to draw. ${counts(scenario)}\n$notes", 0L, counts(scenario).failed)
    }

    /** Draws [layered]'s page as the editor shows it now into a [KeptPicture]
     * on a drawing thread of its own, then frames of scenes made from it, each
     * from the picture as the canvas would draw it and whole, and compares
     * them. Everything made here is given up before it returns. */
    private fun probe(scenario: ActivityScenario<MainActivity>, layered: Layered) {
        await(scenario, "ready-to-probe", predicate = ::settled)
        var shownScene: CanvasScene? = null; var surround = 0
        scenario.onActivity { shownScene = it.editor.scene; surround = EditorDrawing.surroundArgb(it.editor.preferences.darkTheme) }
        val scene = requireNotNull(shownScene)
        val band = scene.objects.first { it.item.objectId == layered.band }.item
        val turnedBand = EditorDrawing.compose(Transform(e = scene.document.width.toDouble() * 0.1, f = scene.document.height.toDouble() * 0.15), band.transform)
        val hidden = scene.objects.filter { it.item.layerId == layered.faint }.map { it.item.objectId }
        val variants = listOf(
            Triple("hidden-one", scene.copy(revision = scene.revision + 1, hidden = setOf(layered.column)), false),
            Triple("hidden-all-translucent", scene.copy(revision = scene.revision + 2, hidden = hidden.toSet()), false),
            Triple("hidden-and-moved", scene.copy(revision = scene.revision + 3, hidden = setOf(layered.column), transformed = mapOf(layered.band to turnedBand)), false),
            Triple("gone", scene.copy(revision = scene.revision + 4, objects = scene.objects.filter { it.item.objectId != layered.column }), true))
        val width = scene.camera.viewportWidth.toInt(); val height = scene.camera.viewportHeight.toInt()
        val thread = HandlerThread("vw-kept-probe").apply { start() }
        val handler = Handler(thread.looper)
        var surface: PictureSurface? = null; var picture: KeptPicture? = null
        fun onThread(work: () -> Unit) {
            val done = CountDownLatch(1); val error = AtomicReference<Throwable?>(null)
            handler.post { try { work() } catch (t: Throwable) { error.set(t) } finally { done.countDown() } }
            assertTrue("The probe's drawing thread did not answer", done.await(30, TimeUnit.SECONDS))
            error.get()?.let { throw it }
        }
        try {
            onThread {
                surface = PictureSurface(width, height); picture = KeptPicture()
                requireNotNull(requireNotNull(surface).render { assertTrue("The page at rest is drawn from the picture", requireNotNull(picture).draw(it, scene, true, false, false)) }).release()
            }
            for ((name, drawn, steady) in variants) {
                var compared: PictureComparison? = null; var fromPicture = false; var counted = ""
                onThread {
                    val target = requireNotNull(surface); val kept = requireNotNull(picture)
                    val before = kept.counts.over.get() + kept.counts.patched.get()
                    val frame = requireNotNull(target.render { fromPicture = kept.draw(it, drawn, steady, false, false) })
                    val whole = requireNotNull(target.render { PrecisionDrawing.draw(it, drawn) })
                    try {
                        compared = PictureComparison(requireNotNull(frame.bitmap.copy(Bitmap.Config.ARGB_8888, false)),
                            requireNotNull(whole.bitmap.copy(Bitmap.Config.ARGB_8888, false)), drawn.matrix, drawn.document.width.toDouble(), drawn.document.height.toDouble())
                    } finally { frame.release(); whole.release() }
                    counted = "${kept.counts} drawn_again=${kept.counts.over.get() + kept.counts.patched.get() - before}"
                }
                assertTrue("probe-$name: drawn from the picture ($counted)\n$notes", fromPicture)
                assertTrue("probe-$name: a part was drawn again ($counted)\n$notes", counted.endsWith("drawn_again=1"))
                val found = difference(requireNotNull(compared), surround)
                note("phase=probe-$name $found $counted")
                assertTrue("probe-$name: nothing but page and surround was drawn. $found\n$notes", found.inked > 500)
                assertTrue("probe-$name: the frame from the kept picture is not the whole scene (allowed: $PATCHED). $found\n$notes", PATCHED.allows(found))
            }
        } finally {
            onThread { picture?.discard(); surface?.close() }
            thread.quitSafely(); thread.join(5_000)
        }
    }

    private companion object {
        const val TAG = "VW_KEPT_PICTURE"
        /** A frame that follows a picture made from every object is the whole scene. */
        val REBUILT = Bound(steps = 0, coarse = 0, coarseSteps = 0)
        /** On a page of several layers, each layer's picture is drawn alone,
         * and the renderer may smooth an outline drawn alone otherwise than one
         * drawn with everything else, as for a stroke added to a picture
         * ([ADDED]): measured on the S23, one or two pixels of a page, at the
         * edge of a highlighter stroke. */
        val LAYERED by lazy { ADDED }
        /** After strokes have been added to the picture: rounding at their edges. */
        const val COARSE_PIXELS = 8
        const val COARSE_STEPS = 64
        val ADDED = Bound(steps = 1, coarse = COARSE_PIXELS, coarseSteps = COARSE_STEPS)
        /** After a part of the picture was drawn again, or while one is drawn over it: the same bound. */
        val PATCHED = ADDED
        /** Pixels this close to the page's edge are cut by its anti-aliased outline. */
        const val EDGE_PX = 2.0
    }
}
