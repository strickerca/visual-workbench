package com.visualworkbench.android

import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Rect
import android.graphics.RectF
import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.*
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.Shape
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import kotlin.math.abs

/** Actual app SurfaceView MotionEvents + native persistence; synthetic input only. */
@RunWith(AndroidJUnit4::class)
class EditorSurfaceInstrumentedTest {
    private fun ready(activity: MainActivity): Boolean = !activity.editor.busy && activity.editor.pending == 0 &&
        !activity.editor.instructionEditor.state.value.busy && !activity.editor.semanticEditor.state.value.busy

    private fun await(scenario: ActivityScenario<MainActivity>, phase: String, predicate: (MainActivity) -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + 45_000
        var nextProgress = SystemClock.uptimeMillis() + 5_000
        while (SystemClock.uptimeMillis() < deadline) {
            var ready = false
            scenario.onActivity { ready = predicate(it) }
            if (ready) return
            if (SystemClock.uptimeMillis() >= nextProgress) {
                Log.i("VW_EDITOR_TEST", "phase=$phase waiting=true"); nextProgress += 5_000
            }
            SystemClock.sleep(25)
        }
        var detail = ""
        scenario.onActivity { activity ->
            val editor = activity.editor
            val closeDetail = editor.retirementTimeoutSuffix
            detail = "busy=${editor.busy} pending=${editor.pending} message=${editor.message} blocked=${editor.blockingMessage} $closeDetail"
        }
        fail("Timed out in $phase: $detail")
    }
    private fun canvas(view: View): CanvasSurface? {
        if (view is CanvasSurface) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) canvas(view.getChildAt(i))?.let { return it }
        return null
    }

    /** Screen-space probes for the composited canvas. The renderer draws into
     * child SurfaceControls, so PixelCopy of the SurfaceView's own Surface or of
     * the Window cannot observe it; only a display composition screenshot can. */
    private class CanvasProbe(val canvas: Rect, val page: RectF, val pagePoint: Point, val surround: Int, val railAtEnd: Boolean)
    private class PresentSample(val presented: Boolean, val blackFraction: Double, val blackBandFraction: Double, val detail: String)

    private fun canvasProbe(activity: MainActivity): CanvasProbe? {
        val view = canvas(activity.window.decorView) ?: return null
        val scene = activity.editor.scene ?: return null
        if (scene.background == null || !view.holder.surface.isValid || view.width <= 0 || view.height <= 0) return null
        val location = IntArray(2); view.getLocationOnScreen(location)
        val w = scene.document.width.toDouble(); val h = scene.document.height.toDouble()
        val corners = listOf(Point(0.0, 0.0), Point(w, 0.0), Point(0.0, h), Point(w, h)).map { EditorDrawing.map(scene.matrix, it) }
        val page = RectF(corners.minOf { it.x }.toFloat(), corners.minOf { it.y }.toFloat(),
            corners.maxOf { it.x }.toFloat(), corners.maxOf { it.y }.toFloat())
        page.offset(location[0].toFloat(), location[1].toFloat())
        // Lower-right page area: every drawing fixture in this class stays above/left of it.
        val inside = EditorDrawing.map(scene.matrix, Point(w * .85, h * .85))
        return CanvasProbe(Rect(location[0], location[1], location[0] + view.width, location[1] + view.height), page,
            Point(inside.x + location[0], inside.y + location[1]), EditorDrawing.surroundArgb(scene.dark),
            activity.editor.preferences.leftHanded)
    }

    private fun near(actual: Int, expected: Int, tolerance: Int): Boolean =
        abs(Color.red(actual) - Color.red(expected)) <= tolerance && abs(Color.green(actual) - Color.green(expected)) <= tolerance &&
            abs(Color.blue(actual) - Color.blue(expected)) <= tolerance

    private fun presentSample(shot: Bitmap, probe: CanvasProbe): PresentSample {
        fun pixel(x: Double, y: Double): Int? {
            val ix = x.toInt(); val iy = y.toInt()
            return if (ix in 0 until shot.width && iy in 0 until shot.height) shot.getPixel(ix, iy) else null
        }
        val c = probe.canvas
        // The tool rail overlays one canvas edge; the central 40% band never contains it.
        // Every row of the band is sampled so a partial black strip is measured too.
        val band = Rect(c.left + (c.width() * .3).toInt(), c.top, c.left + (c.width() * .7).toInt(), c.bottom)
        var black = 0; var total = 0; var run = 0; var longestRun = 0
        if (band.intersect(0, 0, shot.width, shot.height) && band.width() > 0) {
            val rowPixels = IntArray(band.width())
            for (y in band.top until band.bottom) {
                shot.getPixels(rowPixels, 0, band.width(), band.left, y, band.width(), 1)
                var rowBlack = 0; var rowTotal = 0
                var x = 0
                while (x < rowPixels.size) {
                    val color = rowPixels[x]
                    rowTotal++
                    if (Color.red(color) <= 10 && Color.green(color) <= 10 && Color.blue(color) <= 10) rowBlack++
                    x += 8
                }
                black += rowBlack; total += rowTotal
                // A row is black when >=90% of its samples are near-black.
                if (rowBlack * 10 >= rowTotal * 9) { run++; if (run > longestRun) longestRun = run } else run = 0
            }
        }
        val fraction = if (total == 0) 1.0 else black.toDouble() / total
        val bandFraction = if (c.height() <= 0) 1.0 else longestRun.toDouble() / c.height()
        val pageColors = (-1..1).flatMap { dy -> (-1..1).map { dx -> pixel(probe.pagePoint.x + 4 * dx, probe.pagePoint.y + 4 * dy) } }
        val white = pageColors.all { it != null && near(it, Color.WHITE, 12) }
        val surroundPoints = buildList {
            val p = probe.page
            if (p.top - c.top >= 24) add(Point(c.exactCenterX().toDouble(), ((c.top + p.top) / 2).toDouble()))
            if (c.bottom - p.bottom >= 24) add(Point(c.exactCenterX().toDouble(), ((c.bottom + p.bottom) / 2).toDouble()))
            if (probe.railAtEnd && p.left - c.left >= 24) add(Point(((c.left + p.left) / 2).toDouble(), p.centerY().toDouble()))
            if (!probe.railAtEnd && c.right - p.right >= 24) add(Point(((c.right + p.right) / 2).toDouble(), p.centerY().toDouble()))
        }
        val surroundColors = surroundPoints.map { pixel(it.x, it.y) }
        val surround = surroundColors.isNotEmpty() && surroundColors.all { it != null && near(it, probe.surround, 16) }
        fun hex(value: Int?) = value?.let { String.format("%08x", it) } ?: "none"
        return PresentSample(white && surround && fraction < .01 && bandFraction < .02, fraction, bandFraction,
            "page=${hex(pageColors[4])} surround=${surroundColors.joinToString(",") { hex(it) }} black=${"%.3f".format(fraction)} " +
                "black_band=${"%.3f".format(bandFraction)}")
    }

    /** Polls composited display pixels: the page must be white, the surround its
     * theme colour and the canvas band <1% near-black with no contiguous black
     * strip of >=2% of the canvas rows. Any sample that is >=90% black, or that
     * contains such a strip, during the observation window fails, so a transient
     * full or partial black presentation gap is reported with its count, not only
     * a persistent one. Screenshots stay in memory and are recycled immediately. */
    private fun assertPresented(scenario: ActivityScenario<MainActivity>, phase: String, deadlineMs: Long = 2_000, observeMs: Long = 800) {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val start = SystemClock.uptimeMillis()
        var samples = 0; var blackSamples = 0; var bandSamples = 0; var firstBlackMs = -1L; var lastBlackMs = -1L
        var firstPresentedMs = -1L; var lastPresented = false; var detail = "canvas-unavailable"
        while (true) {
            var probe: CanvasProbe? = null
            scenario.onActivity { probe = canvasProbe(it) }
            val current = probe
            val elapsed = SystemClock.uptimeMillis() - start
            if (current == null) { lastPresented = false; detail = "canvas-unavailable" }
            else {
                val raw = automation.takeScreenshot()
                if (raw == null) { lastPresented = false; detail = "screenshot-unavailable" }
                else {
                    val shot = if (raw.config == Bitmap.Config.HARDWARE) raw.copy(Bitmap.Config.ARGB_8888, false).also { raw.recycle() } else raw
                    try {
                        val sample = presentSample(shot, current)
                        samples++
                        val fullBlack = sample.blackFraction >= .9
                        val blackBand = sample.blackBandFraction >= .02
                        if (fullBlack) blackSamples++
                        if (blackBand) bandSamples++
                        if (fullBlack || blackBand) { if (firstBlackMs < 0) firstBlackMs = elapsed; lastBlackMs = elapsed }
                        lastPresented = sample.presented; detail = sample.detail
                        if (sample.presented && firstPresentedMs < 0) firstPresentedMs = elapsed
                    } finally { shot.recycle() }
                }
            }
            val now = SystemClock.uptimeMillis() - start
            if ((lastPresented && now >= observeMs) || now >= deadlineMs) break
        }
        val summary = "phase=$phase samples=$samples black_samples=$blackSamples black_band_samples=$bandSamples " +
            "first_black_ms=$firstBlackMs last_black_ms=$lastBlackMs first_presented_ms=$firstPresentedMs presented=$lastPresented $detail"
        Log.i("VW_CANVAS_PRESENT", summary)
        if (!lastPresented || blackSamples > 0 || bandSamples > 0) fail("Canvas presentation failed: $summary")
    }

    private fun resizeCanvas(scenario: ActivityScenario<MainActivity>, phase: String, shrink: Boolean, action: (MainActivity) -> Unit) {
        var before = 0
        scenario.onActivity { before = requireNotNull(canvas(it.window.decorView)).height; action(it) }
        await(scenario, phase) { activity ->
            val view = canvas(activity.window.decorView)
            view != null && view.holder.surface.isValid && (if (shrink) view.height < before else view.height > before)
        }
    }
    private fun gesture(scenario: ActivityScenario<MainActivity>, tool: EditorTool, points: List<Point>, cancel: Boolean = false, finger: Boolean = false) {
        // The real input gate also waits for the instruction/semantic refreshes
        // that share the native project worker after an edit.
        await(scenario, "input-ready-$tool", ::ready)
        scenario.onActivity { activity ->
            val editor = activity.editor; editor.chooseTool(tool)
            val surface = requireNotNull(canvas(activity.window.decorView))
            val snapshot = requireNotNull(editor.scene)
            val screen = points.map { EditorDrawing.map(snapshot.matrix, it) }
            val start = SystemClock.uptimeMillis()
            screen.forEachIndexed { index, p ->
                val action = when (index) { 0 -> MotionEvent.ACTION_DOWN; screen.lastIndex -> if (cancel) MotionEvent.ACTION_CANCEL else MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 0; toolType = if (finger) MotionEvent.TOOL_TYPE_FINGER else MotionEvent.TOOL_TYPE_STYLUS })
                val coordinates = arrayOf(MotionEvent.PointerCoords().apply { x = p.x.toFloat(); y = p.y.toFloat(); pressure = .2f + .7f * index / maxOf(1, screen.lastIndex); size = .01f })
                val event = MotionEvent.obtain(start, start + index * 8, action, 1, properties, coordinates, 0, 0, 1f, 1f, -1, 0,
                    if (finger) InputDevice.SOURCE_TOUCHSCREEN else InputDevice.SOURCE_STYLUS, 0)
                try { assertTrue(surface.dispatchTouchEvent(event)) } finally { event.recycle() }
            }
        }
        await(scenario, "gesture-$tool", ::ready)
    }
    private fun withCanvas(body: (ActivityScenario<MainActivity>, String) -> Unit) {
        var ownedProject: String? = null
        var storageClosed = false
        var primaryFailure: Throwable? = null
        try {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            try {
                await(scenario, "startup") { !it.editor.busy }
                scenario.onActivity { it.editor.newCanvas() }
                await(scenario, "new-project-and-background") { ready(it) && it.editor.scene?.background != null && canvas(it.window.decorView)?.holder?.surface?.isValid == true }
                scenario.onActivity { ownedProject = requireNotNull(it.editor.info).projectId; assertNull(it.editor.blockingMessage) }
                body(scenario, requireNotNull(ownedProject))
            } catch (error: Throwable) {
                primaryFailure = error
                throw error
            } finally {
                try {
                    scenario.onActivity { it.editor.cancelTool() }
                    await(scenario, "finish-owned-edits") { it.editor.pending == 0 }
                    scenario.onActivity { it.editor.navigate(WorkbenchScreen.Projects) }
                    await(scenario, "close-owned-project") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
                    storageClosed = true
                } catch (cleanup: Throwable) {
                    val primary = primaryFailure
                    if (primary == null) throw cleanup
                    if (primary !== cleanup) primary.addSuppressed(cleanup)
                }
            }
        }
        } catch (error: Throwable) {
            if (primaryFailure == null) primaryFailure = error
            throw error
        } finally {
            try { ownedProject?.let { id ->
                val root = File(InstrumentationRegistry.getInstrumentation().targetContext.filesDir, "projects").canonicalFile
                val target = File(root, id).canonicalFile
                assertEquals(root, target.parentFile); assertTrue(id.matches(Regex("[0-9a-fA-F-]{36}")))
                if (storageClosed) {
                    assertTrue(!target.exists() || target.deleteRecursively())
                    Log.i("VW_EDITOR_TEST", "owned_project_cleanup=true")
                } else Log.e("VW_EDITOR_TEST", "owned_project_cleanup=false reason=storage_close_unconfirmed")
            } } catch (cleanup: Throwable) {
                val primary = primaryFailure
                if (primary == null) throw cleanup
                if (primary !== cleanup) primary.addSuppressed(cleanup)
            }
        }
    }

    @Test fun everyDrawingToolPersistsAndCancellationCreatesNoObject() = withCanvas { scenario, id ->
        val drawingTools = listOf(EditorTool.Pen, EditorTool.Highlighter, EditorTool.Marker, EditorTool.Line, EditorTool.Arrow, EditorTool.Rectangle, EditorTool.Ellipse)
        for ((i, tool) in drawingTools.withIndex()) {
            val y = 100.0 + i * 115
            gesture(scenario, tool, listOf(Point(260.0, y), Point(330.0, y + 15), Point(420.0, y + 45)))
            scenario.onActivity { assertEquals("tool=$tool ${it.editor.message}", i + 1, it.editor.document?.render?.items?.size) }
        }
        gesture(scenario, EditorTool.Pen, listOf(Point(600.0, 200.0), Point(720.0, 210.0), Point(800.0, 230.0)), cancel = true)
        scenario.onActivity { assertEquals(7, it.editor.document?.render?.items?.size) }
        // A finger uses navigation by default, even while Pen is selected.
        gesture(scenario, EditorTool.Pen, listOf(Point(650.0, 300.0), Point(700.0, 300.0), Point(750.0, 300.0)), finger = true)
        scenario.onActivity { assertEquals(7, it.editor.document?.render?.items?.size); it.editor.fit() }
        var hash = ""
        scenario.onActivity { hash = requireNotNull(it.editor.info).stateHash; it.editor.navigate(WorkbenchScreen.Projects) }
        await(scenario, "closed-for-reopen") { it.editor.screen == WorkbenchScreen.Projects && !it.editor.busy }
        scenario.onActivity { it.editor.open(id) }
        await(scenario, "reopen-document") { !it.editor.busy && it.editor.document != null }
        scenario.onActivity {
            assertEquals(hash, it.editor.info?.stateHash)
            assertEquals(7, it.editor.document?.render?.items?.size)
            assertTrue(it.editor.document?.render?.items?.any { item -> item.layerBlend == "multiply" } == true)
        }
    }

    @Test fun textSelectionMoveResizeEraserUndoAndActivityRecreationUseRealCore() = withCanvas { scenario, _ ->
        gesture(scenario, EditorTool.Rectangle, listOf(Point(420.0, 260.0), Point(600.0, 420.0)))
        val created = arrayOf("")
        scenario.onActivity { created[0] = requireNotNull(it.editor.document).render.items.single().objectId }
        gesture(scenario, EditorTool.Select, listOf(Point(420.0, 300.0), Point(460.0, 330.0)))
        scenario.onActivity {
            val item = requireNotNull(it.editor.document).render.items.single()
            assertEquals(created[0], item.objectId); assertEquals(40.0, item.transform.e, .02); assertEquals(30.0, item.transform.f, .02)
        }
        var corner = Point(0.0, 0.0)
        scenario.onActivity { val b = requireNotNull(it.editor.document).render.items.single().bounds; corner = Point(b.x + b.width, b.y + b.height) }
        gesture(scenario, EditorTool.Select, listOf(corner, Point(corner.x + 80, corner.y + 60)))
        scenario.onActivity { assertTrue(requireNotNull(it.editor.document).render.items.single().transform.a > 1) }
        gesture(scenario, EditorTool.Text, listOf(Point(500.0, 620.0), Point(500.0, 620.0)))
        scenario.onActivity { assertNotNull(it.editor.textDraft); it.editor.saveText("Editable text", "Inter", 36.0) }
        await(scenario, "save-text") { !it.editor.busy && it.editor.pending == 0 && it.editor.textDraft == null }
        scenario.onActivity { assertTrue(it.editor.document?.render?.items?.any { item -> item.shape is Shape.Text } == true) }
        gesture(scenario, EditorTool.Eraser, listOf(Point(455.0, 290.0), Point(475.0, 360.0)), cancel = true)
        scenario.onActivity { assertEquals(2, it.editor.document?.render?.items?.size) }
        // Explicit selected-object deletion is one transaction; undo restores it.
        scenario.onActivity { it.editor.deleteSelected() }
        await(scenario, "delete-selected") { it.editor.pending == 0 }
        scenario.onActivity { assertEquals(1, it.editor.document?.render?.items?.size); it.editor.undo() }
        await(scenario, "undo-delete") { it.editor.pending == 0 }
        var hash = ""
        scenario.onActivity { assertEquals(2, it.editor.document?.render?.items?.size); hash = requireNotNull(it.editor.info).stateHash }
        scenario.recreate()
        // Native state and a valid Surface do not prove presentation: require actual composited pixels.
        assertPresented(scenario, "renderer-after-recreation", deadlineMs = 3_000)
        scenario.onActivity { assertEquals(hash, it.editor.info?.stateHash); assertEquals(2, it.editor.document?.render?.items?.size) }
        gesture(scenario, EditorTool.Pen, listOf(Point(900.0, 600.0), Point(970.0, 650.0)))
        scenario.onActivity { assertEquals(3, it.editor.document?.render?.items?.size) }
    }

    @Test fun eraserResizeAndRecreationPresentCanvasPixels() = withCanvas { scenario, _ ->
        gesture(scenario, EditorTool.Pen, listOf(Point(260.0, 100.0), Point(330.0, 115.0), Point(420.0, 145.0)))
        assertPresented(scenario, "baseline")
        // Eraser controls take layout space below the weighted canvas: an actual SurfaceView resize.
        resizeCanvas(scenario, "eraser-controls-resize", shrink = true) { it.editor.chooseTool(EditorTool.Eraser) }
        assertPresented(scenario, "after-eraser-resize")
        scenario.recreate()
        assertPresented(scenario, "after-recreation", deadlineMs = 3_000)
        resizeCanvas(scenario, "pen-controls-resize", shrink = false) { it.editor.chooseTool(EditorTool.Pen) }
        assertPresented(scenario, "after-pen-resize")
    }
}
