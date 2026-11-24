@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.android

import android.app.Instrumentation
import android.content.pm.ActivityInfo
import android.content.res.Configuration
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
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.BrushPreference
import com.visualworkbench.android.editor.CanvasScene
import com.visualworkbench.android.editor.CanvasSurface
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.EditorDrawing
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.android.editor.KeptPicture
import com.visualworkbench.android.editor.PictureSurface
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.android.precision.PrecisionDrawing
import com.visualworkbench.shared.EditCommand
import com.visualworkbench.shared.EditOptions
import com.visualworkbench.shared.VectorEraseEdit
import com.visualworkbench.shared.VectorEraseTarget
import com.visualworkbench.shared.WorkflowBinding
import com.visualworkbench.shared.WorkflowMetadata
import com.visualworkbench.shared.vectorEraser
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.RenderItem
import com.visualworkbench.shared.SampleBatch
import com.visualworkbench.shared.StrokeOptions
import com.visualworkbench.shared.Transform
import com.visualworkbench.shared.WorkbenchProject
import com.visualworkbench.shared.workbenchCore
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt

/** A stroke, and any other change to the page, must cost the same on a page
 * that holds two hundred strokes and on one that holds five thousand. The same
 * work is done on both, through the real app, its canvas view's own touch
 * dispatch and a real native project filled through the core, and what it
 * costs is compared between the two:
 *
 * - the time to draw one ink frame, and from a pen sample to the frame that
 *   shows it being ready;
 * - from a stroke being saved to the editor having shown it, and the time to
 *   draw one saved frame and to have it ready;
 * - measured here: from the pen lifting to the canvas having drawn the newest
 *   saved scene with nothing left in flight;
 * - for undo, redo, erasing a whole stroke and part of one, moving, turning,
 *   deleting and restyling a stroke, and a stroke moved by the paired computer
 *   (an edit the editor did not make, applied to its project): from the change
 *   to the canvas having drawn the newest saved scene, and the longest saved
 *   frame drawn meanwhile (the one that brings the picture up to date);
 * - while a stroke is being moved: the time to draw one frame, drawn as the
 *   canvas draws it (from its kept picture where it can, whole where it
 *   cannot) and finished by the graphics processor.
 *
 * The page of five thousand may take at most [FACTOR] times as long as the
 * page of two hundred, plus a small fixed allowance for timer resolution and
 * noise, named with each check. All numbers are in the instrumentation's
 * output and the log before anything is checked, and in each failure message.
 * The time from a change to its saved scene being drawn is measured and
 * reported, not checked: it holds the editor's reading and preparing the
 * changed document and bringing its other lists up to date, which grows with
 * the page outside the canvas. What the canvas does for the change is the
 * longest saved frame, which is checked. The changed strokes lie in the band
 * the page leaves empty, also when moved or turned, so that the part drawn
 * again holds as many objects on both pages.
 *
 * Synthetic pen events on one phone: this compares two pages on one build. It
 * says nothing about a pen's feel, and its absolute numbers belong to that
 * phone.
 *
 * To measure an older build, copy this one file into its tree and run it
 * there: it uses nothing but what the app has had since the kept picture was
 * introduced (D25 step 2), and reads the pen timings from the text the
 * diagnostics screen copies, by name. On a build that draws every object for
 * a frame the comparison fails, and the failure message is the measurement. */
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
     * is then the same on both pages; the same holds for the strokes that are
     * changed, which lie there too. What is compared is what the rest of the
     * page costs. */
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

    /** The changes measured on each page, in the order they are made. */
    private enum class Change { Undo, Redo, EraseWhole, ErasePart, Move, Rotate, Delete, Edit, Computer }

    /** Each time one kind of change was made: from the change to the newest
     * saved scene drawn ([shownMs]), and the longest saved frame drawn
     * meanwhile ([frameMs]). */
    private class Changed(val shownMs: List<Long>, val frameMs: List<Double>, val pictures: List<String>) {
        val shown: Double get() = median(shownMs.map { it.toDouble() })
        val frame: Double get() = median(frameMs)
        override fun toString() = "median=$shown ms frame=$frame ms (each: ${shownMs.joinToString("/")} ms, frames ${frameMs.joinToString("/")} ms; " +
            "pictures ${pictures.joinToString(" / ")})"
    }

    /** What one page cost. [shownMs]: for each stroke, from the pen lifting to
     * the canvas having drawn the newest saved scene with nothing in flight.
     * [moveFrameMs] and [moveRecordMs]: each frame of a stroke being moved,
     * drawn and finished, and the part of that spent recording it. */
    private class Cost(val page: Int, val strokes: Int, val timings: Map<String, Timing>, val shownMs: List<Long>, val report: String,
                       val changes: Map<Change, Changed>, val moveFrameMs: List<Double>, val moveRecordMs: List<Double>) {
        val shownMedian: Double get() = shownMs.sorted().let { if (it.isEmpty()) Double.NaN else it[it.size / 2].toDouble() }
        val shownWorst: Long get() = shownMs.maxOrNull() ?: -1
        fun p50(key: String): Double? = timings[key]?.p50
        override fun toString(): String = "page of $page strokes, $strokes drawn: pen-up to saved scene drawn median=${shownMedian} ms worst=$shownWorst ms (n=${shownMs.size}); " +
            KEYS.joinToString("; ") { key -> "$key ${timings[key] ?: "not measured by this build"}" } +
            "\n  " + Change.entries.joinToString("\n  ") { "${it.name}: ${changes[it] ?: "not measured"}" } +
            "\n  live move frame p50=${median(moveFrameMs)} ms max=${moveFrameMs.maxOrNull()} ms, recorded p50=${median(moveRecordMs)} ms (n=${moveFrameMs.size})"
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

    // The changes. Every point is in document space; the page's size is [page].

    private fun items(scenario: ActivityScenario<MainActivity>): List<RenderItem> {
        var read: List<RenderItem> = emptyList()
        scenario.onActivity { read = it.editor.document?.render?.items.orEmpty() }
        return read
    }
    /** The object with [id] in the scene the editor last published to the canvas: a change counts once the canvas has been given it. */
    private fun item(activity: MainActivity, id: String): RenderItem? = activity.editor.scene?.objects?.firstOrNull { it.item.objectId == id }?.item

    /** One pen contact through the canvas view's own touch dispatch, in one main-thread turn. */
    private fun contact(activity: MainActivity, points: List<Point>) {
        val surface = requireNotNull(canvas(activity.window.decorView))
        val matrix = requireNotNull(activity.editor.scene).matrix
        val start = SystemClock.uptimeMillis()
        points.forEachIndexed { index, point ->
            val action = when (index) { 0 -> MotionEvent.ACTION_DOWN; points.lastIndex -> MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
            val screen = EditorDrawing.map(matrix, point)
            val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_STYLUS })
            val coordinates = arrayOf(MotionEvent.PointerCoords().apply { x = screen.x.toFloat(); y = screen.y.toFloat(); pressure = .6f; size = .01f })
            val event = MotionEvent.obtain(start, start + index * 8L, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
            try { assertTrue(surface.dispatchTouchEvent(event)) } finally { event.recycle() }
        }
    }
    private fun line(from: Point, to: Point, steps: Int = 12): List<Point> =
        (0..steps).map { Point(from.x + (to.x - from.x) * it / steps, from.y + (to.y - from.y) * it / steps) }

    private fun tool(scenario: ActivityScenario<MainActivity>, choose: (EditorController) -> Unit) {
        scenario.onActivity { choose(it.editor) }
        // Choosing a tool can show other controls and so resize the canvas.
        SystemClock.sleep(400)
        await(scenario, "tool-chosen", predicate = ::canvasReady)
    }

    /** A straight pen stroke across the band at [y] (a fraction of the page's height); returns its id. */
    private fun stroke(scenario: ActivityScenario<MainActivity>, page: Point, y: Double): String {
        await(scenario, "ready-to-draw", predicate = ::canvasReady)
        val before = items(scenario).map { it.objectId }.toSet()
        // Short enough that the stroke stays within the band when it is moved or turned.
        scenario.onActivity { contact(it, line(Point(page.x * 0.4, page.y * y), Point(page.x * 0.6, page.y * y))) }
        await(scenario, "stroke-saved") { canvasReady(it) && it.editor.document?.render?.items?.size == before.size + 1 }
        return items(scenario).map { it.objectId }.single { it !in before }
    }
    private fun centre(item: RenderItem) = Point(item.bounds.x + item.bounds.width / 2, item.bounds.y + item.bounds.height / 2)

    /** With the selection tool: a tap on an empty part of the band, so that
     * nothing is selected and no handle of an earlier selection lies under
     * the next contact. */
    private fun deselect(scenario: ActivityScenario<MainActivity>, page: Point) {
        val empty = Point(page.x * 0.92, page.y * 0.44)
        scenario.onActivity { contact(it, listOf(empty, empty)) }
        await(scenario, "nothing-selected") { canvasReady(it) && it.editor.selected == null }
    }

    /** Times one change: [act] on the main thread, then from it until [done]
     * holds and the canvas has drawn the newest saved scene with nothing in
     * flight, looked for once a millisecond. The pen timings are cleared
     * first, so that the longest saved frame read afterwards is one drawn
     * for this change. */
    private fun timeChange(scenario: ActivityScenario<MainActivity>, phase: String, into: MutableMap<Change, Changed>, change: Change,
                           act: (MainActivity) -> Unit, done: (MainActivity) -> Boolean) {
        await(scenario, "ready-for-$phase", predicate = ::canvasReady)
        // Apart from what came before, as a change made by hand is.
        SystemClock.sleep(PAUSE_MS)
        val finished = CountDownLatch(1)
        var took = -1L
        var counted = LongArray(4)
        scenario.onActivity { activity ->
            val editor = activity.editor
            val surface = requireNotNull(canvas(activity.window.decorView))
            val main = Handler(Looper.getMainLooper())
            editor.diagnostics.pen.clear()
            counted = surface.pictureCounts.let { longArrayOf(it.rebuilt.get(), it.added.get(), it.frames.get(), it.whole.get()) }
            val start = SystemClock.uptimeMillis()
            act(activity)
            val look = object : Runnable {
                override fun run() {
                    val now = SystemClock.uptimeMillis()
                    if (done(activity) && editor.pending == 0 && !editor.busy && editor.scene?.transformed?.isEmpty() == true &&
                        savedSceneDrawn(surface.lifeState())) { took = now - start; finished.countDown() }
                    else if (now - start > CHANGE_TIMEOUT_MS) finished.countDown()
                    else main.postDelayed(this, 1)
                }
            }
            main.post(look)
        }
        assertTrue("$phase: not looked for in time", finished.await(CHANGE_TIMEOUT_MS + 5_000, TimeUnit.MILLISECONDS))
        if (took < 0) {
            var detail = ""
            scenario.onActivity { activity ->
                val editor = activity.editor
                detail = "busy=${editor.busy} pending=${editor.pending} message=${editor.message} eraser=${editor.erasers.state.value} " +
                    "selected=${editor.selected} transformed=${editor.scene?.transformed?.keys} canvas=${canvas(activity.window.decorView)?.lifeState()}"
            }
            fail("$phase: the change was not drawn within $CHANGE_TIMEOUT_MS ms: $detail")
        }
        var report = ""
        scenario.onActivity { report = it.editor.diagnostics.pen.report(emptyList()) }
        val frame = timings(report)["canvas_saved_draw"]?.max ?: Double.NaN
        // What the kept picture did meanwhile: pictures made from every object, strokes added, frames from it, frames drawn whole.
        var pictures = ""
        scenario.onActivity { activity ->
            val now = requireNotNull(canvas(activity.window.decorView)).pictureCounts
            pictures = "rebuilt+${now.rebuilt.get() - counted[0]} added+${now.added.get() - counted[1]} frames+${now.frames.get() - counted[2]} whole+${now.whole.get() - counted[3]}"
        }
        val earlier = into[change]
        into[change] = Changed((earlier?.shownMs ?: emptyList()) + took, (earlier?.frameMs ?: emptyList()) + frame, (earlier?.pictures ?: emptyList()) + pictures)
    }

    /** The project the editor holds. Edited here without the editor, as the
     * paired computer's session edits it, and as the eraser does once it has
     * found what to erase. */
    private fun heldProject(editor: EditorController): WorkbenchProject =
        EditorController::class.java.getDeclaredField("project").apply { isAccessible = true }.get(editor) as WorkbenchProject

    /** Runs [edit] on the held project on a thread of its own, with the project's revision as it stands. */
    private fun outside(activity: MainActivity, edit: suspend (WorkbenchProject, String, com.visualworkbench.shared.ProjectInfo) -> Unit) {
        val project = heldProject(activity.editor)
        val documentId = requireNotNull(activity.editor.document).documentId
        Thread { runBlocking { edit(project, documentId, project.info()) } }.start()
    }

    /** Every kind of change, [ROUNDS] times each, on strokes drawn in the empty band. */
    private fun measureChanges(scenario: ActivityScenario<MainActivity>): Map<Change, Changed> {
        val measured = LinkedHashMap<Change, Changed>()
        var page = Point(0.0, 0.0)
        scenario.onActivity { val doc = requireNotNull(it.editor.document); page = Point(doc.width.toDouble(), doc.height.toDouble()) }
        tool(scenario) { it.chooseTool(EditorTool.Pen); it.brush(BrushPreference(width = 10.0, colorArgb = 0xff14326e)) }
        repeat(ROUNDS) { round ->
            // Undo and redo of a stroke just drawn.
            stroke(scenario, page, 0.47 + 0.01 * round)
            var count = 0; scenario.onActivity { count = it.editor.scene?.objects?.size ?: -1 }
            timeChange(scenario, "undo-$round", measured, Change.Undo, { it.editor.undo() }) { it.editor.scene?.objects?.size == count - 1 }
            scenario.onActivity { count = it.editor.scene?.objects?.size ?: -1 }
            timeChange(scenario, "redo-$round", measured, Change.Redo, { it.editor.redo() }) { it.editor.scene?.objects?.size == count + 1 }

            // A whole stroke erased, then part of one, saved as the eraser
            // saves them once it has found what its sweep touched. (The
            // eraser's own search takes pages of at most 512 objects for
            // whole strokes and 4,096 for parts of them; it is not the
            // canvas's.)
            val whole = stroke(scenario, page, 0.52)
            timeChange(scenario, "erase-whole-$round", measured, Change.EraseWhole, { activity ->
                outside(activity) { project, documentId, info ->
                    val now = System.currentTimeMillis()
                    project.edit(EditOptions(core.newId(now.toULong()), documentId, info.deviceId, info.nextLamport, now, info.hostSeq, info.stateHash),
                        listOf(EditCommand.Delete(whole)))
                }
            }) { item(it, whole) == null }
            val part = stroke(scenario, page, 0.56)
            timeChange(scenario, "erase-part-$round", measured, Change.ErasePart, { activity ->
                outside(activity) { project, documentId, info ->
                    val now = System.currentTimeMillis()
                    project.vectorEraser().erase(VectorEraseEdit(WorkflowBinding(info.projectId, documentId, info.hostSeq, info.stateHash),
                        WorkflowMetadata(core.newId(now.toULong()), info.deviceId, info.nextLamport, now),
                        listOf(VectorEraseTarget(part, core.newId(now.toULong()))),
                        line(Point(page.x * 0.5, page.y * 0.54), Point(page.x * 0.5, page.y * 0.58), steps = 6), 12.0))
                }
            }) { item(it, part) == null }

            // A stroke moved, turned and deleted with the selection tool.
            val moved = stroke(scenario, page, 0.6)
            tool(scenario) { it.chooseTool(EditorTool.Select) }
            deselect(scenario, page)
            var before = items(scenario).first { it.objectId == moved }
            val grip = centre(before)
            timeChange(scenario, "move-$round", measured, Change.Move,
                { contact(it, line(grip, Point(grip.x + page.x * 0.04, grip.y + page.y * 0.02), steps = 8)) }) { item(it, moved)?.transform != before.transform }
            before = items(scenario).first { it.objectId == moved }
            var handle: com.visualworkbench.shared.ObjectRotationHandle? = null
            scenario.onActivity { handle = it.editor.scene?.rotationHandle }
            val turn = requireNotNull(handle) { "No rotation handle for the selected stroke" }
            val arm = Point(turn.point.x - turn.pivot.x, turn.point.y - turn.pivot.y)
            val arc = (0..8).map { step -> val angle = 0.7 * step / 8
                Point(turn.pivot.x + arm.x * cos(angle) - arm.y * sin(angle), turn.pivot.y + arm.x * sin(angle) + arm.y * cos(angle)) }
            timeChange(scenario, "rotate-$round", measured, Change.Rotate, { contact(it, arc) }) { item(it, moved)?.transform != before.transform }
            timeChange(scenario, "delete-$round", measured, Change.Delete, { it.editor.deleteSelected() }) { item(it, moved) == null }
            tool(scenario) { it.chooseTool(EditorTool.Pen) }

            // A stroke restyled, then moved by the paired computer.
            val styled = stroke(scenario, page, 0.65)
            tool(scenario) { it.chooseTool(EditorTool.Select) }
            deselect(scenario, page)
            val tap = centre(items(scenario).first { it.objectId == styled })
            scenario.onActivity { contact(it, listOf(tap, tap)) }
            await(scenario, "selected-to-restyle") { canvasReady(it) && it.editor.selected == styled }
            val colour = listOf(0xffc0392bL, 0xff1f7a4dL, 0xff8e44adL)[round % 3]
            timeChange(scenario, "edit-$round", measured, Change.Edit,
                { it.editor.brush(BrushPreference(width = 14.0, colorArgb = colour)) }) { item(it, styled)?.style?.rgba == EditorDrawing.rgba(colour) }
            val held = items(scenario).first { it.objectId == styled }
            val target = EditorDrawing.compose(Transform(e = page.x * 0.03, f = -page.y * 0.01), held.transform)
            timeChange(scenario, "computer-$round", measured, Change.Computer, { activity ->
                // Another device's edit, as a session applies one.
                outside(activity) { project, documentId, info ->
                    val now = System.currentTimeMillis()
                    project.edit(EditOptions(core.newId(now.toULong()), documentId, info.deviceId, info.nextLamport, now),
                        listOf(EditCommand.SetTransform(styled, target)))
                }
            }) { item(it, styled)?.transform == target }
            tool(scenario) { it.chooseTool(EditorTool.Pen) }
        }
        return measured
    }

    /** Frames of a stroke being moved, drawn as the canvas draws them: from a
     * kept picture where it can ([KeptPicture.draw]), whole where it cannot.
     * Each frame is drawn off screen by the same renderer as the canvas, and
     * timed until the graphics processor has finished it. One frame is drawn
     * at a time, each in its own turn of a drawing thread, as the canvas does. */
    private fun liveMoveFrames(scenario: ActivityScenario<MainActivity>): Pair<List<Double>, List<Double>> {
        await(scenario, "ready-to-probe", predicate = ::canvasReady)
        var scene: CanvasScene? = null
        scenario.onActivity { scene = it.editor.scene }
        val base = requireNotNull(scene)
        val page = Point(base.document.width.toDouble(), base.document.height.toDouble())
        // The newest stroke in the band: the one moved by the computer last.
        val target = base.objects.last { it.item.bounds.y > page.y * BAND_TOP && it.item.bounds.y < page.y * BAND_BOTTOM }.item
        val width = base.camera.viewportWidth.toInt(); val height = base.camera.viewportHeight.toInt()
        val frames = ArrayList<Double>(); val recorded = ArrayList<Double>()
        val thread = HandlerThread("vw-cost-probe").apply { start() }
        val handler = Handler(thread.looper)
        val surface = AtomicHolder<PictureSurface>(); val picture = AtomicHolder<KeptPicture>()
        fun onThread(work: () -> Unit) {
            val done = CountDownLatch(1); var error: Throwable? = null
            handler.post { try { work() } catch (t: Throwable) { error = t } finally { done.countDown() } }
            assertTrue("The drawing thread did not answer", done.await(30, TimeUnit.SECONDS))
            error?.let { throw it }
        }
        fun frame(drawn: CanvasScene): Pair<Double, Double> {
            var took = 0L; var record = 0L
            onThread {
                val start = System.nanoTime()
                val made = requireNotNull(surface.value).render { canvas ->
                    val begun = System.nanoTime()
                    if (!requireNotNull(picture.value).draw(canvas, drawn, true, false, false)) PrecisionDrawing.draw(canvas, drawn)
                    record = System.nanoTime() - begun
                }
                took = System.nanoTime() - start
                assertNotNull("A probe frame was not drawn", made)
                made?.release()
            }
            return took / 1e6 to record / 1e6
        }
        try {
            onThread { surface.value = PictureSurface(width, height); picture.value = KeptPicture() }
            // The page at rest: its picture is made, then drawn from.
            frame(base); frame(base)
            for (step in 1..PROBE_FRAMES) {
                val moved = base.copy(revision = base.revision + step, transformed = mapOf(target.objectId to
                    EditorDrawing.compose(Transform(e = page.x * 0.002 * step, f = -page.y * 0.001 * step), target.transform)))
                val (took, record) = frame(moved)
                if (step > PROBE_WARM_UP) { frames += took; recorded += record }
            }
        } finally {
            onThread { picture.value?.discard(); surface.value?.close() }
            thread.quitSafely(); thread.join(5_000)
        }
        return frames to recorded
    }
    private class AtomicHolder<T> { @Volatile var value: T? = null }

    /** A new blank page with [filled] strokes put into it through the core,
     * then the run of strokes on it and the changes, measured. The project is deleted. */
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
                    val changes = measureChanges(scenario)
                    val (moveFrames, moveRecords) = liveMoveFrames(scenario)
                    result = Cost(filled, after - before, timings(report), shown, report, changes, moveFrames, moveRecords)
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
            if (lightMs == null || busyMs == null || lightMs.isNaN() || busyMs.isNaN()) {
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
        within("The time to draw one saved frame (canvas_saved_draw, median)", light.p50("canvas_saved_draw"), busy.p50("canvas_saved_draw"), 0.3)
        within("From a saved frame being drawn to its being ready (canvas_saved_ready, median)", light.p50("canvas_saved_ready"), busy.p50("canvas_saved_ready"), 2.0)
        for (change in Change.entries) {
            within("${change.name}: the saved frame that brings the picture up to date (longest canvas_saved_draw, median)",
                light.changes[change]?.frame, busy.changes[change]?.frame, 1.0)
        }
        within("One frame of a stroke being moved, drawn and finished (median)", median(light.moveFrameMs), median(busy.moveFrameMs), 1.0)
        within("One frame of a stroke being moved, recorded (median)", median(light.moveRecordMs), median(busy.moveRecordMs), 0.3)
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
        /** Each kind of change is made this many times on each page. */
        const val ROUNDS = 3
        const val PAUSE_MS = 250L
        const val CHANGE_TIMEOUT_MS = 10_000L
        /** Frames of a moving stroke drawn, and how many of the first are not counted. */
        const val PROBE_FRAMES = 40
        const val PROBE_WARM_UP = 5
        val KEYS = listOf("canvas_draw_time", "canvas_frame_ready", "canvas_ink_drawn", "canvas_first_ink", "canvas_save", "canvas_refresh",
            "canvas_saved_draw", "canvas_saved_ready")
        fun median(values: List<Double>): Double = values.filter { !it.isNaN() }.sorted().let { if (it.isEmpty()) Double.NaN else it[it.size / 2] }
    }
}
